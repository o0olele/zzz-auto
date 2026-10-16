use crate::mail::{MailRun, MailState};
use chrono::Utc;
use maa_adapter::{CaptureMethod, InferenceDevice, InputMode, NativeSession};
use od_core::config::{ConfigError, ConfigStore};
use od_core::{
    InputProbeConfig, RunEvent, RunRecord, RunSlot, RunState, SdkStatus, Snapshot, WindowInfo,
};
use od_core::{mail::MailConfig, pipeline::PipelineRequest};
use od_core::{
    run_history::RunStore,
    run_plan::{RunContext, plan_group},
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
    record_error: Option<String>,
    failed_save: Option<RunRecord>,
    sdk: SdkStatus,
    window: Option<WindowInfo>,
    capture_method: Option<CaptureMethod>,
    slot: RunSlot,
    native_pending: bool,
    cancel: Option<Arc<AtomicBool>>,
    mail_control: Option<PipelineRequest>,
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
            record_error: self.record_error.clone(),
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
    Mail(MailWork),
    MailControl(Uuid, bool),
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
        pipeline_timeout: Option<u32>,
        window: WindowInfo,
        capture: CaptureMethod,
        cancel: Arc<AtomicBool>,
    },
    Shutdown(oneshot::Sender<()>),
}

struct MailWork {
    fixture: bool,
    run: Box<MailRun>,
    window: WindowInfo,
    capture: CaptureMethod,
    prepared: bool,
    events: Option<maa_adapter::PipelineEvents>,
    dropped: u64,
}

enum InputOutcome {
    Direct(od_core::InputProbeResult),
    Pipeline(od_core::InputPipelineResult),
}

#[derive(Clone)]
pub struct Engine {
    state: Arc<Mutex<State>>,
    sender: mpsc::SyncSender<Work>,
    fixture: bool,
    config: ConfigStore,
    history: RunStore,
    _config_owner: Arc<std::fs::File>,
}

fn native_error(error: anyhow::Error) -> ApiError {
    ApiError::new("native_error", format!("{error:#}"))
}

fn config_error(error: ConfigError) -> ApiError {
    match error {
        ConfigError::Conflict(latest) => ApiError {
            code: "config_conflict".into(),
            message: "配置已被其他窗口或进程修改，请核对最新配置后重新编辑".into(),
            details: serde_json::to_string(&latest).ok(),
        },
        error => ApiError::new("config_error", error.to_string()),
    }
}

impl Engine {
    pub fn new(paths: EnginePaths) -> anyhow::Result<Self> {
        Self::with_factory(paths, None)
    }

    // 独立测试仓的会反应控制器入口，不通过 CLI / IPC 暴露，也不创建游戏控制器。
    pub fn for_mail_fixture(
        paths: EnginePaths,
        callback: impl maa_adapter::CustomControllerCallback + 'static,
    ) -> anyhow::Result<Self> {
        let resources = paths.resources.clone();
        Self::with_factory(
            paths,
            Some(Box::new(move || {
                NativeSession::for_custom_controller(callback, &resources, InputMode::Foreground)
            })),
        )
    }

    fn with_factory(
        paths: EnginePaths,
        factory: Option<Box<dyn FnOnce() -> anyhow::Result<NativeSession> + Send>>,
    ) -> anyhow::Result<Self> {
        let fixture = factory.is_some();
        let config = ConfigStore::new(&paths.data)?;
        let config_owner = Arc::new(config.acquire_owner()?);
        config.read()?;
        let history = RunStore::new(&paths.data)?;
        std::fs::create_dir_all(paths.data.join("runs"))?;
        let state = Arc::new(Mutex::new(State {
            record_error: None,
            failed_save: None,
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
            mail_control: None,
            events: VecDeque::new(),
            sequence: 0,
            dropped: 0,
            callback_dropped: Arc::new(AtomicU64::new(0)),
        }));
        let (sender, receiver) = mpsc::sync_channel(8);
        let worker_state = state.clone();
        let worker_owner = config_owner.clone();
        std::thread::Builder::new()
            .name("maa-worker".into())
            .spawn(move || {
                let _owner = worker_owner;
                worker(paths, worker_state, receiver, factory);
            })?;
        Ok(Self {
            state,
            sender,
            fixture,
            config,
            history,
            _config_owner: config_owner,
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().expect("运行状态锁失效").snapshot()
    }

    pub fn cancel_active(&self) {
        let mut state = self.state.lock().expect("运行状态锁失效");
        if let Some(cancel) = &state.cancel {
            cancel.store(true, Ordering::Release);
        }
        if let Some(control) = &state.mail_control {
            control.stop();
        }
        if let Some(id) = state
            .slot
            .current()
            .filter(|run| !run.state.is_terminal())
            .map(|run| run.id)
        {
            let was_stopping = state
                .slot
                .current()
                .is_some_and(|run| run.state == RunState::Stopping);
            let _ = state.slot.request_stop(id);
            if state.mail_control.is_some() && !was_stopping {
                let _ = self.sender.try_send(Work::MailControl(id, false));
            }
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
            Command::RetryRunSave => {
                let mut state = self.state.lock().expect("运行状态锁失效");
                if let Some(record) = &state.failed_save {
                    self.history
                        .save(record)
                        .map_err(|error| ApiError::new("history_error", error.to_string()))?;
                    let id = record.id;
                    state.failed_save = None;
                    state.record_error = None;
                    state.event(
                        Some(id),
                        "info",
                        "运行记录已重新保存，未执行游戏动作".into(),
                    );
                }
                Ok(Payload::Snapshot(Box::new(state.snapshot())))
            }
            Command::PreviewGroup {
                instance_id,
                group_id,
            } => {
                let config = self.config.read().map_err(config_error)?;
                plan_group(&config, instance_id, &group_id)
                    .map(Payload::GroupPlan)
                    .map_err(|error| ApiError::new("invalid_group", error))
            }
            Command::RunHistory { instance_id } => {
                let config = self.config.read().map_err(config_error)?;
                if !config
                    .instances
                    .iter()
                    .any(|instance| instance.id == instance_id)
                {
                    return Err(ApiError::new("unknown_instance", "账号不存在"));
                }
                let live = {
                    let state = self.state.lock().expect("运行状态锁失效");
                    state
                        .slot
                        .current()
                        .filter(|run| {
                            !run.state.is_terminal()
                                && run
                                    .context
                                    .as_ref()
                                    .is_some_and(|context| context.instance_id == instance_id)
                        })
                        .map(|run| run.id)
                };
                let history = self.history.clone();
                tokio::task::spawn_blocking(move || history.history(instance_id, live))
                    .await
                    .map_err(|_| ApiError::new("history_error", "历史查询线程异常退出"))?
                    .map(Payload::RunHistory)
                    .map_err(|error| ApiError::new("history_error", error.to_string()))
            }
            Command::StartGroupTrial {
                instance_id,
                group_id,
                revision,
                config,
                allow_input,
                confirm_account,
            } => {
                if !confirm_account {
                    return Err(ApiError::new(
                        "account_not_confirmed",
                        "请先确认游戏登录账号与所选账号一致",
                    ));
                }
                self.start_mail(config, allow_input, Some((instance_id, group_id, revision)))
            }
            Command::AccountConfig => self
                .config
                .read()
                .map(Payload::AccountConfig)
                .map_err(config_error),
            Command::UpdateAccountConfig { revision, change } => {
                let mut state = self.state.lock().expect("运行状态锁失效");
                if state.native_pending
                    || state
                        .slot
                        .current()
                        .is_some_and(|run| !run.state.is_terminal())
                {
                    return Err(ApiError::new(
                        "busy",
                        "请先停止任务并等待原生请求与输入收尾，再修改账号或应用组",
                    ));
                }
                let config = self.config.update(revision, change).map_err(config_error)?;
                state.event(
                    None,
                    "info",
                    format!("账号与应用组配置已保存，修订 {}", config.revision),
                );
                Ok(Payload::AccountConfig(config))
            }
            Command::StartMail {
                config,
                allow_input,
            } => self.start_mail(config, allow_input, None),
            Command::Pause { run_id } => {
                let mut state = self.state.lock().expect("运行状态锁失效");
                let record = state
                    .slot
                    .request_pause(run_id)
                    .map_err(|error| ApiError::new("invalid_run_state", error))?;
                if let Some(control) = &state.mail_control {
                    control.pause();
                }
                Ok(Payload::Run(record))
            }
            Command::Resume { run_id } => {
                let mut state = self.state.lock().expect("运行状态锁失效");
                let current = state
                    .slot
                    .current()
                    .filter(|run| {
                        run.id == run_id && run.state == RunState::Paused && run.mail.is_some()
                    })
                    .ok_or_else(|| {
                        ApiError::new("invalid_run_state", "只有已确认暂停的邮件任务可以继续")
                    })?;
                if self
                    .sender
                    .try_send(Work::MailControl(current.id, true))
                    .is_err()
                {
                    return Err(ApiError::new(
                        "worker_unavailable",
                        "工作线程无法接收继续请求",
                    ));
                }
                let record = state
                    .slot
                    .request_resume(run_id)
                    .map_err(|error| ApiError::new("invalid_run_state", error))?;
                Ok(Payload::Run(record))
            }
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
                if state.record_error.is_some() {
                    return Err(ApiError::new(
                        "history_error",
                        "请先修复目录并重试保存运行记录",
                    ));
                }
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
            } => self.start_input(config, allow_input, None),
            Command::StartInputPipeline {
                config,
                allow_input,
            } => self.start_input(config.input, allow_input, Some(config.timeout_ms)),
            Command::Stop { run_id } => {
                let mut state = self.state.lock().expect("运行状态锁失效");
                let was_stopping = state
                    .slot
                    .current()
                    .is_some_and(|run| run.state == RunState::Stopping);
                let record = state
                    .slot
                    .request_stop(run_id)
                    .map_err(|e| ApiError::new("stale_run", e))?;
                if let Some(cancel) = &state.cancel {
                    cancel.store(true, Ordering::Release);
                }
                if let Some(control) = &state.mail_control {
                    control.stop();
                    if !was_stopping {
                        let _ = self.sender.try_send(Work::MailControl(run_id, false));
                    }
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

    fn start_mail(
        &self,
        config: MailConfig,
        allow_input: bool,
        group: Option<(u32, String, u32)>,
    ) -> Result<Payload, ApiError> {
        if !allow_input {
            return Err(ApiError::new(
                "input_not_confirmed",
                "邮件流程会操作游戏，请先确认允许真实输入",
            ));
        }
        config
            .validate()
            .map_err(|error| ApiError::new("invalid_input", error))?;
        let mut state = self.state.lock().expect("运行状态锁失效");
        if state.record_error.is_some() {
            return Err(ApiError::new(
                "history_error",
                "请先修复目录并重试保存运行记录",
            ));
        }
        let context: Option<RunContext> = if let Some((instance_id, group_id, revision)) = group {
            if state.slot.busy() || state.native_pending {
                return Err(ApiError::new("busy", "已有任务或原生请求尚未完成"));
            }
            let saved = self.config.read().map_err(config_error)?;
            if saved.revision != revision {
                return Err(config_error(ConfigError::Conflict(Box::new(saved))));
            }
            let plan = plan_group(&saved, instance_id, &group_id)
                .map_err(|error| ApiError::new("invalid_group", error))?;
            if !plan.executable {
                return Err(ApiError::new("group_blocked", plan.blockers.join("；")));
            }
            Some(plan.context)
        } else {
            None
        };
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
        let record = match context {
            Some(context) => state.slot.begin_group_mail(config, context),
            None => state.slot.begin_mail(config),
        }
        .map_err(|error| ApiError::new("busy", error))?;
        let run = MailRun::with_id(record.id, config).map_err(native_error)?;
        if record.context.is_some()
            && let Err(error) = self.history.save(&record)
        {
            let message = format!("启动记录保存失败，未投递游戏动作：{error}");
            let mut progress = record.mail.clone().expect("邮件进度已创建");
            progress.state = MailState::Failed;
            progress.inputs_released = true;
            state.failed_save = state
                .slot
                .update_mail(record.id, progress, message.clone())
                .ok();
            state.record_error = Some(message.clone());
            state.event(Some(record.id), "error", message.clone());
            return Err(ApiError::new("history_error", message));
        }
        let control = run.control();
        let work = MailWork {
            fixture: self.fixture,
            run: Box::new(run),
            window,
            capture: state.capture_method.unwrap_or_default(),
            prepared: false,
            events: None,
            dropped: 0,
        };
        if self.sender.try_send(Work::Mail(work)).is_err() {
            let mut progress = record.mail.clone().expect("邮件进度已创建");
            progress.state = MailState::Failed;
            progress.inputs_released = true;
            if let Ok(failed) =
                state
                    .slot
                    .update_mail(record.id, progress, "工作线程不可用，未投递游戏动作".into())
                && let Err(error) = self.history.save(&failed)
            {
                state.failed_save = Some(failed);
                state.record_error = Some(format!("运行 {} 的结果保存失败：{error}", record.id));
            }
            return Err(ApiError::new(
                "worker_unavailable",
                "工作线程无法接收邮件任务",
            ));
        }
        state.mail_control = Some(control);
        state.event(
            Some(record.id),
            "info",
            "开始邮件试运行；只使用前台键鼠，暂停期间继续占用运行槽".into(),
        );
        Ok(Payload::Run(record))
    }

    fn start_input(
        &self,
        config: InputProbeConfig,
        allow_input: bool,
        pipeline_timeout: Option<u32>,
    ) -> Result<Payload, ApiError> {
        if !allow_input {
            return Err(ApiError::new(
                "input_not_confirmed",
                "输入诊断会操作游戏，请先确认允许真实输入",
            ));
        }
        config
            .validate()
            .map_err(|e| ApiError::new("invalid_input", e))?;
        if let Some(timeout_ms) = pipeline_timeout {
            od_core::InputPipelineConfig {
                input: config,
                timeout_ms,
            }
            .validate()
            .map_err(|e| ApiError::new("invalid_input", e))?;
        }
        let mut state = self.state.lock().expect("运行状态锁失效");
        if state.record_error.is_some() {
            return Err(ApiError::new(
                "history_error",
                "请先修复目录并重试保存运行记录",
            ));
        }
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
            .begin(if pipeline_timeout.is_some() {
                "prototype.input_pipeline"
            } else {
                "prototype.input_probe"
            })
            .map_err(|e| ApiError::new("busy", e))?;
        let cancel = Arc::new(AtomicBool::new(false));
        if self
            .sender
            .try_send(Work::InputProbe {
                id: record.id,
                config,
                pipeline_timeout,
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
                "开始真实输入诊断：{:?}，动作 {:?}，延时 {}ms，按住 {}ms，Pipeline 超时 {:?}",
                config.mode, config.action, config.start_delay_ms, config.hold_ms, pipeline_timeout
            ),
        );
        Ok(Payload::Run(record))
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

fn worker(
    paths: EnginePaths,
    state: Arc<Mutex<State>>,
    receiver: mpsc::Receiver<Work>,
    factory: Option<Box<dyn FnOnce() -> anyhow::Result<NativeSession> + Send>>,
) {
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
    if let Some(factory) = factory {
        match factory() {
            Ok(session) => {
                native = Some(session);
                state.lock().expect("运行状态锁失效").window = Some(WindowInfo {
                    hwnd: "fixture".into(),
                    pid: std::process::id(),
                    title: "离线流程".into(),
                    executable: "Fixture".into(),
                    width: 1920,
                    height: 1080,
                    keyboard_permission: None,
                });
            }
            Err(error) => {
                let mut state = state.lock().expect("运行状态锁失效");
                state.sdk.ready = false;
                state.sdk.message = format!("{error:#}");
            }
        }
    }
    let mut paused_mail: Option<MailWork> = None;
    for command in receiver {
        match command {
            Work::Mail(work) => {
                paused_mail = run_mail_work(work, &paths, &state, &mut native);
            }
            Work::MailControl(id, resume) => {
                if paused_mail.as_ref().is_some_and(|work| work.run.id == id) {
                    let mut work = paused_mail.take().expect("已有暂停任务");
                    if resume
                        && work.run.interrupt_requested()
                            != Some(od_core::pipeline::Interrupt::Stop)
                    {
                        if let Err(error) = work.run.resume()
                            && work.run.interrupt_requested()
                                != Some(od_core::pipeline::Interrupt::Stop)
                        {
                            work.run.state = MailState::Failed;
                            work.run.message = format!("{error:#}");
                        }
                    } else {
                        work.run.control().stop();
                    }
                    paused_mail = run_mail_work(work, &paths, &state, &mut native);
                }
            }
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
                pipeline_timeout,
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
                        let release_ms = started.elapsed().as_millis() as u64;
                        if let Some(timeout_ms) = pipeline_timeout {
                            return Ok(InputOutcome::Pipeline(od_core::InputPipelineResult {
                                config: od_core::InputPipelineConfig {
                                    input: config,
                                    timeout_ms,
                                },
                                pipeline: od_core::PipelineProbeResult {
                                    sdk_version: maa_adapter::SDK_VERSION.into(),
                                    entry: maa_adapter::INPUT_PIPELINE_ENTRY.into(),
                                    end: od_core::PipelineEnd::Cancelled,
                                    native_status: "NotStarted".into(),
                                    stop_status: None,
                                    enqueue_us: 0,
                                    elapsed_ms: release_ms,
                                    release_ms,
                                    stop_ms: Some(release_ms),
                                    released: true,
                                },
                            }));
                        }
                        return Ok(InputOutcome::Direct(od_core::InputProbeResult {
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
                        }));
                    }
                    let controller = native
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("控制器未连接"))?;
                    if let Some(timeout_ms) = pipeline_timeout {
                        controller
                            .input_pipeline(
                                od_core::InputPipelineConfig {
                                    input: config,
                                    timeout_ms,
                                },
                                cancel,
                            )
                            .map(InputOutcome::Pipeline)
                    } else {
                        controller
                            .input_probe(config, cancel)
                            .map(InputOutcome::Direct)
                    }
                })();
                let mut state = state.lock().expect("运行状态锁失效");
                let record = match result {
                    Ok(InputOutcome::Direct(mut result)) => {
                        if state
                            .slot
                            .current()
                            .is_some_and(|run| run.state == RunState::Stopping)
                        {
                            result.stopped = true;
                        }
                        state.slot.finish_input_probe(id, result)
                    }
                    Ok(InputOutcome::Pipeline(mut result)) => {
                        if result.pipeline.end == od_core::PipelineEnd::Succeeded
                            && state
                                .slot
                                .current()
                                .is_some_and(|run| run.state == RunState::Stopping)
                        {
                            result.pipeline.end = od_core::PipelineEnd::Cancelled;
                        }
                        state.slot.finish_input_pipeline(id, result)
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
                if let Some(work) = paused_mail.take() {
                    work.run.control().stop();
                    let _ = run_mail_work(work, &paths, &state, &mut native);
                }
                native.take();
                let _ = reply.send(());
                return;
            }
        }
    }
    if let Some(work) = paused_mail.take() {
        work.run.control().stop();
        let _ = run_mail_work(work, &paths, &state, &mut native);
    }
}

fn drain_mail_events(work: &MailWork, state: &mut State) {
    if let Some(events) = &work.events {
        for event in events.receiver.try_iter() {
            state.event(
                Some(work.run.id),
                "debug",
                format!("{} {}", event.message, event.node.as_deref().unwrap_or("")),
            );
        }
    }
}

fn run_mail_work(
    mut work: MailWork,
    paths: &EnginePaths,
    shared: &Arc<Mutex<State>>,
    native: &mut Option<NativeSession>,
) -> Option<MailWork> {
    let result = (|| -> anyhow::Result<()> {
        if work.run.state.is_terminal() {
            return Ok(());
        }
        if !work.prepared && work.fixture {
            work.events = Some(
                native
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("离线控制器未准备"))?
                    .pipeline_events(EVENT_CAPACITY)?,
            );
            work.prepared = true;
        }
        if !work.prepared {
            if work.run.interrupt_requested().is_some() {
                if let Some(controller) = native.as_mut() {
                    controller.release_inputs()?;
                }
                work.run.acknowledge_idle();
                return Ok(());
            }
            let bundle =
                maa_adapter::validate_mail_bundle(&paths.resources.join("generated/mail"))?;
            if let Some(controller) = native.as_mut() {
                controller.release_inputs()?;
            }
            native.take();
            let current = od_platform::parse_game_window(&work.window.hwnd)?;
            anyhow::ensure!(
                current.pid == work.window.pid && current.width == 1920 && current.height == 1080,
                "游戏窗口身份或尺寸已变化，请重连"
            );
            let mut controller = NativeSession::for_game(
                &current,
                &paths.resources,
                work.capture,
                InputMode::Foreground,
                InferenceDevice::Cpu,
            )?;
            controller.load_mail_bundle(&bundle)?;
            work.events = Some(controller.pipeline_events(EVENT_CAPACITY)?);
            *native = Some(controller);
            work.prepared = true;
        }
        let controller = native
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("邮件控制器未连接"))?;
        let events = work.events.as_ref();
        let id = work.run.id;
        let publish = |progress| {
            let mut state = shared.lock().expect("运行状态锁失效");
            if let Some(events) = events {
                for event in events.receiver.try_iter() {
                    state.event(
                        Some(id),
                        "debug",
                        format!("{} {}", event.message, event.node.as_deref().unwrap_or("")),
                    );
                }
            }
            let _ = state
                .slot
                .update_mail(id, progress, "邮件流程运行中".into());
        };
        if work.fixture {
            work.run.execute_fixture(controller, publish)
        } else {
            work.run.execute_game(controller, publish)
        }
    })();
    if let Err(error) = result {
        work.run.state = MailState::Failed;
        work.run.message = format!("邮件流程失败：{error:#}");
        work.run.inputs_released = native
            .as_mut()
            .is_none_or(|controller| controller.release_inputs().is_ok());
    }
    let mut state = shared.lock().expect("运行状态锁失效");
    drain_mail_events(&work, &mut state);
    if let Some(events) = &work.events {
        let dropped = events.dropped();
        state.dropped += dropped.saturating_sub(work.dropped);
        work.dropped = dropped;
    }
    if let Ok(record) =
        state
            .slot
            .update_mail(work.run.id, work.run.progress(), work.run.message.clone())
    {
        save_run(paths, &mut state, &record);
        state.event(
            Some(record.id),
            if record.state == RunState::Failed {
                "error"
            } else {
                "info"
            },
            record.message,
        );
    }
    if work.run.state == MailState::Paused {
        Some(work)
    } else {
        state.mail_control = None;
        None
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
    let saved = RunStore::new(&paths.data).and_then(|store| store.save(record));
    if let Err(error) = saved {
        state.failed_save = Some(record.clone());
        state.record_error = Some(format!("运行 {} 的结果保存失败：{error}", record.id));
        state.event(
            Some(record.id),
            "error",
            format!("运行结果保存失败：{error}"),
        );
    } else if state
        .failed_save
        .as_ref()
        .is_some_and(|failed| failed.id == record.id)
    {
        state.failed_save = None;
        state.record_error = None;
    }
}
