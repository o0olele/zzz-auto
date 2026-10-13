use chrono::Utc;
use maa_adapter::{CaptureMethod, InferenceDevice, InputMode, NativeSession};
use od_core::{
    InputProbeConfig, RunEvent, RunRecord, RunSlot, RunState, SdkStatus, Snapshot, WindowInfo,
};
use od_protocol::{ApiError, Command, Payload};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};
use tokio::sync::oneshot;
use uuid::Uuid;

const EVENT_CAPACITY: usize = 256;

pub struct EnginePaths {
    pub sdk: PathBuf,
    pub resources: PathBuf,
    pub data: PathBuf,
}

struct State {
    sdk: SdkStatus,
    window: Option<WindowInfo>,
    capture_method: Option<CaptureMethod>,
    slot: RunSlot,
    native_pending: bool,
    cancel: Option<Arc<AtomicBool>>,
    events: VecDeque<RunEvent>,
    sequence: u64,
    dropped: u64,
    callback_dropped: Arc<AtomicU64>,
}

impl State {
    fn event(&mut self, run_id: Option<Uuid>, level: &str, message: String) {
        self.sequence += 1;
        if self.events.len() == EVENT_CAPACITY {
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(RunEvent {
            sequence: self.sequence,
            run_id,
            at: Utc::now(),
            level: level.into(),
            message,
        });
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            engine_version: env!("CARGO_PKG_VERSION").into(),
            sdk: self.sdk.clone(),
            window: self.window.clone(),
            capture_method: self.capture_method,
            run: self.slot.current().cloned(),
            events: self.events.iter().cloned().collect(),
            event_sequence: self.sequence,
            dropped_events: self.dropped + self.callback_dropped.load(Ordering::Relaxed),
        }
    }
}

enum Work {
    Attach(
        String,
        CaptureMethod,
        oneshot::Sender<Result<Payload, ApiError>>,
    ),
    Screenshot(oneshot::Sender<Result<Payload, ApiError>>),
    Probe(Uuid, Arc<AtomicBool>),
    InputProbe {
        id: Uuid,
        config: InputProbeConfig,
        window: WindowInfo,
        capture: CaptureMethod,
        cancel: Arc<AtomicBool>,
    },
    Shutdown(oneshot::Sender<()>),
}

#[derive(Clone)]
pub struct Engine {
    state: Arc<Mutex<State>>,
    sender: mpsc::SyncSender<Work>,
}

fn native_error(error: anyhow::Error) -> ApiError {
    ApiError::new("native_error", format!("{error:#}"))
}

impl Engine {
    pub fn new(paths: EnginePaths) -> anyhow::Result<Self> {
        std::fs::create_dir_all(paths.data.join("runs"))?;
        let state = Arc::new(Mutex::new(State {
            sdk: SdkStatus {
                ready: false,
                version: None,
                expected_version: maa_adapter::SDK_VERSION.into(),
                binding_version: maa_adapter::BINDING_VERSION.into(),
                directory: paths.sdk.display().to_string(),
                message: "正在校验并加载 Maa SDK".into(),
            },
            window: None,
            capture_method: None,
            slot: RunSlot::default(),
            native_pending: false,
            cancel: None,
            events: VecDeque::new(),
            sequence: 0,
            dropped: 0,
            callback_dropped: Arc::new(AtomicU64::new(0)),
        }));
        let (sender, receiver) = mpsc::sync_channel(8);
        let worker_state = state.clone();
        std::thread::Builder::new()
            .name("maa-worker".into())
            .spawn(move || worker(paths, worker_state, receiver))?;
        Ok(Self { state, sender })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().expect("运行状态锁失效").snapshot()
    }

    pub fn cancel_active(&self) {
        let mut state = self.state.lock().expect("运行状态锁失效");
        if let Some(cancel) = &state.cancel {
            cancel.store(true, Ordering::Release);
        }
        if let Some(id) = state
            .slot
            .current()
            .filter(|run| !run.state.is_terminal())
            .map(|run| run.id)
        {
            let _ = state.slot.request_stop(id);
        }
    }

    pub async fn shutdown(&self) -> Result<(), ApiError> {
        self.cancel_active();
        let (tx, rx) = oneshot::channel();
        self.sender
            .try_send(Work::Shutdown(tx))
            .map_err(|_| ApiError::new("worker_unavailable", "工作线程无法接收退出请求"))?;
        rx.await
            .map_err(|_| ApiError::new("worker_unavailable", "工作线程已异常退出"))
    }

    pub async fn execute(&self, command: Command) -> Result<Payload, ApiError> {
        match command {
            Command::Snapshot => Ok(Payload::Snapshot(Box::new(self.snapshot()))),
            Command::Applications => od_core::applications()
                .map(Payload::Applications)
                .map_err(|e| ApiError::new("catalog_error", e.to_string())),
            Command::Windows => od_platform::game_windows()
                .map(Payload::Windows)
                .map_err(native_error),
            Command::Events { after } => {
                let state = self.state.lock().expect("运行状态锁失效");
                Ok(Payload::Events(
                    state
                        .events
                        .iter()
                        .filter(|event| event.sequence > after)
                        .cloned()
                        .collect(),
                ))
            }
            Command::StartApplication { app_id } => {
                let apps = od_core::applications()
                    .map_err(|e| ApiError::new("catalog_error", e.to_string()))?;
                if !apps.iter().any(|app| app.id == app_id) {
                    return Err(ApiError::new("unknown_application", "应用 ID 未登记"));
                }
                Err(ApiError::new(
                    "not_migrated",
                    "该游戏应用尚未完成迁移与验收，目前仅开放原生集成诊断",
                ))
            }
            Command::StartProbe => {
                let mut state = self.state.lock().expect("运行状态锁失效");
                if !state.sdk.ready || state.window.is_none() {
                    return Err(ApiError::new(
                        "not_ready",
                        "请先加载 SDK 并连接 1920×1080 游戏窗口",
                    ));
                }
                if state.native_pending {
                    return Err(ApiError::new("busy", "截图或连接操作尚未完成"));
                }
                let record = state
                    .slot
                    .begin("prototype.health_check")
                    .map_err(|e| ApiError::new("busy", e))?;
                let cancel = Arc::new(AtomicBool::new(false));
                if self
                    .sender
                    .try_send(Work::Probe(record.id, cancel.clone()))
                    .is_err()
                {
                    let _ = state
                        .slot
                        .finish(record.id, RunState::Failed, "工作线程不可用".into());
                    return Err(ApiError::new("worker_unavailable", "工作线程无法接收任务"));
                }
                state.cancel = Some(cancel);
                state.event(
                    Some(record.id),
                    "info",
                    "开始 Maa 原生健康检查，无游戏输入".into(),
                );
                Ok(Payload::Run(record))
            }
            Command::StartInputProbe {
                config,
                allow_input,
            } => {
                if !allow_input {
                    return Err(ApiError::new(
                        "input_not_confirmed",
                        "输入诊断会操作游戏，请先确认允许真实输入",
                    ));
                }
                config
                    .validate()
                    .map_err(|e| ApiError::new("invalid_input", e))?;
                let mut state = self.state.lock().expect("运行状态锁失效");
                if !state.sdk.ready {
                    return Err(ApiError::new("sdk_not_ready", state.sdk.message.clone()));
                }
                let window = state
                    .window
                    .clone()
                    .ok_or_else(|| ApiError::new("not_connected", "请先连接游戏窗口"))?;
                if state.native_pending {
                    return Err(ApiError::new("busy", "截图或连接操作尚未完成"));
                }
                let capture = state.capture_method.unwrap_or_default();
                let record = state
                    .slot
                    .begin("prototype.input_probe")
                    .map_err(|e| ApiError::new("busy", e))?;
                let cancel = Arc::new(AtomicBool::new(false));
                if self
                    .sender
                    .try_send(Work::InputProbe {
                        id: record.id,
                        config,
                        window,
                        capture,
                        cancel: cancel.clone(),
                    })
                    .is_err()
                {
                    let _ = state
                        .slot
                        .finish(record.id, RunState::Failed, "工作线程不可用".into());
                    return Err(ApiError::new("worker_unavailable", "工作线程无法接收任务"));
                }
                state.cancel = Some(cancel);
                state.event(
                    Some(record.id),
                    "info",
                    format!(
                        "开始真实输入诊断：{:?}，动作 {:?}，延时 {}ms，按住 {}ms",
                        config.mode, config.action, config.start_delay_ms, config.hold_ms
                    ),
                );
                Ok(Payload::Run(record))
            }
            Command::Stop { run_id } => {
                let mut state = self.state.lock().expect("运行状态锁失效");
                let record = state
                    .slot
                    .request_stop(run_id)
                    .map_err(|e| ApiError::new("stale_run", e))?;
                if let Some(cancel) = &state.cancel {
                    cancel.store(true, Ordering::Release);
                }
                Ok(Payload::Run(record))
            }
            Command::Attach {
                hwnd,
                capture_method,
            } => self.native_request(Some((hwnd, capture_method))).await,
            Command::Screenshot => self.native_request(None).await,
            Command::Shutdown => {
                self.shutdown().await?;
                Ok(Payload::Ack)
            }
        }
    }

    async fn native_request(
        &self,
        connection: Option<(String, CaptureMethod)>,
    ) -> Result<Payload, ApiError> {
        let (tx, rx) = oneshot::channel();
        {
            let mut state = self.state.lock().expect("运行状态锁失效");
            if !state.sdk.ready {
                return Err(ApiError::new("sdk_not_ready", state.sdk.message.clone()));
            }
            if state.slot.busy() || state.native_pending {
                return Err(ApiError::new(
                    "busy",
                    "原生控制器正在使用，请等待当前操作结束",
                ));
            }
            state.native_pending = true;
            let work = match connection {
                Some((hwnd, capture_method)) => Work::Attach(hwnd, capture_method, tx),
                None => Work::Screenshot(tx),
            };
            if self.sender.try_send(work).is_err() {
                state.native_pending = false;
                return Err(ApiError::new("worker_unavailable", "工作线程不可用"));
            }
        }
        rx.await
            .map_err(|_| ApiError::new("worker_unavailable", "工作线程在原生操作期间退出"))?
    }
}

fn worker(paths: EnginePaths, state: Arc<Mutex<State>>, receiver: mpsc::Receiver<Work>) {
    {
        let result = maa_adapter::load_sdk(&paths.sdk, &paths.data.join("maa-log"));
        let mut state = state.lock().expect("运行状态锁失效");
        match result {
            Ok(sdk) => {
                state.sdk = sdk;
                state.event(None, "info", "Maa SDK 校验与加载成功".into());
            }
            Err(error) => {
                state.sdk.message = format!("{error:#}");
                let message = state.sdk.message.clone();
                state.event(None, "error", message);
            }
        }
    }
    let mut native: Option<NativeSession> = None;
    for command in receiver {
        match command {
            Work::Attach(hwnd, capture_method, reply) => {
                let result = (|| -> anyhow::Result<Payload> {
                    let window = od_platform::parse_game_window(&hwnd)?;
                    anyhow::ensure!(
                        window.width == 1920 && window.height == 1080,
                        "游戏客户区须为 1920×1080，当前为 {}×{}",
                        window.width,
                        window.height
                    );
                    // 释放旧控制器与控制权，再创建新控制器；切换中不保留旧窗口快照。
                    if let Some(controller) = &mut native {
                        controller.release_inputs()?;
                    }
                    native.take();
                    {
                        let mut state = state.lock().expect("运行状态锁失效");
                        state.window = None;
                        state.capture_method = None;
                    }
                    let controller = NativeSession::for_game(
                        &window,
                        &paths.resources,
                        capture_method,
                        InputMode::Foreground,
                        InferenceDevice::Cpu,
                    )?;
                    watch_native(&controller, &state)?;
                    native = Some(controller);
                    let mut state = state.lock().expect("运行状态锁失效");
                    state.window = Some(window);
                    state.capture_method = Some(capture_method);
                    state.event(
                        None,
                        "info",
                        format!("已连接 Maa 控制器，截图方式：{}", capture_method.name()),
                    );
                    Ok(Payload::Snapshot(Box::new(state.snapshot())))
                })()
                .map_err(native_error);
                {
                    let mut state = state.lock().expect("运行状态锁失效");
                    state.native_pending = false;
                    if let Err(error) = &result {
                        state.event(
                            None,
                            "error",
                            format!("连接 {} 失败：{}", capture_method.name(), error.message),
                        );
                    }
                }
                let _ = reply.send(result);
            }
            Work::Screenshot(reply) => {
                let result = native
                    .as_ref()
                    .ok_or_else(|| ApiError::new("not_connected", "尚未连接游戏窗口"))
                    .and_then(|native| {
                        native
                            .screenshot()
                            .map(Payload::Screenshot)
                            .map_err(native_error)
                    });
                {
                    let mut state = state.lock().expect("运行状态锁失效");
                    state.native_pending = false;
                    match &result {
                        Ok(Payload::Screenshot(frame)) => state.event(
                            None,
                            if frame.quality.near_black {
                                "warn"
                            } else {
                                "info"
                            },
                            format!(
                                "{} 截图：{}×{}，亮度 {:.2}/255，暗像素 {:.2}%，{} ms{}",
                                frame
                                    .capture_method
                                    .map(CaptureMethod::name)
                                    .unwrap_or("离线"),
                                frame.width,
                                frame.height,
                                frame.quality.mean_luma,
                                frame.quality.dark_pixel_ratio * 100.0,
                                frame.elapsed_ms,
                                if frame.quality.near_black {
                                    "；疑似黑帧，请退出加载画面或切换截图方式"
                                } else {
                                    ""
                                }
                            ),
                        ),
                        Err(error) => state.event(None, "error", error.message.clone()),
                        _ => {}
                    }
                }
                let _ = reply.send(result);
            }
            Work::Probe(id, cancel) => {
                let result = native
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("控制器未连接"))
                    .and_then(|native| {
                        native.health_check(cancel.clone(), Duration::from_secs(10))
                    });
                let (terminal, message) = match result {
                    Ok(result) => (
                        if result.stopped {
                            RunState::Stopped
                        } else {
                            RunState::Succeeded
                        },
                        serde_json::to_string(&result).unwrap_or_else(|_| "健康检查已完成".into()),
                    ),
                    Err(error) => (RunState::Failed, format!("{error:#}")),
                };
                let mut state = state.lock().expect("运行状态锁失效");
                let terminal = if terminal == RunState::Succeeded
                    && state
                        .slot
                        .current()
                        .is_some_and(|run| run.state == RunState::Stopping)
                {
                    RunState::Stopped
                } else {
                    terminal
                };
                if let Ok(record) = state.slot.finish(id, terminal, message.clone()) {
                    save_run(&paths, &mut state, &record);
                }
                state.cancel = None;
                state.event(
                    Some(id),
                    if terminal == RunState::Failed {
                        "error"
                    } else {
                        "info"
                    },
                    message,
                );
            }
            Work::InputProbe {
                id,
                config,
                window,
                capture,
                cancel,
            } => {
                let result = (|| -> anyhow::Result<_> {
                    // 延时前已收到停止时不切换控制器，也不投递按下。
                    if !cancel.load(Ordering::Acquire) {
                        if let Some(controller) = &mut native {
                            controller.release_inputs()?;
                        }
                        native.take();
                        {
                            let mut state = state.lock().expect("运行状态锁失效");
                            state.window = None;
                            state.capture_method = None;
                        }
                        // 重新解析句柄，不能沿用游戏重启前的进程身份。
                        let current = od_platform::parse_game_window(&window.hwnd)?;
                        anyhow::ensure!(
                            current.pid == window.pid,
                            "游戏窗口身份已改变，请重新连接"
                        );
                        anyhow::ensure!(
                            current.width == 1920 && current.height == 1080,
                            "游戏客户区须为 1920×1080"
                        );
                        let controller = NativeSession::for_game(
                            &current,
                            &paths.resources,
                            capture,
                            config.mode,
                            InferenceDevice::Cpu,
                        )?;
                        watch_native(&controller, &state)?;
                        native = Some(controller);
                        let mut state = state.lock().expect("运行状态锁失效");
                        state.window = Some(current);
                        state.capture_method = Some(capture);
                    }
                    // 取消发生在重建前时，原控制器模式可能不同；返回未按下的终态。
                    if cancel.load(Ordering::Acquire) {
                        let started = std::time::Instant::now();
                        if let Some(controller) = &mut native {
                            controller.release_inputs()?;
                        }
                        return Ok(od_core::InputProbeResult {
                            config,
                            pressed: false,
                            released: true,
                            stopped: true,
                            enqueue_us: 0,
                            key_down_ms: 0,
                            held_ms: 0,
                            release_ms: started.elapsed().as_millis() as u64,
                            stop_ms: Some(started.elapsed().as_millis() as u64),
                            elapsed_ms: started.elapsed().as_millis() as u64,
                        });
                    }
                    native
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("控制器未连接"))?
                        .input_probe(config, cancel)
                })();
                let mut state = state.lock().expect("运行状态锁失效");
                let record = match result {
                    Ok(mut result) => {
                        if state
                            .slot
                            .current()
                            .is_some_and(|run| run.state == RunState::Stopping)
                        {
                            result.stopped = true;
                        }
                        state.slot.finish_input_probe(id, result)
                    }
                    Err(error) => state
                        .slot
                        .finish(id, RunState::Failed, format!("{error:#}")),
                };
                if let Ok(record) = record {
                    save_run(&paths, &mut state, &record);
                    state.event(
                        Some(id),
                        if record.state == RunState::Failed {
                            "error"
                        } else {
                            "info"
                        },
                        record.message,
                    );
                }
                state.cancel = None;
            }
            Work::Shutdown(reply) => {
                native.take();
                let _ = reply.send(());
                return;
            }
        }
    }
}

fn watch_native(controller: &NativeSession, state: &Arc<Mutex<State>>) -> anyhow::Result<()> {
    let sink_state = state.clone();
    let dropped = state
        .lock()
        .expect("运行状态锁失效")
        .callback_dropped
        .clone();
    controller.add_event_sink(move |message| {
        // 回调不等待锁；日志拥堵时允许丢弃，终态另行保存。
        if let Ok(mut state) = sink_state.try_lock() {
            let run_id = state.slot.current().map(|run| run.id);
            state.event(run_id, "debug", message);
        } else {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
    })
}

fn save_run(paths: &EnginePaths, state: &mut State, record: &RunRecord) {
    let path = paths.data.join("runs").join(format!("{}.json", record.id));
    let saved = (|| -> anyhow::Result<()> {
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(record)?)?;
        std::fs::rename(temp, path)?;
        Ok(())
    })();
    if let Err(error) = saved {
        state.event(
            Some(record.id),
            "error",
            format!("运行结果保存失败：{error}"),
        );
    }
}
