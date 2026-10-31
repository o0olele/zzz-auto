//! 战斗模板直接调用固定 OpenCV；模板只解码一次，匹配借用截图 ROI，不经过 Maa 任务队列。
use crate::{RecognitionImage, combat_vision::CombatFrame};
use anyhow::{Context, Result, bail, ensure};
use std::{cell::RefCell, ffi::CStr, path::Path, sync::OnceLock};

unsafe extern "C" {
    fn zzz_combat_opencv_init(
        construct: *const std::ffi::c_void,
        destroy: *const std::ffi::c_void,
        matcher: *const std::ffi::c_void,
    );
    fn zzz_combat_match(
        image: *const u8,
        width: i32,
        height: i32,
        stride: usize,
        template: *const u8,
        tw: i32,
        th: i32,
        mask: *const u8,
        scratch: *mut f32,
        score: *mut f64,
        error: *mut std::ffi::c_char,
        error_capacity: usize,
    ) -> i32;
}
static CHECKED: OnceLock<std::result::Result<(), String>> = OnceLock::new();

fn initialize() -> Result<()> {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    crate::sdk::verify_loaded_modules()?;
    let name: Vec<u16> = "opencv_world4_maa.dll"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    ensure!(!module.is_null(), "SDK OpenCV 尚未加载");
    let address = |name: &CStr| -> Result<*const std::ffi::c_void> {
        Ok(unsafe { GetProcAddress(module, name.as_ptr().cast()) }
            .with_context(|| format!("SDK OpenCV 缺少导出 {}", name.to_string_lossy()))?
            as *const () as *const std::ffi::c_void)
    };
    let mut version = 0;
    for (symbol, multiplier) in [
        (c"?getVersionMajor@cv@@YAHXZ", 10000),
        (c"?getVersionMinor@cv@@YAHXZ", 100),
        (c"?getVersionRevision@cv@@YAHXZ", 1),
    ] {
        let function: unsafe extern "C" fn() -> i32 =
            unsafe { std::mem::transmute(address(symbol)?) };
        version += unsafe { function() } * multiplier;
    }
    ensure!(
        version == 41200,
        "战斗直接识别需要 OpenCV 4.12.0，实际版本编号 {version}"
    );
    let construct = address(c"??0Mat@cv@@QEAA@HHHPEAX_K@Z")?;
    let destroy = address(c"??1Mat@cv@@QEAA@XZ")?;
    let matcher = address(c"?matchTemplate@cv@@YAXAEBV_InputArray@1@0AEBV_OutputArray@1@H0@Z")?;
    // 三个入口均来自已检查路径、固定哈希及版本的同一 MSVC x64 DLL。
    unsafe { zzz_combat_opencv_init(construct, destroy, matcher) };
    Ok(())
}
thread_local! {
    static SCORES: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

pub struct CombatTemplate {
    width: i32,
    height: i32,
    pixels: Vec<u8>,
    mask: Option<Vec<u8>>,
}
impl CombatTemplate {
    pub(crate) fn load(path: &Path, green_mask: bool) -> Result<Self> {
        let image = image::open(path)?.to_rgb8();
        let (width, height) = image.dimensions();
        ensure!(
            (1..=1920).contains(&width) && (1..=1080).contains(&height),
            "战斗模板尺寸无效"
        );
        let mut pixels = image.into_raw();
        for pixel in pixels.chunks_exact_mut(3) {
            pixel.swap(0, 2);
        }
        // Maa 绿色掩码恰好排除 BGR(0,255,0)，全有效时使用无掩码快速路径。
        let mask = green_mask
            .then(|| {
                pixels
                    .chunks_exact(3)
                    .map(|pixel| if pixel == [0, 255, 0] { 0 } else { 255 })
                    .collect::<Vec<_>>()
            })
            .filter(|mask| mask.contains(&0));
        Ok(Self {
            width: width as i32,
            height: height as i32,
            pixels,
            mask,
        })
    }

    pub fn score(&self, frame: &CombatFrame, rect: [i32; 4]) -> Result<Option<f64>> {
        self.score_image(frame.recognition_image(), rect)
    }

    fn score_image(&self, image: &RecognitionImage, rect: [i32; 4]) -> Result<Option<f64>> {
        od_core::Roi::from_legacy(rect).map_err(anyhow::Error::msg)?;
        ensure!(
            image.buffer.image_type() == 16 && image.buffer.channels() == 3,
            "战斗模板输入须为 BGR8"
        );
        let (iw, ih) = (image.buffer.width(), image.buffer.height());
        ensure!(rect[2] <= iw && rect[3] <= ih, "战斗模板 ROI 超出截图");
        let [left, top, right, bottom] = rect;
        let (width, height) = (right - left, bottom - top);
        if width < self.width || height < self.height {
            return Ok(None);
        }
        let checked = CHECKED.get_or_init(|| initialize().map_err(|error| format!("{error:#}")));
        if let Err(error) = checked {
            bail!("{error}");
        }
        let pixels = image.buffer.raw_data().context("战斗帧像素缺失")?;
        let stride = iw as usize * 3;
        let start = top as usize * stride + left as usize * 3;
        let end = start + (height as usize - 1) * stride + width as usize * 3;
        ensure!(end <= pixels.len(), "战斗帧像素长度不足");
        let count = (width - self.width + 1) as usize * (height - self.height + 1) as usize;
        SCORES.with(|scratch| {
            let mut scratch = scratch.borrow_mut();
            if scratch.len() < count {
                scratch.resize(count, 0.0);
            }
            let mut error = [0_i8; 512];
            let mut score = 0.0;
            // 帧/模板/掩码在同步调用期间保持借用；结果矩阵使用足量、独占的线程局部缓冲。
            // 桥接捕获 C++ 异常，既不保留指针，也不跨 ABI 传递对象所有权。
            let status = unsafe {
                zzz_combat_match(
                    pixels.as_ptr().add(start),
                    width,
                    height,
                    stride,
                    self.pixels.as_ptr(),
                    self.width,
                    self.height,
                    self.mask
                        .as_ref()
                        .map_or(std::ptr::null(), |mask| mask.as_ptr()),
                    scratch.as_mut_ptr(),
                    &mut score,
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            match status {
                1 => Ok(Some(score)),
                0 => Ok(None),
                _ => bail!(
                    "战斗 OpenCV 匹配失败：{}",
                    unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy()
                ),
            }
        })
    }
}
