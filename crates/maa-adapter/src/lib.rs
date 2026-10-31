pub mod agent_goal;
pub mod agent_mission;
pub mod agent_transport;
mod basic_daily;
pub mod challenge_move;
mod coffee;
pub mod suibian_temple;
pub use basic_daily::{
    basic_daily_agents, validate_basic_daily_app_bundle, validate_basic_daily_bundle,
};
mod coffee_run;
pub mod combat_contours;
mod combat_input;
pub use combat_input::CombatInputJob;
pub mod combat_vision;
mod combat_reader;
pub use combat_reader::{CombatReader, CombatVision};
mod combat_template;
pub use combat_template::CombatTemplate;
mod engagement_reward;
pub use coffee_run::{
    CoffeeRunObservation, validate_coffee_approach_bundle, validate_coffee_run_bundle,
};
pub use engagement_reward::{EngagementRewardObservation, validate_engagement_reward_bundle};
mod events;
mod fixture;
pub mod flash;
mod handoff;
mod inference;
mod input;
pub use handoff::{GameSessionConfig, SessionHandoff};
pub mod area_patrol;
pub mod charge_navigation;
pub mod charge_resources;
pub mod charge_transport;
pub mod compendium;
pub mod ether_synthesis;
pub mod ether_synthesis_entry;
pub mod expert_challenge;
mod exploration_navigation;
pub mod intel_board;
pub mod lost_void_battle;
pub mod lost_void_battle_hud;
pub mod lost_void_bounty;
pub mod lost_void_choice;
pub mod lost_void_classic_teams;
pub mod lost_void_deploy;
pub mod lost_void_entry;
pub mod lost_void_event;
pub mod lost_void_gear;
pub mod lost_void_interact;
pub mod lost_void_loading;
pub mod lost_void_matrix;
pub mod lost_void_navigation;
pub mod lost_void_npc;
pub mod lost_void_recovery;
pub mod lost_void_roster;
pub mod lost_void_scene;
pub mod lost_void_settlement;
pub mod lost_void_store;
pub mod lost_void_strategy;
mod mail_bundle;
pub mod map_exploration;
mod map_transport;
pub mod navigation;
pub mod navigation_vision;
mod predefined_team_slots;
pub mod predefined_teams;
mod random_play;
pub mod restore_charge;
pub mod simulation;
pub mod simulation_deploy;
pub mod simulation_mission;
pub mod simulation_settlement;
pub mod simulation_teams;
pub mod team_selection;
mod team_selection_slots;
pub mod world_patrol;
pub use coffee::{CoffeeObservation, validate_coffee_bundle};
pub use mail_bundle::validate_mail_bundle;
pub use map_transport::{MapObservation, validate_map_bundle};
pub use random_play::RandomPlayRead;
pub mod ocr;
mod pipeline;
mod raw_control;
pub mod raw_inference;
mod sdk;
pub mod yolo;
pub use events::{PipelineEvent, PipelineEvents};
pub use inference::{GpuLease, RecognitionImage, RecognitionMeasurement, RecognitionTiming};
pub use maa_framework::common::ControllerFeature;
pub use maa_framework::custom_controller::CustomControllerCallback;
pub use od_core::{
    CaptureMethod, InputMode, InputPipelineConfig, InputPipelineResult, InputProbeAction,
    InputProbeConfig, InputProbeResult, PipelineEnd, PipelineProbeResult,
};
pub use pipeline::{
    INPUT_PIPELINE_ENTRY, MailStep, PipelineExecution, PipelineInputPlan, PipelineNodeResult,
};
pub use raw_control::{
    RawRecognitionCancelled, RawRecognitionControl, RawRecognitionPhase, RawRecognitionStatus,
};
pub use sdk::{BINDING_VERSION, SDK_VERSION, load_sdk};

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use chrono::Utc;
use maa_framework::{
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
    sync::{Arc, Mutex, atomic::AtomicBool},
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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

// 回调订阅、Tasker、原始推理会话、Controller、Resource、控制权和 GPU 使用权按声明顺序销毁。
pub struct NativeSession {
    pipeline_subscriptions: Vec<events::PipelineSubscription>,
    tasker: Tasker,
    recognition_tasker: Tasker,
    raw_flash: Arc<Mutex<Option<raw_inference::RawClassifier>>>,
    raw_detectors: Mutex<BTreeMap<String, yolo::RawDetector>>,
    inference_gate: Arc<Mutex<()>>,
    resource_root: PathBuf,
    map_bundle: Option<PathBuf>,
    coffee_bundle: Option<PathBuf>,
    daily_bundle: Option<PathBuf>,
    suibian_temple_bundle: Option<PathBuf>,
    restore_charge_bundle: Option<PathBuf>,
    simulation_bundle: Option<PathBuf>,
    simulation_deploy_bundle: Option<PathBuf>,
    ether_synthesis_bundle: Option<PathBuf>,
    ether_synthesis_entry_bundle: Option<PathBuf>,
    charge_navigation_bundle: Option<PathBuf>,
    charge_transport_bundle: Option<PathBuf>,
    lost_void_scene_bundle: Option<PathBuf>,
    lost_void_loading_trace:
        std::sync::Arc<std::sync::Mutex<Option<lost_void_loading::LoadingTrace>>>,
    agent_transport_bundle: Option<PathBuf>,
    agent_goal_bundle: Option<PathBuf>,
    agent_mission_bundle: Option<PathBuf>,
    area_patrol_bundle: Option<PathBuf>,
    expert_challenge_bundle: Option<PathBuf>,
    deep_hunt_bundle: Option<PathBuf>,
    charge_level_bundle: Option<PathBuf>,
    simulation_mission_bundle: Option<PathBuf>,
    simulation_settlement_bundle: Option<PathBuf>,
    compendium_bundle: Option<PathBuf>,
    combat_bundle: Option<PathBuf>,
    detector_bundle: Option<PathBuf>,
    inference_device: InferenceDevice,
    raw_cpu_threads: usize,
    controller: fixture::NativeController,
    _recognition_controller: fixture::NativeController,
    _resource: Resource,
    input_mode: Option<InputMode>,
    game_window: Option<WindowInfo>,
    capture_method: Option<CaptureMethod>,
    held_keys: BTreeSet<i32>,
    patrol_steering: od_core::world_patrol::PatrolSteering,
    held_contacts: BTreeSet<i32>,
    input_cleanup_pending: bool,
    combat_capture: Option<combat_vision::CombatCapture>,
    pipeline_controller_pending: bool,
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

    pub fn for_fixture_with_cpu_threads(
        images: &Path,
        resources: &Path,
        device: InferenceDevice,
        cpu_threads: usize,
    ) -> Result<Self> {
        raw_inference::raw_threading(device, cpu_threads)?;
        let mut session = Self::for_fixture(images, resources, device)?;
        session.raw_cpu_threads = cpu_threads;
        Ok(session)
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
        ocr::initialize_color_filter(&resource)?;
        let tasker = Tasker::new()?;
        tasker.bind_resource(&resource)?;
        ensure!(
            unsafe { maa_framework::sys::MaaTaskerBindController(tasker.raw(), controller.raw()) }
                != 0,
            "绑定 Maa 控制器失败"
        );
        ensure!(tasker.inited(), "Maa Tasker 初始化失败");
        // PostRecognition 也会在任务结束时 inactive。绑定真实 Win32 控制器
        // 会让每个头像匹配都等待窗口/输入恢复，一帧累积超过战斗有效期。
        // 共享已校验资源，仅给只读识别绑定不具备输入能力的控制器。
        let recognition_controller =
            fixture::NativeController::from_callback(fixture::RecognitionController)?;
        ensure!(
            recognition_controller
                .wait(recognition_controller.post_connection()?)
                .is_success(),
            "只读识别控制器连接失败"
        );
        let recognition_tasker = Tasker::new()?;
        recognition_tasker.bind_resource(&resource)?;
        ensure!(
            unsafe {
                maa_framework::sys::MaaTaskerBindController(
                    recognition_tasker.raw(),
                    recognition_controller.raw(),
                )
            } != 0,
            "只读识别控制器绑定失败"
        );
        ensure!(recognition_tasker.inited(), "只读识别 Tasker 初始化失败");
        sdk::verify_loaded_modules()?;
        Ok(Self {
            pipeline_subscriptions: Vec::new(),
            tasker,
            recognition_tasker,
            raw_flash: Arc::new(Mutex::new(None)),
            raw_detectors: Mutex::new(BTreeMap::new()),
            inference_gate: Arc::new(Mutex::new(())),
            resource_root: path,
            map_bundle: None,
            coffee_bundle: None,
            daily_bundle: None,
            suibian_temple_bundle: None,
            restore_charge_bundle: None,
            simulation_bundle: None,
            simulation_deploy_bundle: None,
            ether_synthesis_bundle: None,
            ether_synthesis_entry_bundle: None,
            charge_navigation_bundle: None,
            charge_transport_bundle: None,
            lost_void_scene_bundle: None,
            lost_void_loading_trace: std::sync::Arc::new(std::sync::Mutex::new(None)),
            agent_transport_bundle: None,
            agent_goal_bundle: None,
            agent_mission_bundle: None,
            area_patrol_bundle: None,
            expert_challenge_bundle: None,
            deep_hunt_bundle: None,
            charge_level_bundle: None,
            simulation_mission_bundle: None,
            simulation_settlement_bundle: None,
            compendium_bundle: None,
            combat_bundle: None,
            detector_bundle: None,
            inference_device: device,
            raw_cpu_threads: raw_inference::DEFAULT_RAW_CPU_THREADS,
            controller,
            _recognition_controller: recognition_controller,
            _resource: resource,
            input_mode,
            game_window: None,
            capture_method,
            held_keys: BTreeSet::new(),
            patrol_steering: Default::default(),
            held_contacts: BTreeSet::new(),
            input_cleanup_pending: false,
            combat_capture: None,
            pipeline_controller_pending: false,
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
        let report = self.run_pipeline(
            "Prototype.HealthCheck",
            &PipelineInputPlan::default(),
            cancel,
            timeout,
        )?;
        if report.end == PipelineEnd::TimedOut {
            bail!("健康检查超时，原生队列已停止并完成输入释放");
        }
        ensure!(
            report.end != PipelineEnd::Failed,
            "Maa 健康检查失败：{}",
            report.native_status
        );
        Ok(ProbeResult {
            sdk_version: SDK_VERSION.into(),
            stopped: report.end == PipelineEnd::Cancelled,
            enqueue_us: report.enqueue_us,
            elapsed_ms: report.elapsed_ms,
            stop_ms: report.stop_ms,
            native_status: report.native_status,
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
        if matches!(kind, "LegacyFlashRaw" | "LegacyYoloDetect") {
            return self.recognize_prepared_cancellable(
                image,
                kind,
                params,
                &RawRecognitionControl::default(),
            );
        }
        let _guard = self
            .inference_gate
            .try_lock()
            .map_err(|_| anyhow::anyhow!("本会话已有识别调用，请等待完成"))?;
        ensure!(!self.tasker.running(), "Maa Tasker 正在执行其他任务");
        if kind == "LegacyFlashClassify" {
            return self.recognize_flash(image, params);
        }
        if kind == "OcrText" {
            return self.recognize_ocr_text(image, params);
        }
        self.recognize_maa_prepared(image, kind, params)
    }

    /// 原始闪光 / YOLO 的可取消调用；收到停止后仍等待在途原生调用返回。
    pub fn recognize_timed_cancellable(
        &self,
        image: &Path,
        kind: &str,
        params: &serde_json::Value,
        control: &RawRecognitionControl,
    ) -> Result<RecognitionMeasurement> {
        ensure!(
            matches!(kind, "LegacyFlashRaw" | "LegacyYoloDetect"),
            "此取消入口只支持原始闪光与 YOLO 推理"
        );
        control.execute(|| {
            let started = Instant::now();
            control.checkpoint(RawRecognitionPhase::ImageLoad)?;
            let prepared = RecognitionImage::load(image)?;
            let mut result = self.recognize_raw_prepared(&prepared, kind, params, control)?;
            result.timing.prepare_us += prepared.prepare_us;
            result.timing.total_us = started.elapsed().as_micros() as u64;
            Ok(result)
        })
    }

    /// 复用已准备的图像；控制对象只用于一次调用，取消返回 RawRecognitionCancelled。
    pub fn recognize_prepared_cancellable(
        &self,
        image: &RecognitionImage,
        kind: &str,
        params: &serde_json::Value,
        control: &RawRecognitionControl,
    ) -> Result<RecognitionMeasurement> {
        ensure!(
            matches!(kind, "LegacyFlashRaw" | "LegacyYoloDetect"),
            "此取消入口只支持原始闪光与 YOLO 推理"
        );
        control.execute(|| self.recognize_raw_prepared(image, kind, params, control))
    }

    fn recognize_raw_prepared(
        &self,
        image: &RecognitionImage,
        kind: &str,
        params: &serde_json::Value,
        control: &RawRecognitionControl,
    ) -> Result<RecognitionMeasurement> {
        control.checkpoint(RawRecognitionPhase::SessionSetup)?;
        let _guard = self
            .inference_gate
            .try_lock()
            .map_err(|_| anyhow::anyhow!("本会话已有识别调用，请等待完成"))?;
        ensure!(!self.tasker.running(), "Maa Tasker 正在执行其他任务");
        match kind {
            "LegacyFlashRaw" => self.recognize_raw_flash(image, params, control),
            "LegacyYoloDetect" => self.recognize_yolo(image, params, control),
            _ => unreachable!(),
        }
    }

    pub(crate) fn recognize_maa_prepared(
        &self,
        image: &RecognitionImage,
        kind: &str,
        params: &serde_json::Value,
    ) -> Result<RecognitionMeasurement> {
        combat_reader::recognize_maa(&self.recognition_tasker, image, kind, params)
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

    /// 深读页面前确认原生任务和所有登记输入均已收尾。
    pub fn inputs_released(&self) -> bool {
        !self.tasker.running()
            && self.combat_capture.is_none()
            && !self.input_cleanup_pending
            && !self.pipeline_controller_pending
            && self.held_keys.is_empty()
            && self.held_contacts.is_empty()
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        if self.recognition_tasker.running()
            && let Ok(id) = self.recognition_tasker.post_stop()
        {
            Job::for_tasker(&self.recognition_tasker, id).wait();
        }
        if self.tasker.running()
            && let Ok(id) = self.tasker.post_stop()
        {
            Job::for_tasker(&self.tasker, id).wait();
        }
        let _ = self.release_inputs();
        self.pipeline_subscriptions.clear();
        self.tasker.clear_sinks();
        self.recognition_tasker.clear_sinks();
    }
}

pub mod charge_level;
pub mod deep_hunt;
pub mod suibian_adventure;
pub mod suibian_boo_box;
pub mod suibian_craft;
pub mod suibian_good_goods;
pub mod suibian_sales;
pub mod suibian_yum;

pub mod agent_plan;
