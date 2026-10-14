use anyhow::{Context, Result, ensure};
use maa_framework::buffer::MaaImageBuffer;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{ffi::c_void, sync::OnceLock, time::Instant};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

use crate::{NativeSession, RecognitionImage, RecognitionMeasurement};

pub const FLASH_MODEL: &str = "yolov8n-640-flash-20250921-logits.onnx";
pub const FLASH_LABELS: [&str; 3] = ["none", "red", "yellow"];
pub const FLASH_SIZE: i32 = 640;

fn default_roi() -> [i32; 4] {
    [0, 0, 1920, 1080]
}
fn default_threshold() -> f64 {
    0.9
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlashConfig {
    pub model: String,
    #[serde(default = "default_roi")]
    pub roi: [i32; 4],
    #[serde(default = "default_threshold")]
    pub threshold: f64,
    pub labels: Option<Vec<String>>,
}

impl FlashConfig {
    pub fn parse(params: &Value) -> Result<Self> {
        let config: Self = serde_json::from_value(params.clone())?;
        ensure!(
            config.model == FLASH_MODEL,
            "闪光适配须使用固定 logits 模型"
        );
        ensure!(
            config
                .labels
                .as_ref()
                .is_none_or(|labels| labels.iter().map(String::as_str).eq(FLASH_LABELS)),
            "闪光标签顺序须为 none,red,yellow"
        );
        ensure!(
            config.threshold.is_finite() && (0.0..=1.0).contains(&config.threshold),
            "闪光阈值须为 0..1"
        );
        let [x, y, w, h] = config.roi;
        ensure!(
            x >= 0
                && y >= 0
                && w > 0
                && h > 0
                && x.checked_add(w).is_some_and(|r| r <= 1920)
                && y.checked_add(h).is_some_and(|b| b <= 1080),
            "闪光 ROI 超出 1920×1080"
        );
        Ok(config)
    }
}

#[repr(C)]
struct CvSize {
    width: i32,
    height: i32,
}
type CreateHeader = unsafe extern "C" fn(CvSize, i32, i32) -> *mut c_void;
type SetData = unsafe extern "C" fn(*mut c_void, *mut c_void, i32);
type Resize = unsafe extern "C" fn(*const c_void, *mut c_void, i32);
type ReleaseHeader = unsafe extern "C" fn(*mut *mut c_void);

#[derive(Clone, Copy)]
struct CvApi {
    create: CreateHeader,
    set: SetData,
    resize: Resize,
    release: ReleaseHeader,
}
static CV_API: OnceLock<CvApi> = OnceLock::new();

fn cv_api() -> Result<CvApi> {
    if let Some(api) = CV_API.get() {
        return Ok(*api);
    }
    crate::sdk::verify_loaded_modules()?;
    let name: Vec<u16> = "opencv_world4_maa.dll"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    ensure!(!module.is_null(), "SDK OpenCV 尚未加载");
    // 固定 SDK 已校验并终身加载；只取其现有 C 导出，不再加载其他 OpenCV。
    let api = unsafe {
        CvApi {
            create: std::mem::transmute::<unsafe extern "system" fn() -> isize, CreateHeader>(
                GetProcAddress(module, c"cvCreateImageHeader".as_ptr().cast())
                    .context("SDK 缺少 cvCreateImageHeader")?,
            ),
            set: std::mem::transmute::<unsafe extern "system" fn() -> isize, SetData>(
                GetProcAddress(module, c"cvSetData".as_ptr().cast())
                    .context("SDK 缺少 cvSetData")?,
            ),
            resize: std::mem::transmute::<unsafe extern "system" fn() -> isize, Resize>(
                GetProcAddress(module, c"cvResize".as_ptr().cast()).context("SDK 缺少 cvResize")?,
            ),
            release: std::mem::transmute::<unsafe extern "system" fn() -> isize, ReleaseHeader>(
                GetProcAddress(module, c"cvReleaseImageHeader".as_ptr().cast())
                    .context("SDK 缺少 cvReleaseImageHeader")?,
            ),
        }
    };
    let _ = CV_API.set(api);
    Ok(api)
}

struct Header {
    raw: *mut c_void,
    release: ReleaseHeader,
}
impl Header {
    fn new(api: CvApi, width: i32, height: i32) -> Result<Self> {
        let raw = unsafe { (api.create)(CvSize { width, height }, 8, 3) };
        ensure!(!raw.is_null(), "OpenCV 图像头创建失败");
        Ok(Self {
            raw,
            release: api.release,
        })
    }
}
impl Drop for Header {
    fn drop(&mut self) {
        unsafe {
            (self.release)(&mut self.raw);
        }
    }
}

pub struct FlashImage {
    pub bgr: Vec<u8>,
    pub scaled_size: [i32; 2],
}

pub fn prepare_flash_bgr(bgr: &[u8], width: i32, height: i32) -> Result<FlashImage> {
    ensure!(
        (1..=1920).contains(&width)
            && (1..=1080).contains(&height)
            && bgr.len() == (width * height * 3) as usize,
        "闪光输入须为连续的 8 位 BGR 图像"
    );
    let scale = (FLASH_SIZE as f64 / width as f64).min(FLASH_SIZE as f64 / height as f64);
    // Python round 使用偶数舍入，不能使用 Rust round 的远离零舍入。
    let scaled_width = (width as f64 * scale).round_ties_even() as i32;
    let scaled_height = (height as f64 * scale).round_ties_even() as i32;
    ensure!(
        scaled_width > 0 && scaled_height > 0,
        "闪光 ROI 缩放后尺寸为空"
    );
    let api = cv_api()?;
    let source = Header::new(api, width, height)?;
    let destination = Header::new(api, scaled_width, scaled_height)?;
    let mut pixels = vec![114; (FLASH_SIZE * FLASH_SIZE * 3) as usize];
    // cvResize 只读来源像素；图像头分别使用真实行跨度，释放头不会释放 Rust 数据。
    unsafe {
        (api.set)(source.raw, bgr.as_ptr() as *mut c_void, width * 3);
        (api.set)(destination.raw, pixels.as_mut_ptr().cast(), FLASH_SIZE * 3);
        (api.resize)(source.raw, destination.raw, 1); // CV_INTER_LINEAR
    }
    Ok(FlashImage {
        bgr: pixels,
        scaled_size: [scaled_width, scaled_height],
    })
}

pub fn flash_decision(native: &Value, config: &FlashConfig) -> Result<Value> {
    let results = native["detail"]["all"]
        .as_array()
        .context("Maa 闪光详情缺少 all")?;
    ensure!(
        native["hit"] == true && results.len() == 1,
        "Maa 闪光调用未返回唯一分类结果"
    );
    let candidate = &results[0];
    let index = candidate["cls_index"]
        .as_u64()
        .context("闪光类别格式无效")? as usize;
    let score = candidate["score"].as_f64().context("闪光分数缺失")?;
    ensure!(
        index < FLASH_LABELS.len()
            && candidate["label"] == FLASH_LABELS[index]
            && score.is_finite()
            && (0.0..=1.0).contains(&score),
        "闪光类别或分数无效"
    );
    ensure!(
        candidate["box"] == json!([0, 0, FLASH_SIZE, FLASH_SIZE]),
        "闪光分类框不是完整模型输入"
    );
    // 固定 SDK 的 JSON 分数只有六位小数。实际低于阈值的概率可能被舍入到
    // 阈值上，不能把无法恢复的边界当作命中或普通未命中。
    ensure!(
        (score - config.threshold).abs() > 0.000001,
        "闪光分数 {score} 距阈值 {} 不超过 0.000001；Maa JSON 六位小数不足以恢复旧判定，需要原始浮点输出",
        config.threshold
    );
    let accepted = score >= config.threshold;
    let candidate =
        json!({"cls_index":index,"label":FLASH_LABELS[index],"score":score,"box":config.roi});
    Ok(json!({
        "hit":accepted,
        "detail":{"all":[candidate.clone()],"filtered":if accepted {vec![candidate.clone()]} else {vec![]},"best":if accepted {candidate.clone()} else {Value::Null}},
        "adaptation":{"kind":"LegacyFlashClassify","model":FLASH_MODEL,"threshold":config.threshold,"class_index":if accepted {index as i32} else {-1},"candidate":candidate,"score_source":"Maa softmax of original model logits","preprocessing":"SDK OpenCV INTER_LINEAR, top-left alignment, right/bottom padding 114"},
        "native_detail":native
    }))
}

impl NativeSession {
    pub(crate) fn recognize_flash(
        &self,
        image: &RecognitionImage,
        params: &Value,
    ) -> Result<RecognitionMeasurement> {
        let started = Instant::now();
        let config = FlashConfig::parse(params)?;
        ensure!(
            image.buffer.width() == 1920
                && image.buffer.height() == 1080
                && image.buffer.image_type() == 16,
            "闪光适配要求原始 1920×1080 BGR 图像"
        );
        let [x, y, w, h] = config.roi;
        let source = image.buffer.raw_data().context("闪光图像像素为空")?;
        let mut crop = Vec::with_capacity((w * h * 3) as usize);
        for row in y..y + h {
            let offset = ((row * 1920 + x) * 3) as usize;
            crop.extend_from_slice(&source[offset..offset + (w * 3) as usize]);
        }
        let prepared = prepare_flash_bgr(&crop, w, h)?;
        let mut buffer = MaaImageBuffer::new()?;
        buffer.set_raw_data(&prepared.bgr, FLASH_SIZE, FLASH_SIZE, 16)?;
        let prepare_us = started.elapsed().as_micros() as u64;
        let mut measured = self.recognize_prepared(
            &RecognitionImage {
                buffer,
                prepare_us: 0,
            },
            "NeuralNetworkClassify",
            &json!({"model":config.model,"roi":[0,0,FLASH_SIZE,FLASH_SIZE],"labels":FLASH_LABELS}),
        )?;
        measured.detail = flash_decision(&measured.detail, &config)?;
        measured.detail["adaptation"]["scaled_size"] = json!(prepared.scaled_size);
        measured.timing.prepare_us = prepare_us;
        measured.timing.total_us = started.elapsed().as_micros() as u64;
        Ok(measured)
    }
}
