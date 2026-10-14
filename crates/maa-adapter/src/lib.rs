mod fixture;
pub mod flash;
mod inference;
mod input;
pub mod raw_inference;
mod sdk;
pub mod yolo;
pub use inference::{GpuLease, RecognitionImage, RecognitionMeasurement, RecognitionTiming};
pub use maa_framework::common::ControllerFeature;
pub use maa_framework::custom_controller::CustomControllerCallback;
pub use od_core::{CaptureMethod, InputMode, InputProbeAction, InputProbeConfig, InputProbeResult};
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
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
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

// Tasker、原始推理会话、Controller、Resource、控制权和 GPU 使用权按声明顺序销毁。
pub struct NativeSession {
    tasker: Tasker,
    raw_flash: Mutex<Option<raw_inference::RawClassifier>>,
    raw_detectors: Mutex<BTreeMap<String, yolo::RawDetector>>,
    inference_gate: Mutex<()>,
    resource_root: PathBuf,
    inference_device: InferenceDevice,
    controller: fixture::NativeController,
    _resource: Resource,
    input_mode: Option<InputMode>,
    game_window: Option<WindowInfo>,
    capture_method: Option<CaptureMethod>,
    held_keys: BTreeSet<i32>,
    held_contacts: BTreeSet<i32>,
    input_cleanup_pending: bool,
    _lease: Option<GameLease>,
    _gpu_lease: Option<GpuLease>,
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
                    InputMode::BackgroundPost => Win32InputMethod::POST_MESSAGE,
                    InputMode::ForegroundLegacy => Win32InputMethod::LEGACY_EVENT,
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
        let mut session = Self::bind(
            fixture::NativeController::Game(controller),
            resources,
            device,
            Some(lease),
            Some(input),
            Some(capture),
        )?;
        session.game_window = Some(window.clone());
        Ok(session)
    }

    pub fn for_fixture(images: &Path, resources: &Path, device: InferenceDevice) -> Result<Self> {
        let controller = fixture::NativeController::from_image(images)?;
        Self::bind(controller, resources, device, None, None, None)
    }

    // 离线验证入口：回调由测试仓提供，使用真实 Maa 队列，不取得游戏控制权。
    pub fn for_custom_controller(
        callback: impl maa_framework::custom_controller::CustomControllerCallback + 'static,
        resources: &Path,
        input: InputMode,
    ) -> Result<Self> {
        Self::bind(
            fixture::NativeController::from_callback(callback)?,
            resources,
            InferenceDevice::Cpu,
            None,
            Some(input),
            None,
        )
    }

    fn bind(
        controller: fixture::NativeController,
        resources: &Path,
        device: InferenceDevice,
        lease: Option<GameLease>,
        input_mode: Option<InputMode>,
        capture_method: Option<CaptureMethod>,
    ) -> Result<Self> {
        let gpu_lease = match device {
            InferenceDevice::Cpu => None,
            InferenceDevice::DirectMl(id) => Some(GpuLease::acquire(id)?),
        };
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
            raw_flash: Mutex::new(None),
            raw_detectors: Mutex::new(BTreeMap::new()),
            inference_gate: Mutex::new(()),
            resource_root: path,
            inference_device: device,
            controller,
            _resource: resource,
            input_mode,
            game_window: None,
            capture_method,
            held_keys: BTreeSet::new(),
            held_contacts: BTreeSet::new(),
            input_cleanup_pending: false,
            _lease: lease,
            _gpu_lease: gpu_lease,
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
        if self.input_cleanup_pending {
            self.release_inputs()?;
        }
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
        Ok(self.recognize_timed(image, kind, params)?.detail)
    }

    pub fn recognize_timed(
        &self,
        image: &Path,
        kind: &str,
        params: &serde_json::Value,
    ) -> Result<RecognitionMeasurement> {
        let started = Instant::now();
        let prepared = RecognitionImage::load(image)?;
        let mut result = self.recognize_prepared(&prepared, kind, params)?;
        result.timing.prepare_us += prepared.prepare_us;
        result.timing.total_us = started.elapsed().as_micros() as u64;
        Ok(result)
    }

    pub fn recognize_prepared(
        &self,
        image: &RecognitionImage,
        kind: &str,
        params: &serde_json::Value,
    ) -> Result<RecognitionMeasurement> {
        let _guard = self
            .inference_gate
            .try_lock()
            .map_err(|_| anyhow::anyhow!("本会话已有识别调用，请等待完成"))?;
        ensure!(!self.tasker.running(), "Maa Tasker 正在执行其他任务");
        if kind == "LegacyFlashClassify" {
            return self.recognize_flash(image, params);
        }
        if kind == "LegacyFlashRaw" {
            return self.recognize_raw_flash(image, params);
        }
        if kind == "LegacyYoloDetect" {
            return self.recognize_yolo(image, params);
        }
        self.recognize_maa_prepared(image, kind, params)
    }

    pub(crate) fn recognize_maa_prepared(
        &self,
        image: &RecognitionImage,
        kind: &str,
        params: &serde_json::Value,
    ) -> Result<RecognitionMeasurement> {
        let started = Instant::now();
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
        let enqueue_at = Instant::now();
        let job = self
            .tasker
            .post_recognition(kind, &params.to_string(), &image.buffer)?;
        let enqueue_us = enqueue_at.elapsed().as_micros() as u64;
        let completion_at = Instant::now();
        let status = job.wait();
        ensure!(
            status.is_success() || status == MaaStatus::FAILED,
            "Maa 识别调用状态无效：{status}"
        );
        let completion_us = completion_at.elapsed().as_micros() as u64;
        let details_at = Instant::now();
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
        // 未命中可能返回 Failed，但有完整 recognition 详情，不能当作原生异常。
        ensure!(
            status.is_success() || !detail.hit,
            "Maa 识别任务失败但详情报告命中"
        );
        Ok(RecognitionMeasurement {
            detail: serde_json::to_value(detail)?,
            native_status: status.to_string(),
            timing: RecognitionTiming {
                session_setup_us: 0,
                prepare_us: 0,
                enqueue_us,
                completion_us,
                details_us: details_at.elapsed().as_micros() as u64,
                total_us: started.elapsed().as_micros() as u64,
            },
        })
    }

    pub fn hold_key_probe(&mut self, key: i32, duration: Duration) -> Result<()> {
        let mode = self.input_mode.context("离线控制器不投递游戏输入")?;
        ensure!(
            duration <= Duration::from_secs(2),
            "按住时间不能超过 2000ms"
        );
        self.input_probe(
            InputProbeConfig {
                mode,
                action: InputProbeAction::Key { key },
                hold_ms: duration.as_millis() as u32,
                start_delay_ms: 0,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .map(|_| ())
    }

    pub fn release_inputs(&mut self) -> Result<()> {
        self.release_inputs_observed(&AtomicBool::new(false), &mut None)
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
