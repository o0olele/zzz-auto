use crate::coffee::CoffeeRun;
#[path = "group_runtime.rs"]
mod group_runtime;
use crate::mail::{MailRun, MailState};
use crate::map_transport::{MapRun, MapState};
use chrono::Utc;
use group_runtime::{GroupWork, run_group_work};
use maa_adapter::{CaptureMethod, InferenceDevice, InputMode, NativeSession};
use od_core::coffee_run::{CoffeeConfig, CoffeeState};
use od_core::config::{ConfigError, ConfigStore};
use od_core::daily::{GameDay, RunPolicy};
use od_core::map_transport::MapConfig;
use od_core::{
    InputProbeConfig, RunEvent, RunRecord, RunSlot, RunState, SdkStatus, Snapshot, WindowInfo,
};
use od_core::{mail::MailConfig, pipeline::PipelineRequest};
use od_core::{
    run_history::RunStore,
    run_plan::{GroupPlan, GroupRunOptions, plan_group_with_options_at, plan_standalone_at},
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

enum AccountPlanSelection {
    Group {
        app_ids: Option<Vec<String>>,
        rerun_completed: bool,
    },
    Standalone(String),
}

impl AccountPlanSelection {
    fn plan(
        &self,
        config: &od_core::config::AccountConfig,
        instance_id: u32,
        group_id: &str,
        now: chrono::DateTime<Utc>,
        completed: Option<&od_core::run_history::DailyCompletions>,
    ) -> Result<GroupPlan, String> {
        match self {
            Self::Group {
                app_ids,
                rerun_completed,
            } => plan_group_with_options_at(
                config,
                instance_id,
                group_id,
                now,
                completed,
                GroupRunOptions {
                    app_ids: app_ids.as_deref(),
                    rerun_completed: *rerun_completed,
                },
            ),
            Self::Standalone(app_id) => {
                plan_standalone_at(config, instance_id, group_id, app_id, now, completed)
            }
        }
    }
    fn needs_daily_proof(&self) -> bool {
        matches!(
            self,
            Self::Standalone(_)
                | Self::Group {
                    rerun_completed: true,
                    ..
                }
        )
    }
}

struct AccountRunRequest {
    combat_revision: Option<String>,
    instance_id: u32,
    group_id: String,
    revision: u32,
    game_day: String,
    selection: AccountPlanSelection,
    allow_input: bool,
    confirm_account: bool,
}

struct State {
    combat_recognition: Option<Arc<Mutex<Option<maa_adapter::RawRecognitionControl>>>>,
    record_error: Option<String>,
    failed_save: Option<RunRecord>,
    sdk: SdkStatus,
    window: Option<WindowInfo>,
    capture_method: Option<CaptureMethod>,
    slot: RunSlot,
    native_pending: bool,
    cancel: Option<Arc<AtomicBool>>,
    pipeline_control: Option<PipelineRequest>,
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
    Combat(Box<CombatWork>),
    Group(Box<GroupWork>),
    Mail(MailWork),
    Map(MapWork),
    Coffee(CoffeeWork),
    PipelineControl(Uuid, bool),
    RetryChargeSave(Uuid),
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

struct CombatWork {
    run: crate::combat::CombatRun,
    recognizer: maa_adapter::combat_vision::CombatRecognizer,
    window: WindowInfo,
    capture: CaptureMethod,
    prepared: bool,
    fixture: bool,
}

// 即使请求方断线或取消等待，也要等文件工作结束后再开放任务。
struct ConfigWorkGuard {
    state: Arc<Mutex<State>>,
    _owner: Arc<std::fs::File>,
}

impl Drop for ConfigWorkGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.native_pending = false;
        }
    }
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

struct MapWork {
    fixture: bool,
    run: Box<MapRun>,
    window: WindowInfo,
    capture: CaptureMethod,
    prepared: bool,
    events: Option<maa_adapter::PipelineEvents>,
    dropped: u64,
}

struct CoffeeWork {
    fixture: bool,
    run: Box<CoffeeRun>,
    window: WindowInfo,
    capture: CaptureMethod,
    prepared: bool,
    events: Option<maa_adapter::PipelineEvents>,
    dropped: u64,
}

struct PreparedGroup {
    plan: GroupPlan,
    previous_run: Option<Uuid>,
}

enum InputOutcome {
    Direct(od_core::InputProbeResult),
    Pipeline(od_core::InputPipelineResult),
}

#[derive(Clone)]
pub struct Engine {
    execution_root: Arc<PathBuf>,
    combat_root: Arc<PathBuf>,
    combat_data: Arc<PathBuf>,
    game_launcher: Arc<Mutex<Box<dyn od_platform::game_launch::GameLauncher>>>,
    resources: Arc<crate::model_resources::ModelResourceService>,
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
    // 独立测试仓注入执行器，不启动用户游戏；CLI / IPC 不开放此入口。
    pub fn with_game_launcher(
        paths: EnginePaths,
        launcher: Box<dyn od_platform::game_launch::GameLauncher>,
    ) -> anyhow::Result<Self> {
        let mut engine = Self::new(paths)?;
        engine.game_launcher = Arc::new(Mutex::new(launcher));
        Ok(engine)
    }
    pub fn new(paths: EnginePaths) -> anyhow::Result<Self> {
        Self::with_resources(paths, false)
    }

    /// 显式资源目录优先于保存的选择，不修改用户已有的资源选择。
    pub fn with_resources(paths: EnginePaths, explicit_resources: bool) -> anyhow::Result<Self> {
        Self::with_factory(paths, None, explicit_resources)
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
            true,
        )
    }

    pub fn for_map_fixture(
        paths: EnginePaths,
        callback: impl maa_adapter::CustomControllerCallback + 'static,
    ) -> anyhow::Result<Self> {
        Self::for_mail_fixture(paths, callback)
    }
    /// 独立测试仓通过公开试运行命令验证体力入口；只创建自定义控制器。
    pub fn for_charge_fixture(
        paths: EnginePaths,
        callback: impl maa_adapter::CustomControllerCallback + 'static,
    ) -> anyhow::Result<Self> {
        let resources = paths.resources.join("generated/compendium");
        Self::with_factory(
            paths,
            Some(Box::new(move || {
                NativeSession::for_custom_controller(callback, &resources, InputMode::Foreground)
            })),
            true,
        )
    }

    /// 仅供独立测试仓；公开命令仍走相同工作线程，控制器不会连接游戏。
    pub fn for_lost_void_fixture(
        paths: EnginePaths,
        callback: impl maa_adapter::CustomControllerCallback + 'static,
    ) -> anyhow::Result<Self> {
        let resources = paths.resources.join("generated/lost-void-scene");
        Self::with_factory(
            paths,
            Some(Box::new(move || {
                NativeSession::for_custom_controller(callback, &resources, InputMode::Foreground)
            })),
            true,
        )
    }

    pub fn for_combat_fixture(
        paths: EnginePaths,
        callback: impl maa_adapter::CustomControllerCallback + 'static,
        mode: InputMode,
    ) -> anyhow::Result<Self> {
        let resources = paths.resources.join("generated/combat");
        Self::with_factory(
            paths,
            Some(Box::new(move || {
                NativeSession::for_custom_controller(callback, &resources, mode)
            })),
            true,
        )
    }

    pub fn for_coffee_fixture(
        paths: EnginePaths,
        callback: impl maa_adapter::CustomControllerCallback + 'static,
    ) -> anyhow::Result<Self> {
        let resources = paths.resources.clone();
        Self::with_factory(
            paths,
            Some(Box::new(move || {
                let map = resources.join("generated/map-preview");
                let mut session =
                    NativeSession::for_custom_controller(callback, &map, InputMode::Foreground)?;
                let mail = resources.join("generated/mail");
                if mail.exists() {
                    session.load_mail_bundle(&mail)?;
                }
                session.load_map_bundle(&map)?;
                session.load_coffee_run_bundle(&resources.join("generated/coffee-run"))?;
                Ok(session)
            })),
            true,
        )
    }

    fn with_factory(
        mut paths: EnginePaths,
        factory: Option<Box<dyn FnOnce() -> anyhow::Result<NativeSession> + Send>>,
        explicit_resources: bool,
    ) -> anyhow::Result<Self> {
        let fixture = factory.is_some();
        let resources = Arc::new(crate::model_resources::ModelResourceService::new(
            paths.data.join("model-cache"),
        )?);
        let config = ConfigStore::new(&paths.data)?;
        let config_owner = Arc::new(config.acquire_owner()?);
        let resource_id = if !fixture && !explicit_resources {
            let (directory, id) =
                crate::runtime_resources::select_for_engine(&paths.data, &paths.resources)?;
            paths.resources = directory;
            id
        } else {
            None
        };
        config.read_for_startup()?;
        let charge_recovery = config.recover_charge_runs()?;
        let history = RunStore::new(&paths.data)?;
        std::fs::create_dir_all(paths.data.join("runs"))?;
        let state = Arc::new(Mutex::new(State {
            combat_recognition: None,
            record_error: None,
            failed_save: None,
            sdk: SdkStatus {
                ready: false,
                version: None,
                expected_version: maa_adapter::SDK_VERSION.into(),
                binding_version: maa_adapter::BINDING_VERSION.into(),
                directory: paths.sdk.display().to_string(),
                message: "正在校验运行资源并加载 Maa SDK".into(),
            },
            window: None,
            capture_method: None,
            slot: RunSlot::default(),
            native_pending: false,
            cancel: None,
            pipeline_control: None,
            events: VecDeque::new(),
            sequence: 0,
            dropped: 0,
            callback_dropped: Arc::new(AtomicU64::new(0)),
        }));
        if explicit_resources && !fixture {
            state.lock().expect("运行状态锁失效").event(
                None,
                "info",
                format!("使用显式运行资源目录：{}", paths.resources.display()),
            );
        }
        if charge_recovery.recovered > 0 || charge_recovery.stale > 0 {
            state.lock().expect("运行状态锁失效").event(None, "info", format!(
                "体力运行历史检查：补交 {} 个运行的已确认次数；{} 个旧配置分支仅保留历史，不重放游戏输入。",
                charge_recovery.recovered, charge_recovery.stale,
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(8);
        let combat_root = Arc::new(paths.resources.join("generated/combat"));
        let combat_data = Arc::new(paths.data.clone());
        let execution_root = Arc::new(paths.resources.clone());
        let worker_state = state.clone();
        let worker_owner = config_owner.clone();
        std::thread::Builder::new()
            .name("maa-worker".into())
            .spawn(move || {
                let _owner = worker_owner;
                worker(paths, worker_state, receiver, factory, resource_id);
            })?;
        Ok(Self {
            execution_root,
            combat_root,
            combat_data,
            game_launcher: Arc::new(Mutex::new(Box::new(
                od_platform::game_launch::NativeGameLauncher::default(),
            ))),
            resources,
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
        self.resources.close();
        let mut state = self.state.lock().expect("运行状态锁失效");
        if let Some(cancel) = &state.cancel {
            cancel.store(true, Ordering::Release);
        }
        if let Some(control) = &state.pipeline_control {
            control.stop();
        }
        if let Some(raw) = &state.combat_recognition
            && let Some(control) = raw.lock().expect("战斗推理取消锁失效").as_ref()
        {
            let _ = control.cancel();
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
            if state.pipeline_control.is_some() && !was_stopping {
                let _ = self.sender.try_send(Work::PipelineControl(id, false));
            }
        }
    }

    pub async fn shutdown(&self) -> Result<(), ApiError> {
        self.cancel_active();
        let (tx, rx) = oneshot::channel();
        let native = match self.sender.try_send(Work::Shutdown(tx)) {
            Ok(()) => rx
                .await
                .map_err(|_| ApiError::new("worker_unavailable", "工作线程已异常退出")),
            Err(_) => Err(ApiError::new(
                "worker_unavailable",
                "工作线程无法接收退出请求",
            )),
        };
        self.resources.shutdown().await;
        native
    }

    // 命令含完整配置 / 计划，各业务分支的 future 会随迁移扩大。
    // 在公共边界装箱，避免 CLI、管道或桌面调用方把它嵌进自己的栈帧。
    pub fn execute(
        &self,
        command: Command,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Payload, ApiError>> + Send + '_>>
    {
        Box::pin(self.execute_inner(command))
    }

    async fn execute_inner(&self, command: Command) -> Result<Payload, ApiError> {
        match command {
            Command::PreviewChargePlan {
                instance_id,
                group_id,
                revision,
                resources,
                double_reward_left,
            } => {
                let config = self.config.read().map_err(config_error)?;
                od_core::charge_plan::preview_group(
                    &config,
                    instance_id,
                    &group_id,
                    revision,
                    Utc::now(),
                    resources,
                    double_reward_left,
                )
                .map(Payload::ChargePlanPreview)
                .map_err(|e| ApiError::new("invalid_charge_plan", e))
            }
            Command::LaunchGame {
                instance_id,
                revision,
                confirm,
            } => self.launch_game(instance_id, revision, confirm).await,
            Command::ModelResources => Ok(Payload::ModelResources(self.resources.snapshot())),
            Command::StartModelResources {
                kind,
                models,
                offline,
            } => {
                let proxy = self
                    .config
                    .read()
                    .map_err(config_error)?
                    .general
                    .download_proxy;
                self.resources
                    .start_with_proxy(kind, models, offline, proxy)
                    .map(Payload::ModelResources)
            }
            Command::CancelModelResources { job_id } => {
                self.resources.cancel(job_id).map(Payload::ModelResources)
            }
            Command::RetryRunSave => {
                let mut state = self.state.lock().expect("运行状态锁失效");
                if let Some(record) = &state.failed_save {
                    self.history
                        .save(record)
                        .map_err(|error| ApiError::new("history_error", error.to_string()))?;
                    let id = record.id;
                    let record = record.clone();
                    let _ = state.slot.confirm_group_saved(&record);
                    state.failed_save = None;
                    state.record_error = None;
                    state.event(
                        Some(id),
                        "info",
                        "运行记录已重新保存，未执行游戏动作".into(),
                    );
                }
                if let Some(id) = state
                    .slot
                    .current()
                    .filter(|r| {
                        r.charge.as_ref().is_some_and(|p| {
                            p.outcome
                                == Some(od_core::charge_application::ChargeAppOutcome::AwaitingSave)
                        })
                    })
                    .map(|r| r.id)
                {
                    if state.native_pending {
                        return Err(ApiError::new("busy", "体力结果正在补交"));
                    }
                    self.sender
                        .try_send(Work::RetryChargeSave(id))
                        .map_err(|_| ApiError::new("worker_unavailable", "无法补交体力结果"))?;
                    state.native_pending = true;
                    state.record_error = Some("正在补交原体力结果，不执行游戏动作".into());
                }
                Ok(Payload::Snapshot(Box::new(state.snapshot())))
            }
            Command::PreviewGroup {
                instance_id,
                group_id,
                app_ids,
                rerun_completed,
            } => {
                let config = self.config.read().map_err(config_error)?;
                self.group_plan(
                    config,
                    instance_id,
                    group_id,
                    AccountPlanSelection::Group {
                        app_ids,
                        rerun_completed,
                    },
                )
                .await
                .map(Payload::GroupPlan)
            }
            Command::PreviewStandalone {
                instance_id,
                group_id,
                app_id,
            } => {
                let config = self.config.read().map_err(config_error)?;
                self.group_plan(
                    config,
                    instance_id,
                    group_id,
                    AccountPlanSelection::Standalone(app_id),
                )
                .await
                .map(Payload::GroupPlan)
            }
            Command::QueryRunHistory { instance_id, query } => {
                self.query_history(instance_id, query, None).await
            }
            Command::ExportRunHistory {
                instance_id,
                query,
                content_id,
            } => {
                self.query_history(instance_id, query, Some(content_id))
                    .await
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
                combat_revision,
                instance_id,
                group_id,
                revision,
                game_day,
                app_ids,
                rerun_completed,
                allow_input,
                confirm_account,
            } => {
                self.start_account_run(AccountRunRequest {
                    combat_revision,
                    instance_id,
                    group_id,
                    revision,
                    game_day,
                    selection: AccountPlanSelection::Group {
                        app_ids,
                        rerun_completed,
                    },
                    allow_input,
                    confirm_account,
                })
                .await
            }
            Command::StartStandaloneTrial {
                combat_revision,
                instance_id,
                group_id,
                app_id,
                revision,
                game_day,
                allow_input,
                confirm_account,
            } => {
                self.start_account_run(AccountRunRequest {
                    combat_revision,
                    instance_id,
                    group_id,
                    revision,
                    game_day,
                    selection: AccountPlanSelection::Standalone(app_id),
                    allow_input,
                    confirm_account,
                })
                .await
            }
            Command::AccountConfig => self
                .config
                .read()
                .map(Payload::AccountConfig)
                .map_err(config_error),
            Command::PreviewConfigImport { source } => self.import_config(source, None).await,
            command @ (Command::ConfigBackups
            | Command::PreviewConfigBackup { .. }
            | Command::RestoreConfigBackup { .. }) => self.backup_config(command).await,
            Command::ApplyConfigImport { source, preview_id } => {
                self.import_config(source, Some(preview_id)).await
            }
            Command::UpdateAccountConfig { revision, change } => {
                self.update_config(revision, change).await
            }
            Command::CombatScripts => {
                let root = self.combat_root.clone();
                let data = self.combat_data.clone();
                tokio::task::spawn_blocking(move || {
                    let bundle = maa_adapter::combat_vision::CombatBundle::load(&root)
                        .map_err(native_error)?;
                    Ok(Payload::CombatScripts(
                        crate::combat::script_catalog(&bundle, &data).map_err(native_error)?,
                    ))
                })
                .await
                .map_err(|_| ApiError::new("combat_resources_error", "战斗资源检查线程异常"))?
            }
            Command::StartCombat {
                instance_id,
                revision,
                script_revision,
                allow_input,
                confirm_account,
            } => {
                self.start_combat(
                    instance_id,
                    revision,
                    script_revision,
                    allow_input,
                    confirm_account,
                )
                .await
            }
            Command::PreviewCombatScriptImport { source } => {
                self.combat_script_import(source, None).await
            }
            Command::ApplyCombatScriptImport { source, preview_id } => {
                self.combat_script_import(source, Some(preview_id)).await
            }
            Command::StartMail {
                config,
                allow_input,
            } => self.start_mail(Some(config), allow_input, None),
            Command::StartMap {
                config,
                allow_input,
            } => self.start_map(config, allow_input),
            Command::StartCoffee {
                config,
                allow_input,
            } => self.start_coffee(config, allow_input, false),
            Command::StartCoffeeFromWorld {
                config,
                allow_input,
            } => self.start_coffee(config, allow_input, true),
            Command::Pause { run_id } => {
                let mut state = self.state.lock().expect("运行状态锁失效");
                let record = state
                    .slot
                    .request_pause(run_id)
                    .map_err(|error| ApiError::new("invalid_run_state", error))?;
                if let Some(control) = &state.pipeline_control {
                    control.pause();
                }
                if let Some(raw) = &state.combat_recognition
                    && let Some(control) = raw.lock().expect("战斗推理取消锁失效").as_ref()
                {
                    let _ = control.cancel();
                }
                Ok(Payload::Run(record))
            }
            Command::Resume { run_id } => {
                let mut state = self.state.lock().expect("运行状态锁失效");
                if state.record_error.is_some() {
                    return Err(ApiError::new(
                        "history_error",
                        "请先重试保存运行记录，再继续应用组",
                    ));
                }
                let current = state
                    .slot
                    .current()
                    .filter(|run| {
                        run.id == run_id
                            && run.state == RunState::Paused
                            && (run.mail.is_some()
                                || run.map.is_some()
                                || run.coffee.is_some()
                                || run.combat.is_some()
                                || run.charge.is_some()
                                || run.lost_void.is_some()
                                || !run.daily.is_empty())
                    })
                    .ok_or_else(|| {
                        ApiError::new("invalid_run_state", "只有已确认暂停的流程可以继续")
                    })?;
                if self
                    .sender
                    .try_send(Work::PipelineControl(current.id, true))
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
                if let Some(control) = &state.pipeline_control {
                    control.stop();
                    if !was_stopping {
                        let _ = self.sender.try_send(Work::PipelineControl(run_id, false));
                    }
                } else if record.group.is_some() && !was_stopping {
                    let _ = self.sender.try_send(Work::PipelineControl(run_id, false));
                }
                if let Some(raw) = &state.combat_recognition
                    && let Some(control) = raw.lock().expect("战斗推理取消锁失效").as_ref()
                {
                    let _ = control.cancel();
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

    async fn query_history(
        &self,
        instance_id: u32,
        query: od_core::run_history::HistoryQuery,
        export_content: Option<String>,
    ) -> Result<Payload, ApiError> {
        query
            .validate()
            .map_err(|e| ApiError::new("history_error", e.to_string()))?;
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
                            .is_some_and(|c| c.instance_id == instance_id)
                })
                .map(|run| run.id)
        };
        let history = self.history.clone();
        tokio::task::spawn_blocking(move || {
            if let Some(content) = export_content {
                history
                    .export_page(instance_id, live, query, &content)
                    .map(Payload::HistoryExport)
            } else {
                history
                    .query(instance_id, live, query)
                    .map(Payload::HistoryPage)
            }
        })
        .await
        .map_err(|_| ApiError::new("history_error", "历史查询线程异常退出"))?
        .map_err(|error| ApiError::new("history_error", error.to_string()))
    }

    async fn start_account_run(&self, request: AccountRunRequest) -> Result<Payload, ApiError> {
        let AccountRunRequest {
            combat_revision,
            instance_id,
            group_id,
            revision,
            game_day,
            selection,
            allow_input,
            confirm_account,
        } = request;
        if !confirm_account {
            return Err(ApiError::new(
                "account_not_confirmed",
                "请先确认游戏登录账号与所选账号一致",
            ));
        }
        if !allow_input {
            return Err(ApiError::new("input_not_confirmed", "请先确认允许真实输入"));
        }
        let previous_run = {
            let state = self.state.lock().expect("运行状态锁失效");
            if state.record_error.is_some() {
                return Err(ApiError::new(
                    "history_error",
                    "请先修复目录并重试保存运行记录",
                ));
            }
            if state.slot.busy() || state.native_pending {
                return Err(ApiError::new("busy", "已有任务或原生请求尚未完成"));
            }
            state.slot.current().map(|run| run.id)
        };
        let saved = self.config.read().map_err(config_error)?;
        if saved.revision != revision {
            return Err(config_error(ConfigError::Conflict(Box::new(saved))));
        }
        let plan = self
            .group_plan(saved, instance_id, group_id, selection)
            .await?;
        if plan.game_day.date != game_day {
            return Err(ApiError::new(
                "plan_expired",
                "预览的游戏日已过期，请重新预览并确认",
            ));
        }
        if plan
            .app_combat
            .as_ref()
            .or(plan.world_patrol_combat.as_ref())
            .or(plan.charge_combat.as_ref())
            .or(plan.lost_void_combat.as_ref())
            .map(|p| &p.script_revision)
            != combat_revision.as_ref()
        {
            return Err(ApiError::new(
                "combat_scripts_changed",
                "情报板战斗脚本或资源与预览不符，请重新预览并确认",
            ));
        }
        self.start_mail(None, true, Some(PreparedGroup { plan, previous_run }))
    }

    async fn group_plan(
        &self,
        config: od_core::config::AccountConfig,
        instance_id: u32,
        group_id: String,
        selection: AccountPlanSelection,
    ) -> Result<GroupPlan, ApiError> {
        let now = Utc::now();
        let plan = selection
            .plan(&config, instance_id, &group_id, now, None)
            .map_err(|error| ApiError::new("invalid_group", error))?;
        if plan.run_policy == RunPolicy::Always
            && !selection.needs_daily_proof()
            && !plan.items.iter().any(|item| {
                item.enabled
                    && matches!(
                        item.app_id.as_str(),
                        "redemption_code"
                            | "intel_board"
                            | "world_patrol"
                            | "charge_plan"
                            | "lost_void"
                    )
            })
        {
            return Ok(plan);
        }
        let history = self.history.clone();
        let combat_root = self.combat_root.clone();
        let combat_data = self.combat_data.clone();
        let execution_root = self.execution_root.clone();
        tokio::task::spawn_blocking(move || {
            let completed = history
                .daily_completions(instance_id, plan.game_day, now)
                .map_err(|error| ApiError::new("history_error", error.to_string()))?;
            let mut plan = selection
                .plan(&config, instance_id, &group_id, now, Some(&completed))
                .map_err(|error| ApiError::new("invalid_group", error))?;
            if plan.items.iter().any(|i| {
                i.app_id == "charge_plan" && i.decision == od_core::run_plan::PlanDecision::Run
            }) {
                plan.charge_combat =
                    crate::charge_entry::combat_preview(&config, &plan, &combat_root, &combat_data)
                        .map_err(native_error)?;
            }
            if plan
                .items
                .iter()
                .any(|item| item.enabled && item.app_id == "redemption_code")
            {
                let results = history
                    .redemption_history(instance_id)
                    .map_err(|error| ApiError::new("history_error", error.to_string()))?;
                od_core::redemption::apply_history(&mut plan, results);
            }
            if plan
                .items
                .iter()
                .any(|item| item.enabled && item.app_id == "intel_board")
            {
                let results = history
                    .intel_history(instance_id, &plan.game_day, now)
                    .map_err(|error| ApiError::new("history_error", error.to_string()))?;
                od_core::intel_board::apply_history(&mut plan, results)
                    .map_err(|error| ApiError::new("history_error", error))?;
                if plan.items.iter().any(|i| {
                    i.app_id == "intel_board" && i.decision == od_core::run_plan::PlanDecision::Run
                }) {
                    let instance = config
                        .instances
                        .iter()
                        .find(|i| i.id == instance_id)
                        .ok_or_else(|| ApiError::new("invalid_account", "情报板账号缺失"))?;
                    let settings =
                        od_core::app_combat::intel_settings(instance, &plan.intel_board_config)
                            .map_err(|e| ApiError::new("invalid_combat", e))?;
                    let bundle = maa_adapter::combat_vision::CombatBundle::load(&combat_root)
                        .map_err(native_error)?;
                    let revision = crate::combat::script_catalog(&bundle, &combat_data)
                        .map_err(native_error)?
                        .revision;
                    let frozen = od_core::app_combat::AppCombatPlan {
                        config: settings,
                        script_revision: revision,
                    };
                    crate::intel_board::PreparedIntel::from_bundle(
                        frozen.clone(),
                        bundle,
                        &combat_data,
                    )
                    .map_err(native_error)?;
                    plan.warnings.push(format!(
                        "情报板将执行代行战斗，采用脚本 {}；当前无委托不代表周目标完成。",
                        frozen.config.script
                    ));
                    plan.app_combat = Some(frozen);
                }
            }
            if plan
                .items
                .iter()
                .any(|i| i.enabled && i.app_id == "world_patrol")
                && !plan.context.rerun_completed
                && let Some((checkpoint, active)) = history
                    .patrol_checkpoint(
                        instance_id,
                        &plan.context.group_id,
                        &plan.game_day,
                        &plan.world_patrol_config,
                        now,
                    )
                    .map_err(|e| ApiError::new("history_error", e.to_string()))?
            {
                if checkpoint.completed()
                    && let Some(item) = plan.items.iter_mut().find(|i| i.app_id == "world_patrol")
                {
                    item.decision = od_core::run_plan::PlanDecision::NoWork;
                }
                plan.world_patrol_checkpoint = Some(checkpoint);
                plan.world_patrol_active_ms = active;
                plan.executable = plan.blockers.is_empty()
                    && plan
                        .items
                        .iter()
                        .any(|i| i.decision == od_core::run_plan::PlanDecision::Run);
                plan.all_done = plan.blockers.is_empty() && !plan.executable;
            }
            if plan.items.iter().any(|i| {
                i.enabled
                    && i.app_id == "world_patrol"
                    && i.decision == od_core::run_plan::PlanDecision::Run
            }) {
                let instance = config
                    .instances
                    .iter()
                    .find(|i| i.id == instance_id)
                    .ok_or_else(|| ApiError::new("invalid_account", "锄大地账号缺失"))?;
                let mut settings = instance.combat.clone();
                settings.script = plan.world_patrol_config.auto_battle.clone();
                settings.dodge_only = false;
                settings.start_delay_ms = 0;
                let bundle = maa_adapter::combat_vision::CombatBundle::load(&combat_root)
                    .map_err(native_error)?;
                let revision = crate::combat::script_catalog(&bundle, &combat_data)
                    .map_err(native_error)?
                    .revision;
                let frozen = od_core::app_combat::AppCombatPlan {
                    config: settings,
                    script_revision: revision,
                };
                crate::intel_board::PreparedIntel::from_bundle(
                    frozen.clone(),
                    bundle,
                    &combat_data,
                )
                .map_err(native_error)?;
                let routes = od_core::world_patrol::catalog()
                    .and_then(|c| c.selected(&plan.world_patrol_config))
                    .map_err(|e| ApiError::new("invalid_patrol", e))?;
                plan.warnings.push(format!(
                    "锄大地：{} 条路线，{} 轮，轮次起点间隔 {} 秒；战斗脚本 {}。",
                    routes.len(),
                    plan.world_patrol_config.daily_loop_count,
                    plan.world_patrol_config.loop_interval_seconds,
                    frozen.config.script
                ));
                plan.world_patrol_combat = Some(frozen);
            }
            crate::lost_void_run_plan::preview(&mut plan, &config, &execution_root, &combat_data)
                .map_err(native_error)?;
            Ok(plan)
        })
        .await
        .map_err(|_| ApiError::new("history_error", "每日记录查询线程异常退出"))?
    }

    fn start_mail(
        &self,
        config: Option<MailConfig>,
        allow_input: bool,
        group: Option<PreparedGroup>,
    ) -> Result<Payload, ApiError> {
        if !allow_input {
            return Err(ApiError::new(
                "input_not_confirmed",
                "邮件流程会操作游戏，请先确认允许真实输入",
            ));
        }
        let mut state = self.state.lock().expect("运行状态锁失效");
        if state.record_error.is_some() {
            return Err(ApiError::new(
                "history_error",
                "请先修复目录并重试保存运行记录",
            ));
        }
        let (plan, config): (Option<GroupPlan>, MailConfig) =
            if let Some(PreparedGroup { plan, previous_run }) = group {
                if state.slot.busy() || state.native_pending {
                    return Err(ApiError::new("busy", "已有任务或原生请求尚未完成"));
                }
                let saved = self.config.read().map_err(config_error)?;
                if saved.revision != plan.context.config_revision {
                    return Err(config_error(ConfigError::Conflict(Box::new(saved))));
                }
                let day = GameDay::at(plan.game_day.region, Utc::now())
                    .map_err(|error| ApiError::new("invalid_group", error))?;
                if day != plan.game_day || state.slot.current().map(|run| run.id) != previous_run {
                    return Err(ApiError::new(
                        "plan_expired",
                        "预检期间游戏日或运行记录已变化，请重新预览",
                    ));
                }
                if plan.all_done {
                    return Err(ApiError::new(
                        "group_already_done",
                        "本次没有待执行应用；请重新预览每日状态及待兑换码",
                    ));
                }
                if !plan.executable {
                    return Err(ApiError::new("group_blocked", plan.blockers.join("；")));
                }
                let config = plan.mail_config;
                (Some(plan), config)
            } else {
                (None, config.expect("诊断入口必须提供邮件参数"))
            };
        config
            .validate()
            .map_err(|error| ApiError::new("invalid_input", error))?;
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
        if let Some(plan) = &plan
            && plan.context.app_ids != ["email"]
        {
            return self.start_group_work(&mut state, plan.clone(), window);
        }
        let record = match plan {
            Some(plan) => state.slot.begin_group_plan(config, &plan),
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
        state
            .slot
            .confirm_group_saved(&record)
            .map_err(|error| ApiError::new("history_error", error))?;
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
            {
                match self.history.save(&failed) {
                    Ok(()) => {
                        let _ = state.slot.confirm_group_saved(&failed);
                    }
                    Err(error) => {
                        state.failed_save = Some(failed);
                        state.record_error =
                            Some(format!("运行 {} 的结果保存失败：{error}", record.id));
                    }
                }
            }
            return Err(ApiError::new(
                "worker_unavailable",
                "工作线程无法接收邮件任务",
            ));
        }
        state.pipeline_control = Some(control);
        state.event(
            Some(record.id),
            "info",
            "开始邮件试运行；只使用前台键鼠，暂停期间继续占用运行槽".into(),
        );
        Ok(Payload::Run(record))
    }

    fn start_map(&self, config: MapConfig, allow_input: bool) -> Result<Payload, ApiError> {
        if !allow_input {
            return Err(ApiError::new(
                "input_not_confirmed",
                "地图传送会操作游戏，请先确认本次路线",
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
        if !state.sdk.ready {
            return Err(ApiError::new("sdk_not_ready", state.sdk.message.clone()));
        }
        let window = state
            .window
            .clone()
            .ok_or_else(|| ApiError::new("not_connected", "请先连接游戏窗口"))?;
        if state.native_pending {
            return Err(ApiError::new("busy", "原生请求尚未完成"));
        }
        let record = state
            .slot
            .begin_map(config)
            .map_err(|error| ApiError::new("busy", error))?;
        let run = MapRun::with_id(record.id, config).map_err(native_error)?;
        let control = run.control();
        let work = MapWork {
            fixture: self.fixture,
            run: Box::new(run),
            window,
            capture: state.capture_method.unwrap_or_default(),
            prepared: false,
            events: None,
            dropped: 0,
        };
        if self.sender.try_send(Work::Map(work)).is_err() {
            let mut progress = record.map.clone().expect("地图进度已创建");
            progress.state = MapState::Failed;
            progress.inputs_released = true;
            if let Ok(failed) =
                state
                    .slot
                    .update_map(record.id, progress, "工作线程不可用，未投递输入".into())
                && let Err(error) = self.history.save(&failed)
            {
                state.failed_save = Some(failed);
                state.record_error = Some(format!("地图结果保存失败：{error}"));
            }
            return Err(ApiError::new(
                "worker_unavailable",
                "工作线程无法接收地图任务",
            ));
        }
        state.pipeline_control = Some(control);
        state.event(
            Some(record.id),
            "info",
            "开始地图试运行：澄辉坪 → 汀曼咖啡；请在延时期间切回已打开普通地图的游戏".into(),
        );
        Ok(Payload::Run(record))
    }

    async fn combat_script_import(
        &self,
        source: String,
        preview_id: Option<String>,
    ) -> Result<Payload, ApiError> {
        let pending = self.config_work_guard()?;
        let data = self.combat_data.clone();
        tokio::task::spawn_blocking(move || {
            let _pending = pending;
            let result = if let Some(id) = preview_id {
                od_core::combat_scripts::apply(std::path::Path::new(&source), &data, &id)
            } else {
                od_core::combat_scripts::preview(std::path::Path::new(&source), &data)
            };
            result
                .map(Payload::CombatScriptImport)
                .map_err(native_error)
        })
        .await
        .map_err(|_| ApiError::new("combat_import_failed", "战斗脚本导入线程异常"))?
    }

    async fn start_combat(
        &self,
        instance_id: u32,
        revision: u32,
        script_revision: String,
        allow_input: bool,
        confirm_account: bool,
    ) -> Result<Payload, ApiError> {
        if !allow_input || !confirm_account {
            return Err(ApiError::new(
                "input_not_confirmed",
                "请确认当前账号与战斗真实输入",
            ));
        }
        let pending = self.config_work_guard()?;
        let engine = self.clone();
        tokio::task::spawn_blocking(move || {
            let _pending = pending;
            let config = engine.config.read().map_err(config_error)?;
            if config.revision != revision {
                return Err(config_error(ConfigError::Conflict(Box::new(config))));
            }
            if config.active_instance != instance_id {
                return Err(ApiError::new(
                    "account_changed",
                    "当前账号已变化，请重新读取并确认",
                ));
            }
            let instance = config
                .instances
                .iter()
                .find(|a| a.id == instance_id)
                .ok_or_else(|| ApiError::new("invalid_account", "账号不存在"))?;
            let settings = instance.combat.clone();
            let bundle = maa_adapter::combat_vision::CombatBundle::load(&engine.combat_root)
                .map_err(native_error)?;
            let catalog = crate::combat::script_catalog(&bundle, &engine.combat_data)
                .map_err(native_error)?;
            if catalog.revision != script_revision {
                return Err(ApiError::new(
                    "combat_scripts_changed",
                    "战斗脚本或资源已变化，请重新读取并确认",
                ));
            }
            let (category, script_name) = if settings.dodge_only {
                ("dodge", &settings.dodge_script)
            } else {
                ("auto_battle", &settings.script)
            };
            let script_root = od_core::combat_scripts::select_root(
                &engine.combat_data,
                &bundle.root.join("scripts"),
                category,
                script_name,
            )
            .map_err(native_error)?;
            let script = od_core::conditional::Script::load_entry(
                &script_root,
                category,
                script_name,
                settings.use_merged,
            )
            .map_err(native_error)?;
            bundle.spec.validate_script(&script).map_err(native_error)?;
            settings.validate_script(&script).map_err(native_error)?;
            let app_id = if settings.dodge_only {
                "dodge_assistant"
            } else {
                "auto_battle"
            };
            let context = od_core::run_plan::RunContext {
                instance_id,
                instance_name: instance.name.clone(),
                group_id: "combat".into(),
                group_name: Some("战斗".into()),
                config_revision: revision,
                app_ids: vec![app_id.into()],
                rerun_completed: false,
                standalone: true,
                game_day: None,
            };
            let mut state = engine.state.lock().expect("运行状态锁失效");
            if !state.sdk.ready {
                return Err(ApiError::new("sdk_not_ready", state.sdk.message.clone()));
            }
            let window = state
                .window
                .clone()
                .ok_or_else(|| ApiError::new("not_connected", "请先连接游戏窗口"))?;
            let record = state
                .slot
                .begin_combat(settings.clone(), context)
                .map_err(|e| ApiError::new("busy", e))?;
            let mut progress = record.combat.clone().expect("战斗进度已创建");
            progress.script_revision = catalog.revision;
            let record = state
                .slot
                .update_combat(record.id, progress, "战斗准备中".into())
                .map_err(|e| ApiError::new("invalid_run_state", e))?;
            if let Err(error) = engine.history.save(&record) {
                let mut progress = record.combat.clone().expect("战斗进度已创建");
                progress.state = RunState::Failed;
                progress.inputs_released = true;
                let failed = state
                    .slot
                    .update_combat(record.id, progress, "战斗检查点保存失败，未投递输入".into())
                    .map_err(|e| ApiError::new("invalid_run_state", e))?;
                state.failed_save = Some(failed);
                state.record_error = Some(error.to_string());
                return Err(ApiError::new(
                    "history_error",
                    "战斗检查点保存失败，请先重试保存",
                ));
            }
            let mut run =
                crate::combat::CombatRun::new(record.id, settings, script).map_err(native_error)?;
            run.progress.script_revision = record
                .combat
                .as_ref()
                .expect("战斗进度已创建")
                .script_revision
                .clone();
            let control = run.control();
            let recognizer = maa_adapter::combat_vision::CombatRecognizer::new(bundle);
            state.combat_recognition = Some(recognizer.cancellation_handle());
            let work = CombatWork {
                run,
                recognizer,
                window,
                capture: state.capture_method.unwrap_or_default(),
                prepared: false,
                fixture: engine.fixture,
            };
            if engine
                .sender
                .try_send(Work::Combat(Box::new(work)))
                .is_err()
            {
                let mut progress = record.combat.clone().expect("战斗进度已创建");
                progress.state = RunState::Failed;
                progress.inputs_released = true;
                let failed = state
                    .slot
                    .update_combat(record.id, progress, "工作线程不可用，未投递输入".into())
                    .map_err(|e| ApiError::new("invalid_run_state", e))?;
                if let Err(error) = engine.history.save(&failed) {
                    state.failed_save = Some(failed);
                    state.record_error = Some(error.to_string());
                }
                return Err(ApiError::new("worker_unavailable", "战斗工作线程不可用"));
            }
            state.pipeline_control = Some(control);
            state.event(
                Some(record.id),
                "info",
                "战斗已登记，请在启动延时内切回游戏；F9 暂停 / 继续，F10 停止".into(),
            );
            Ok(Payload::Run(record))
        })
        .await
        .map_err(|_| ApiError::new("combat_start_failed", "战斗准备线程异常"))?
    }

    fn start_coffee(
        &self,
        config: CoffeeConfig,
        allow_input: bool,
        from_world: bool,
    ) -> Result<Payload, ApiError> {
        if !allow_input {
            return Err(ApiError::new(
                "input_not_confirmed",
                "咖啡点单会操作游戏，请先确认本次咖啡",
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
        if !state.sdk.ready {
            return Err(ApiError::new("sdk_not_ready", state.sdk.message.clone()));
        }
        let window = state
            .window
            .clone()
            .ok_or_else(|| ApiError::new("not_connected", "请先连接游戏窗口"))?;
        if state.native_pending {
            return Err(ApiError::new("busy", "原生请求尚未完成"));
        }
        let record = if from_world {
            state.slot.begin_coffee_from_world(config)
        } else {
            state.slot.begin_coffee(config)
        }
        .map_err(|error| ApiError::new("busy", error))?;
        let run = if from_world {
            CoffeeRun::from_world(record.id, config)
        } else {
            CoffeeRun::with_id(record.id, config)
        }
        .map_err(native_error)?;
        let control = run.control();
        let work = CoffeeWork {
            fixture: self.fixture,
            run: Box::new(run),
            window,
            capture: state.capture_method.unwrap_or_default(),
            prepared: false,
            events: None,
            dropped: 0,
        };
        if self.sender.try_send(Work::Coffee(work)).is_err() {
            let mut progress = record.coffee.clone().expect("咖啡进度已创建");
            progress.state = CoffeeState::Failed;
            progress.inputs_released = true;
            if let Ok(failed) =
                state
                    .slot
                    .update_coffee(record.id, progress, "工作线程不可用，未投递输入".into())
                && let Err(error) = self.history.save(&failed)
            {
                state.failed_save = Some(failed);
                state.record_error = Some(format!("咖啡结果保存失败：{error}"));
            }
            return Err(ApiError::new(
                "worker_unavailable",
                "工作线程无法接收咖啡任务",
            ));
        }
        state.pipeline_control = Some(control);
        state.event(
            Some(record.id),
            "info",
            if from_world {
                "开始咖啡自动路线：大世界 → 澄辉坪 → 汀曼咖啡；请在延时内切回游戏"
            } else {
                "开始咖啡试运行：请先手动打开澄辉坪咖啡点单选项，并在延时内切回游戏"
            }
            .into(),
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

    fn config_work_guard(&self) -> Result<ConfigWorkGuard, ApiError> {
        {
            let mut state = self.state.lock().expect("运行状态锁失效");
            if state.slot.busy() || state.native_pending {
                return Err(ApiError::new(
                    "busy",
                    "请先停止任务并等待输入收尾，再操作配置文件",
                ));
            }
            if state.record_error.is_some() {
                return Err(ApiError::new("history_error", "请先重试保存运行记录"));
            }
            if state
                .slot
                .current()
                .is_some_and(|run| !run.inputs_released())
            {
                return Err(ApiError::new(
                    "input_recovery_required",
                    "输入尚未确认释放，请先恢复遗留输入",
                ));
            }
            state.native_pending = true;
        }
        Ok(ConfigWorkGuard {
            state: self.state.clone(),
            _owner: self._config_owner.clone(),
        })
    }

    async fn launch_game(
        &self,
        instance_id: u32,
        revision: u32,
        confirm: bool,
    ) -> Result<Payload, ApiError> {
        if !confirm {
            return Err(ApiError::new(
                "game_launch_not_confirmed",
                "请核对当前账号、保存路径和启动参数，并确认启动游戏",
            ));
        }
        let pending = self.config_work_guard()?;
        let store = self.config.clone();
        let launcher = self.game_launcher.clone();
        tokio::task::spawn_blocking(move || {
            let saved = store.read().map_err(config_error)?;
            if saved.revision != revision {
                return Err(config_error(ConfigError::Conflict(Box::new(saved))));
            }
            if saved.active_instance != instance_id {
                return Err(ApiError::new(
                    "account_changed",
                    "当前账号已变化，请重新读取并确认",
                ));
            }
            let account = saved
                .instances
                .iter()
                .find(|item| item.id == instance_id)
                .expect("配置已验证当前账号");
            account
                .game_launch
                .validate()
                .map_err(|e| ApiError::new("invalid_game_launch", e))?;
            if account.game_launch.executable.is_empty() {
                return Err(ApiError::new(
                    "invalid_game_launch",
                    "请先在账号与应用组中保存游戏路径",
                ));
            }
            let pid = launcher
                .lock()
                .map_err(|_| ApiError::new("game_launch_failed", "启动状态不可用，请重连引擎"))?
                .launch(&account.game_launch)
                .map_err(|error| ApiError::new("game_launch_failed", format!("{error:#}")))?;
            pending.state.lock().expect("运行状态锁失效").event(
                None,
                "info",
                format!(
                    "Windows 已接受账号 {instance_id} 的游戏启动，进程 {pid}；请手动登录并连接游戏"
                ),
            );
            Ok(Payload::GameLaunched(
                od_core::game_launch::GameLaunchResult { instance_id, pid },
            ))
        })
        .await
        .map_err(|_| {
            ApiError::new(
                "game_launch_failed",
                "启动线程异常退出，请先检查游戏是否已打开，勿直接重复启动",
            )
        })?
    }

    async fn update_config(
        &self,
        revision: u32,
        change: od_core::config::ConfigChange,
    ) -> Result<Payload, ApiError> {
        let pending = self.config_work_guard()?;
        let store = self.config.clone();
        tokio::task::spawn_blocking(move || {
            let result = store.update(revision, change).map_err(config_error);
            if let Ok(config) = &result {
                pending.state.lock().expect("运行状态锁失效").event(
                    None,
                    "info",
                    format!("配置已保存，修订 {}", config.revision),
                );
            }
            drop(pending);
            result.map(Payload::AccountConfig)
        })
        .await
        .map_err(|_| {
            ApiError::new(
                "config_error",
                "配置保存线程异常退出，请重新读取配置确认结果",
            )
        })?
    }

    async fn backup_config(&self, command: Command) -> Result<Payload, ApiError> {
        if matches!(
            &command,
            Command::RestoreConfigBackup { confirm: false, .. }
        ) {
            return Err(ApiError::new(
                "config_restore_not_confirmed",
                "请先预览并确认恢复配置",
            ));
        }
        let pending = self.config_work_guard()?;
        let store = self.config.clone();
        tokio::task::spawn_blocking(move || {
            let result = match command {
                Command::ConfigBackups => store.backups().map(Payload::ConfigBackups),
                Command::PreviewConfigBackup { revision, id } => store
                    .preview_backup(revision, &id)
                    .map(Payload::ConfigBackupPreview),
                Command::RestoreConfigBackup {
                    revision,
                    id,
                    preview_id,
                    ..
                } => store
                    .restore_backup(revision, &id, &preview_id)
                    .map(Payload::AccountConfig),
                _ => unreachable!(),
            }
            .map_err(config_error);
            if let Ok(Payload::AccountConfig(config)) = &result {
                pending.state.lock().expect("运行状态锁失效").event(
                    None,
                    "info",
                    format!("配置已从备份恢复，修订 {}", config.revision),
                );
            }
            // 请求方取消等待时，互斥仍由文件工作线程持有至结果落盘。
            drop(pending);
            result
        })
        .await
        .map_err(|_| {
            ApiError::new(
                "config_error",
                "配置备份线程异常退出，请重新读取配置确认结果",
            )
        })?
    }

    async fn import_config(
        &self,
        source: String,
        preview_id: Option<String>,
    ) -> Result<Payload, ApiError> {
        let pending = self.config_work_guard()?;
        let store = self.config.clone();
        tokio::task::spawn_blocking(move || {
            let _pending = pending;
            let source = PathBuf::from(source);
            let result = match preview_id {
                Some(id) => store.apply_import(&source, &id).map(Payload::AccountConfig),
                None => store
                    .preview_import(&source)
                    .map(Payload::ConfigImportPreview),
            };
            result.map_err(|error| ApiError::new("config_import_failed", error.to_string()))
        })
        .await
        .map_err(|_| {
            ApiError::new(
                "config_import_failed",
                "配置导入线程异常退出，请重新读取配置确认结果",
            )
        })?
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
    resource_id: Option<String>,
) {
    {
        let result = (|| -> anyhow::Result<SdkStatus> {
            if let Some(id) = resource_id {
                crate::runtime_resources::validate_directory(&paths.resources, Some(&id))?;
                state.lock().expect("运行状态锁失效").event(
                    None,
                    "info",
                    format!(
                        "运行资源校验通过：{id}，目录：{}",
                        paths.resources.display()
                    ),
                );
            }
            maa_adapter::load_sdk(&paths.sdk, &paths.data.join("maa-log"))
        })();
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
    let mut paused_map: Option<MapWork> = None;
    let mut paused_coffee: Option<CoffeeWork> = None;
    let mut paused_group: Option<Box<GroupWork>> = None;
    // 原生句柄只存在于此线程；GroupWork 投递消息只携带冻结数据。
    let mut charge_worker: Option<crate::charge_worker::ChargeWorker> = None;
    let mut paused_combat: Option<Box<CombatWork>> = None;
    for command in receiver {
        match command {
            Work::Combat(work) => {
                paused_combat = run_combat_work(work, &paths, &state, &mut native)
            }
            Work::Group(work) => {
                paused_group = run_group_work(work, &paths, &state, &mut native, &mut charge_worker)
            }
            Work::RetryChargeSave(id) => {
                if paused_group.as_ref().is_some_and(|work| work.id() == id) {
                    let mut work = paused_group.take().expect("已有待补交的应用组");
                    if work.retry_charge_save() {
                        paused_group =
                            run_group_work(work, &paths, &state, &mut native, &mut charge_worker);
                    } else {
                        paused_group = Some(work);
                    }
                }
                state.lock().expect("运行状态锁失效").native_pending = false;
            }
            Work::Mail(work) => {
                paused_mail = run_mail_work(work, &paths, &state, &mut native);
            }
            Work::Coffee(work) => {
                paused_coffee = run_coffee_work(work, &paths, &state, &mut native);
            }
            Work::Map(work) => {
                paused_map = run_map_work(work, &paths, &state, &mut native);
            }
            Work::PipelineControl(id, resume) => {
                if paused_combat.as_ref().is_some_and(|work| work.run.id == id) {
                    let mut work = paused_combat.take().expect("已有暂停战斗");
                    if resume && !work.run.stopping() {
                        if let Err(error) = work.run.resume() {
                            work.run.progress.state = RunState::Failed;
                            work.run.message = format!("战斗继续失败：{error:#}");
                        }
                    } else {
                        work.run.control().stop();
                    }
                    paused_combat = run_combat_work(work, &paths, &state, &mut native);
                }
                if paused_group.as_ref().is_some_and(|work| work.id() == id) {
                    let mut work = paused_group.take().expect("已有暂停应用组");
                    work.control(resume, &mut charge_worker);
                    paused_group =
                        run_group_work(work, &paths, &state, &mut native, &mut charge_worker);
                }
                if paused_coffee.as_ref().is_some_and(|work| work.run.id == id) {
                    let mut work = paused_coffee.take().expect("已有暂停咖啡任务");
                    if resume
                        && work.run.interrupt_requested()
                            != Some(od_core::pipeline::Interrupt::Stop)
                    {
                        if let Err(error) = work.run.resume()
                            && work.run.interrupt_requested()
                                != Some(od_core::pipeline::Interrupt::Stop)
                        {
                            work.run.progress.state = CoffeeState::Failed;
                            work.run.message = format!("{error:#}");
                        }
                    } else {
                        work.run.control().stop();
                    }
                    paused_coffee = run_coffee_work(work, &paths, &state, &mut native);
                }
                if paused_map.as_ref().is_some_and(|work| work.run.id == id) {
                    let mut work = paused_map.take().expect("已有暂停地图任务");
                    if resume
                        && work.run.interrupt_requested()
                            != Some(od_core::pipeline::Interrupt::Stop)
                    {
                        if let Err(error) = work.run.resume()
                            && work.run.interrupt_requested()
                                != Some(od_core::pipeline::Interrupt::Stop)
                        {
                            work.run.state = MapState::Failed;
                            work.run.message = format!("{error:#}");
                        }
                    } else {
                        work.run.control().stop();
                    }
                    paused_map = run_map_work(work, &paths, &state, &mut native);
                }
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
                if let Some(work) = paused_combat.take() {
                    work.run.control().stop();
                    let _ = run_combat_work(work, &paths, &state, &mut native);
                }
                if let Some(mut work) = paused_group.take() {
                    work.control(false, &mut charge_worker);
                    let _ = run_group_work(work, &paths, &state, &mut native, &mut charge_worker);
                }
                if let Some(work) = paused_coffee.take() {
                    work.run.control().stop();
                    let _ = run_coffee_work(work, &paths, &state, &mut native);
                }
                if let Some(work) = paused_map.take() {
                    work.run.control().stop();
                    let _ = run_map_work(work, &paths, &state, &mut native);
                }
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
    if let Some(mut work) = paused_group.take() {
        work.control(false, &mut charge_worker);
        let _ = run_group_work(work, &paths, &state, &mut native, &mut charge_worker);
    }
    if let Some(work) = paused_combat.take() {
        work.run.control().stop();
        let _ = run_combat_work(work, &paths, &state, &mut native);
    }
    if let Some(work) = paused_mail.take() {
        work.run.control().stop();
        let _ = run_mail_work(work, &paths, &state, &mut native);
    }
    if let Some(work) = paused_coffee.take() {
        work.run.control().stop();
        let _ = run_coffee_work(work, &paths, &state, &mut native);
    }
    if let Some(work) = paused_map.take() {
        work.run.control().stop();
        let _ = run_map_work(work, &paths, &state, &mut native);
    }
}

fn run_combat_work(
    mut work: Box<CombatWork>,
    paths: &EnginePaths,
    shared: &Arc<Mutex<State>>,
    native: &mut Option<NativeSession>,
) -> Option<Box<CombatWork>> {
    let result = (|| -> anyhow::Result<()> {
        if work.run.progress.state.is_terminal() {
            return Ok(());
        }
        if !work.prepared && !work.fixture {
            if let Some(controller) = native.as_mut() {
                controller.release_inputs()?;
            }
            native.take();
            let current = od_platform::parse_game_window(&work.window.hwnd)?;
            anyhow::ensure!(
                current.pid == work.window.pid && current.width == 1920 && current.height == 1080,
                "战斗游戏窗口身份或尺寸已变化"
            );
            let device = work
                .run
                .progress
                .config
                .directml_device
                .map_or(InferenceDevice::Cpu, InferenceDevice::DirectMl);
            *native = Some(NativeSession::for_game(
                &current,
                &work.recognizer.bundle.root,
                work.capture,
                work.run.progress.config.mode,
                device,
            )?);
        }
        work.prepared = true;
        let controller = native
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("战斗控制器未连接"))?;
        let id = work.run.id;
        work.run.execute(
            controller,
            &mut work.recognizer,
            !work.fixture,
            |progress| {
                let mut state = shared.lock().expect("运行状态锁失效");
                if progress.state == RunState::Running {
                    let _ = state.slot.update_combat(id, progress, "战斗运行中".into());
                }
            },
        )
    })();
    if let Err(error) = result {
        work.run.progress.state = RunState::Failed;
        work.run.message = format!("战斗失败：{error:#}");
        work.run.progress.inputs_released = native
            .as_mut()
            .is_none_or(|controller| controller.release_inputs().is_ok());
    }
    let mut state = shared.lock().expect("运行状态锁失效");
    if let Ok(record) = state.slot.update_combat(
        work.run.id,
        work.run.progress.clone(),
        work.run.message.clone(),
    ) {
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
    if work.run.progress.state == RunState::Paused {
        Some(work)
    } else {
        state.pipeline_control = None;
        state.combat_recognition = None;
        None
    }
}

fn run_map_work(
    mut work: MapWork,
    paths: &EnginePaths,
    shared: &Arc<Mutex<State>>,
    native: &mut Option<NativeSession>,
) -> Option<MapWork> {
    let result = (|| -> anyhow::Result<()> {
        if work.run.state.is_terminal() {
            return Ok(());
        }
        if !work.prepared && work.fixture {
            work.events = Some(
                native
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("离线地图控制器未准备"))?
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
                maa_adapter::validate_map_bundle(&paths.resources.join("generated/map-preview"))?;
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
            controller.load_map_bundle(&bundle)?;
            work.events = Some(controller.pipeline_events(EVENT_CAPACITY)?);
            *native = Some(controller);
            work.prepared = true;
        }
        let controller = native
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("地图控制器未连接"))?;
        let id = work.run.id;
        let events = work.events.as_ref();
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
                .update_map(id, progress, "地图传送试运行中".into());
        };
        if work.fixture {
            work.run.execute_fixture_observed(controller, publish)
        } else {
            work.run.execute_game(controller, publish)
        }
    })();
    if let Err(error) = result {
        work.run.state = MapState::Failed;
        work.run.message = format!("地图流程失败：{error:#}");
        work.run.inputs_released = native
            .as_mut()
            .is_none_or(|controller| controller.release_inputs().is_ok());
    }
    let mut state = shared.lock().expect("运行状态锁失效");
    if let Some(events) = &work.events {
        for event in events.receiver.try_iter() {
            state.event(
                Some(work.run.id),
                "debug",
                format!("{} {}", event.message, event.node.as_deref().unwrap_or("")),
            );
        }
        let dropped = events.dropped();
        state.dropped += dropped.saturating_sub(work.dropped);
        work.dropped = dropped;
    }
    if let Ok(record) =
        state
            .slot
            .update_map(work.run.id, work.run.progress(), work.run.message.clone())
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
    if work.run.state == MapState::Paused {
        Some(work)
    } else {
        state.pipeline_control = None;
        None
    }
}

fn run_coffee_work(
    mut work: CoffeeWork,
    paths: &EnginePaths,
    shared: &Arc<Mutex<State>>,
    native: &mut Option<NativeSession>,
) -> Option<CoffeeWork> {
    let result = (|| -> anyhow::Result<()> {
        if work.run.progress.state.is_terminal() {
            return Ok(());
        }
        if !work.prepared && work.fixture {
            work.events = Some(
                native
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("离线咖啡控制器未准备"))?
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
            let bundle = maa_adapter::validate_coffee_run_bundle(
                &paths.resources.join("generated/coffee-run"),
            )?;
            let map_bundle = if work.run.progress.approach.is_some() {
                maa_adapter::validate_coffee_approach_bundle(&bundle)?;
                Some(maa_adapter::validate_map_bundle(
                    &paths.resources.join("generated/map-preview"),
                )?)
            } else {
                None
            };
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
            if let Some(map_bundle) = map_bundle {
                controller.load_map_bundle(&map_bundle)?;
            }
            controller.load_coffee_run_bundle(&bundle)?;
            work.events = Some(controller.pipeline_events(EVENT_CAPACITY)?);
            *native = Some(controller);
            work.prepared = true;
        }
        let controller = native
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("咖啡控制器未连接"))?;
        let id = work.run.id;
        let events = work.events.as_ref();
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
                .update_coffee(id, progress, "咖啡试运行中".into());
        };
        if work.fixture {
            work.run.execute(controller, false, publish)
        } else {
            work.run.execute(controller, true, publish)
        }
    })();
    if let Err(error) = result {
        work.run.progress.state = CoffeeState::Failed;
        work.run.message = format!("咖啡流程失败：{error:#}");
        work.run.progress.inputs_released = native
            .as_mut()
            .is_none_or(|controller| controller.release_inputs().is_ok());
    }
    let mut state = shared.lock().expect("运行状态锁失效");
    if let Some(events) = &work.events {
        for event in events.receiver.try_iter() {
            state.event(
                Some(work.run.id),
                "debug",
                format!("{} {}", event.message, event.node.as_deref().unwrap_or("")),
            );
        }
        let dropped = events.dropped();
        state.dropped += dropped.saturating_sub(work.dropped);
        work.dropped = dropped;
    }
    if let Ok(record) = state.slot.update_coffee(
        work.run.id,
        work.run.progress.clone(),
        work.run.message.clone(),
    ) {
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
    if work.run.progress.state == CoffeeState::Paused {
        Some(work)
    } else {
        state.pipeline_control = None;
        None
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
        state.pipeline_control = None;
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
    } else {
        let _ = state.slot.confirm_group_saved(record);
        if state
            .failed_save
            .as_ref()
            .is_some_and(|failed| failed.id == record.id)
        {
            state.failed_save = None;
            state.record_error = None;
        }
    }
}
