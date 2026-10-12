mod fixture;
mod sdk;
pub use od_core::CaptureMethod;
pub use sdk::{BINDING_VERSION, SDK_VERSION, load_sdk};

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use chrono::Utc;
use maa_framework::{
    MaaStatus,
    buffer::MaaImageBuffer,
    common::{GamepadType, Win32InputMethod, Win32ScreencapMethod},
    controller::Controller,
    job::Job,
    resource::Resource,
    tasker::Tasker,
};
use od_core::{BASE_HEIGHT, BASE_WIDTH, Screenshot, ScreenshotQuality, WindowInfo};
use od_platform::GameLease;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn native_capture(method: CaptureMethod) -> Win32ScreencapMethod {
    match method {
        CaptureMethod::Gdi => Win32ScreencapMethod::GDI,
        CaptureMethod::PrintWindow => Win32ScreencapMethod::PRINT_WINDOW,
        CaptureMethod::FramePool => Win32ScreencapMethod::FRAME_POOL,
        CaptureMethod::Dxgi => Win32ScreencapMethod::DXGI_DESKTOP_DUP_WINDOW,
        CaptureMethod::ScreenDc => Win32ScreencapMethod::SCREEN_DC,
    }
}

fn screenshot_quality(image: &MaaImageBuffer) -> Result<ScreenshotQuality> {
    let channels = image.channels() as usize;
    ensure!(
        (channels == 3 && image.image_type() == 16) || (channels == 4 && image.image_type() == 24),
        "Maa 截图不是 8 位 BGR / BGRA 图像"
    );
    let pixels = image.raw_data().context("Maa 未返回截图像素")?;
    let mut luma_sum = 0u64;
    let mut dark_pixels = 0u64;
    for pixel in pixels.chunks_exact(channels) {
        let [b, g, r] = [pixel[0], pixel[1], pixel[2]];
        luma_sum += 29 * b as u64 + 150 * g as u64 + 77 * r as u64;
        dark_pixels += u64::from(b <= 8 && g <= 8 && r <= 8);
    }
    let count = (image.width() * image.height()) as f64;
    let mean_luma = luma_sum as f64 / (256.0 * count);
    let dark_pixel_ratio = dark_pixels as f64 / count;
    Ok(ScreenshotQuality {
        mean_luma,
        dark_pixel_ratio,
        // 加载过场也可能黑屏，只提示疑似黑帧，不丢弃真实截图。
        near_black: dark_pixel_ratio >= 0.995 && mean_luma <= 2.0,
    })
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    #[default]
    Foreground,
    Background,
    Xbox,
    Ds4,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum InferenceDevice {
    #[default]
    Cpu,
    DirectMl(i32),
}

#[derive(Debug, Serialize)]
pub struct ProbeResult {
    pub sdk_version: String,
    pub stopped: bool,
    pub elapsed_ms: u64,
    pub enqueue_us: u64,
    pub stop_ms: Option<u64>,
    pub native_status: String,
}

// 字段按 Tasker、Controller、Resource、控制权依次销毁。
pub struct NativeSession {
    tasker: Tasker,
    controller: fixture::NativeController,
    _resource: Resource,
    input_mode: Option<InputMode>,
    capture_method: Option<CaptureMethod>,
    held_keys: BTreeSet<i32>,
    _lease: Option<GameLease>,
}

impl NativeSession {
    pub fn for_game(
        window: &WindowInfo,
        resources: &Path,
        capture: CaptureMethod,
        input: InputMode,
        device: InferenceDevice,
    ) -> Result<Self> {
        let lease = GameLease::acquire(window.pid)?;
        let hwnd = usize::from_str_radix(&window.hwnd, 16)? as *mut std::ffi::c_void;
        let controller = match input {
            InputMode::Xbox => {
                Controller::new_gamepad(hwnd, GamepadType::Xbox360, native_capture(capture))?
            }
            InputMode::Ds4 => {
                Controller::new_gamepad(hwnd, GamepadType::DualShock4, native_capture(capture))?
            }
            mode => {
                let method = match mode {
                    InputMode::Background => Win32InputMethod::SEND_MESSAGE,
                    _ => Win32InputMethod::SEIZE,
                };
                Controller::new_win32(
                    hwnd,
                    native_capture(capture).bits(),
                    method.bits(),
                    method.bits(),
                )?
            }
        };
        Self::bind(
            fixture::NativeController::Game(controller),
            resources,
            device,
            Some(lease),
            Some(input),
            Some(capture),
        )
    }

    pub fn for_fixture(images: &Path, resources: &Path, device: InferenceDevice) -> Result<Self> {
        let controller = fixture::NativeController::from_image(images)?;
        Self::bind(controller, resources, device, None, None, None)
    }

    fn bind(
        controller: fixture::NativeController,
        resources: &Path,
        device: InferenceDevice,
        lease: Option<GameLease>,
        input_mode: Option<InputMode>,
        capture_method: Option<CaptureMethod>,
    ) -> Result<Self> {
        controller.set_screenshot_use_raw_size(true)?;
        ensure!(
            controller.wait(controller.post_connection()?).is_success(),
            "Maa 控制器连接失败，检查游戏窗口、权限或驱动"
        );
        let resource = Resource::new()?;
        match device {
            InferenceDevice::Cpu => resource.use_cpu()?,
            InferenceDevice::DirectMl(id) => resource.use_directml(id)?,
        }
        let path = resources.canonicalize().context("资源目录不存在")?;
        ensure!(
            resource
                .post_bundle(path.to_str().context("资源路径不是有效 UTF-8")?)?
                .wait()
                .is_success(),
            "Maa 资源包加载失败"
        );
        let tasker = Tasker::new()?;
        tasker.bind_resource(&resource)?;
        ensure!(
            unsafe { maa_framework::sys::MaaTaskerBindController(tasker.raw(), controller.raw()) }
                != 0,
            "绑定 Maa 控制器失败"
        );
        ensure!(tasker.inited(), "Maa Tasker 初始化失败");
        sdk::verify_loaded_modules()?;
        Ok(Self {
            tasker,
            controller,
            _resource: resource,
            input_mode,
            capture_method,
            held_keys: BTreeSet::new(),
            _lease: lease,
        })
    }

    pub fn add_event_sink(&self, emit: impl Fn(String) + Send + Sync + 'static) -> Result<()> {
        self.tasker.add_sink(move |message, _details| {
            // 回调只投递简短通知，不阻塞、不等待本 Tasker，也不输出识别文本或截图。
            emit(message.to_owned());
        })?;
        Ok(())
    }

    pub fn screenshot(&self) -> Result<Screenshot> {
        let started = Instant::now();
        ensure!(
            self.controller
                .wait(self.controller.post_screencap()?)
                .is_success(),
            "Maa {} 截图失败，请恢复游戏窗口并尝试其他截图方式",
            self.capture_method
                .map(CaptureMethod::name)
                .unwrap_or("离线")
        );
        let image = self.controller.cached_image()?;
        ensure!(
            image.width() == BASE_WIDTH && image.height() == BASE_HEIGHT,
            "截图为 {}×{}，原型要求 1920×1080 客户区",
            image.width(),
            image.height()
        );
        let quality = screenshot_quality(&image)?;
        let png = image.to_vec().context("Maa 未返回 PNG 截图")?;
        ensure!(png.len() < 10 * 1024 * 1024, "截图编码超过预览大小限制");
        Ok(Screenshot {
            png_base64: base64::engine::general_purpose::STANDARD.encode(png),
            width: image.width(),
            height: image.height(),
            color_order: "Maa BGR → PNG RGB".into(),
            capture_method: self.capture_method,
            quality,
            captured_at: Utc::now(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }

    pub fn health_check(
        &mut self,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<ProbeResult> {
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) {
            self.release_inputs()?;
            return Ok(ProbeResult {
                sdk_version: SDK_VERSION.into(),
                stopped: true,
                elapsed_ms: started.elapsed().as_millis() as u64,
                enqueue_us: 0,
                stop_ms: Some(0),
                native_status: "NotStarted".into(),
            });
        }
        let job = self.tasker.post_task("Prototype.HealthCheck", "{}")?;
        let enqueue_us = started.elapsed().as_micros() as u64;
        let mut stop_started = None;
        let mut timed_out = false;
        loop {
            let status = job.status();
            if status.done() {
                break;
            }
            ensure!(status != MaaStatus::INVALID, "Maa 返回无效任务 ID");
            if stop_started.is_none()
                && (cancel.load(Ordering::Acquire) || started.elapsed() >= timeout)
            {
                timed_out = started.elapsed() >= timeout;
                stop_started = Some(Instant::now());
                let stop_id = self.tasker.post_stop()?;
                // 收到停止响应后仍等待原生任务完成；不能提前交还控制权。
                Job::for_tasker(&self.tasker, stop_id).wait();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let status = job.wait();
        drop(job);
        self.release_inputs()?;
        if timed_out {
            bail!("健康检查超时，原生队列已停止并完成输入释放");
        }
        ensure!(
            status.is_success() || stop_started.is_some(),
            "Maa 健康检查失败：{status}"
        );
        Ok(ProbeResult {
            sdk_version: SDK_VERSION.into(),
            stopped: stop_started.is_some(),
            enqueue_us,
            elapsed_ms: started.elapsed().as_millis() as u64,
            stop_ms: stop_started.map(|at| at.elapsed().as_millis() as u64),
            native_status: status.to_string(),
        })
    }

    pub fn recognize(
        &self,
        image: &Path,
        kind: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        ensure!(
            [
                "OCR",
                "TemplateMatch",
                "FeatureMatch",
                "ColorMatch",
                "NeuralNetworkClassify",
                "NeuralNetworkDetect"
            ]
            .contains(&kind),
            "不支持的识别类型"
        );
        let bytes = fixture::read_png(image)?;
        let mut buffer = MaaImageBuffer::new()?;
        buffer.set_encoded(&bytes)?;
        ensure!(
            buffer.width() == BASE_WIDTH && buffer.height() == BASE_HEIGHT,
            "识别语料必须是 1920×1080"
        );
        let job = self
            .tasker
            .post_recognition(kind, &params.to_string(), &buffer)?;
        ensure!(job.wait().is_success(), "Maa 识别调用失败");
        // SDK 返回 MaaTaskId；1.25.2 的 job.get 把它当成 MaaRecoId。
        // 按任务 → 节点 → 识别详情读取，保持使用固定官方绑定。
        let task = self
            .tasker
            .get_task_detail(job.id)?
            .context("Maa 未返回识别任务详情")?;
        let detail = task
            .nodes
            .into_iter()
            .flatten()
            .find_map(|node| node.recognition)
            .context("Maa 未返回识别详情")?;
        Ok(serde_json::to_value(detail)?)
    }

    pub fn hold_key_probe(&mut self, key: i32, duration: Duration) -> Result<()> {
        let mode = self.input_mode.context("离线控制器不投递游戏输入")?;
        let valid_key = match mode {
            InputMode::Foreground | InputMode::Background => (1..=254).contains(&key),
            InputMode::Xbox | InputMode::Ds4 => {
                key > 0
                    && maa_framework::common::GamepadButton::from_bits(key as u32).is_some()
                    && (!matches!(mode, InputMode::Xbox) || key & 0x30000 == 0)
            }
        };
        ensure!(
            valid_key && !duration.is_zero() && duration <= Duration::from_secs(2),
            "输入探针需要有效 VK 或手柄按键位掩码，按住时间为 1..2000ms"
        );
        // 投递前登记，部分失败也必须尝试释放。
        self.held_keys.insert(key);
        let result = (|| {
            ensure!(
                self.controller
                    .wait(self.controller.post_key_down(key)?)
                    .is_success(),
                "按下失败"
            );
            std::thread::sleep(duration);
            Ok(())
        })();
        let cleanup = self.release_inputs();
        result.and(cleanup)
    }

    pub fn release_inputs(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for key in self.held_keys.clone() {
            match self.controller.post_key_up(key) {
                Ok(id) if self.controller.wait(id).is_success() => {
                    self.held_keys.remove(&key);
                }
                _ => errors.push(format!("按键 {key} 释放失败")),
            }
        }
        // inactive 放在队列末端并等待，完成前不交还控制权。
        match self.controller.post_inactive() {
            Ok(id) if self.controller.wait(id).is_success() => {}
            _ => errors.push("控制器恢复输入失败".into()),
        }
        ensure!(errors.is_empty(), "{}", errors.join("；"));
        Ok(())
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        if self.tasker.running()
            && let Ok(id) = self.tasker.post_stop()
        {
            Job::for_tasker(&self.tasker, id).wait();
        }
        let _ = self.release_inputs();
        self.tasker.clear_sinks();
    }
}
