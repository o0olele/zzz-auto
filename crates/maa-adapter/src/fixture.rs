use anyhow::{Context, Result, ensure};
use maa_framework::{
    MaaStatus,
    buffer::MaaImageBuffer,
    controller::Controller,
    custom_controller::{CustomControllerCallback, get_callbacks},
    sys,
};
use std::{path::Path, ptr::NonNull};

pub(crate) fn read_png(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    ensure!(bytes.len() <= 10 * 1024 * 1024, "固定截图超过大小限制");
    // 固定 SDK 对参考仓 WebP 返回空图；仅在离线入口补充 Rust 图片编解码。
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1920);
    limits.max_image_height = Some(1080);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .context("固定截图解码失败或超出 1080p 大小限制")?;
    ensure!(
        image.width() == 1920 && image.height() == 1080,
        "固定截图须为 1920×1080，当前为 {}×{}",
        image.width(),
        image.height()
    );
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    Ok(png.into_inner())
}

struct ImageController {
    png: Vec<u8>,
}

impl CustomControllerCallback for ImageController {
    fn connect(&self) -> bool {
        true
    }
    fn request_uuid(&self) -> Option<String> {
        Some("zzz-auto-fixed-image".into())
    }
    fn screencap(&self) -> Option<Vec<u8>> {
        Some(self.png.clone())
    }
}

// 1.25.2 的 new_custom 未保存 callback 所有权。使用同一官方回调表，
// 显式保留 Box 并在 MaaControllerDestroy 之后释放，避免重复验证泄漏。
pub(crate) struct FixtureHandle {
    handle: NonNull<sys::MaaController>,
    _callback: Box<Box<dyn CustomControllerCallback>>,
}

impl Drop for FixtureHandle {
    fn drop(&mut self) {
        unsafe {
            sys::MaaControllerDestroy(self.handle.as_ptr());
        }
    }
}

pub(crate) enum NativeController {
    Game(Controller),
    Fixture(FixtureHandle),
}

impl NativeController {
    pub fn from_image(path: &Path) -> Result<Self> {
        let image_path = if path.is_file() {
            path.to_path_buf()
        } else {
            let mut images: Vec<_> = std::fs::read_dir(path)?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension().is_some_and(|ext| {
                        ["png", "jpg", "jpeg", "webp"].contains(&ext.to_string_lossy().as_ref())
                    })
                })
                .collect();
            images.sort();
            images
                .into_iter()
                .next()
                .context("离线控制器目录没有图像")?
        };
        let bytes = read_png(&image_path)?;
        let mut image = MaaImageBuffer::new()?;
        image.set_encoded(&bytes)?;
        ensure!(
            image.width() == 1920 && image.height() == 1080,
            "固定截图须为 1920×1080，当前为 {}×{}",
            image.width(),
            image.height()
        );
        let png = image.to_vec().context("固定截图转为 PNG 失败")?;
        Self::from_callback(ImageController { png })
    }

    pub fn from_callback(callback: impl CustomControllerCallback + 'static) -> Result<Self> {
        let mut callback: Box<Box<dyn CustomControllerCallback>> = Box::new(Box::new(callback));
        let handle = unsafe {
            sys::MaaCustomControllerCreate(
                get_callbacks() as *const _ as *mut _,
                (&mut *callback as *mut Box<dyn CustomControllerCallback>).cast(),
            )
        };
        let handle = NonNull::new(handle).context("Maa 自定义控制器创建失败")?;
        Ok(Self::Fixture(FixtureHandle {
            handle,
            _callback: callback,
        }))
    }

    pub fn raw(&self) -> *mut sys::MaaController {
        match self {
            Self::Game(controller) => controller.raw(),
            Self::Fixture(controller) => controller.handle.as_ptr(),
        }
    }
    pub fn wait(&self, id: i64) -> MaaStatus {
        MaaStatus(unsafe { sys::MaaControllerWait(self.raw(), id) })
    }
    pub fn status(&self, id: i64) -> MaaStatus {
        MaaStatus(unsafe { sys::MaaControllerStatus(self.raw(), id) })
    }
    pub fn post_connection(&self) -> Result<i64> {
        Ok(unsafe { sys::MaaControllerPostConnection(self.raw()) })
    }
    pub fn post_screencap(&self) -> Result<i64> {
        Ok(unsafe { sys::MaaControllerPostScreencap(self.raw()) })
    }
    pub fn post_inactive(&self) -> Result<i64> {
        Ok(unsafe { sys::MaaControllerPostInactive(self.raw()) })
    }
    pub fn post_key_down(&self, key: i32) -> Result<i64> {
        Ok(unsafe { sys::MaaControllerPostKeyDown(self.raw(), key) })
    }
    pub fn post_key_up(&self, key: i32) -> Result<i64> {
        Ok(unsafe { sys::MaaControllerPostKeyUp(self.raw(), key) })
    }
    pub fn post_touch_down(&self, contact: i32, x: i32, y: i32, pressure: i32) -> Result<i64> {
        Ok(unsafe { sys::MaaControllerPostTouchDown(self.raw(), contact, x, y, pressure) })
    }
    pub fn post_touch_up(&self, contact: i32) -> Result<i64> {
        Ok(unsafe { sys::MaaControllerPostTouchUp(self.raw(), contact) })
    }

    pub fn post_touch_move(&self, contact: i32, x: i32, y: i32, pressure: i32) -> Result<i64> {
        Ok(unsafe { sys::MaaControllerPostTouchMove(self.raw(), contact, x, y, pressure) })
    }
    pub fn post_relative_move(&self, dx: i32, dy: i32) -> Result<i64> {
        Ok(unsafe { sys::MaaControllerPostRelativeMove(self.raw(), dx, dy) })
    }

    pub fn cached_image(&self) -> Result<MaaImageBuffer> {
        let image = MaaImageBuffer::new()?;
        ensure!(
            unsafe { sys::MaaControllerCachedImage(self.raw(), image.raw()) } != 0,
            "Maa 截图缓存为空"
        );
        Ok(image)
    }

    pub fn set_screenshot_use_raw_size(&self, enabled: bool) -> Result<()> {
        let mut value = enabled as u8;
        ensure!(
            unsafe {
                sys::MaaControllerSetOption(
                    self.raw(),
                    sys::MaaCtrlOptionEnum_MaaCtrlOption_ScreenshotUseRawSize as i32,
                    (&mut value as *mut u8).cast(),
                    1,
                )
            } != 0,
            "无法固定 Maa 原始截图尺寸"
        );
        Ok(())
    }
}
