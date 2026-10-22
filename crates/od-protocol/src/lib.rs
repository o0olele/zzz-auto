use od_core::{
    AppDefinition, CaptureMethod, InputPipelineConfig, InputProbeConfig, RunEvent, RunRecord,
    Screenshot, Snapshot, WindowInfo,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use ts_rs::TS;
use uuid::Uuid;

// v34 接通 Rust 战斗；桌面与引擎须成套使用。
pub const PROTOCOL_VERSION: u32 = 45;
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

// 配对密钥不实现 Debug，避免日志意外输出。
#[derive(Serialize, Deserialize)]
pub struct Handshake {
    pub version: u32,
    pub token: String,
}

// 回连入口的应答包含一次性密钥，不实现 Debug 或发往前端。
#[derive(Serialize, Deserialize)]
pub struct EngineGreeting {
    pub version: u32,
    pub token: String,
    pub engine_integrity: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct EnginePrivilege {
    pub engine_integrity: u32,
    pub desktop_integrity: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Request {
    pub version: u32,
    pub id: Uuid,
    pub command: Command,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "params", rename_all = "snake_case")]
pub enum Command {
    PreviewChargePlan {
        instance_id: u32,
        group_id: String,
        revision: u32,
        #[serde(default)]
        resources: Option<od_core::charge_plan::ChargeResources>,
        #[serde(default)]
        double_reward_left: Option<u8>,
    },
    CombatScripts,
    StartCombat {
        instance_id: u32,
        revision: u32,
        script_revision: String,
        #[serde(default)]
        allow_input: bool,
        #[serde(default)]
        confirm_account: bool,
    },
    PreviewCombatScriptImport {
        source: String,
    },
    ApplyCombatScriptImport {
        source: String,
        preview_id: String,
    },
    LaunchGame {
        instance_id: u32,
        revision: u32,
        #[serde(default)]
        confirm: bool,
    },
    ModelResources,
    StartModelResources {
        kind: od_core::model_resources::ResourceJobKind,
        models: Vec<String>,
        #[serde(default)]
        offline: bool,
    },
    CancelModelResources {
        job_id: Uuid,
    },
    PreviewStandalone {
        instance_id: u32,
        group_id: String,
        app_id: String,
    },
    StartStandaloneTrial {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        combat_revision: Option<String>,
        instance_id: u32,
        group_id: String,
        app_id: String,
        revision: u32,
        game_day: String,
        #[serde(default)]
        allow_input: bool,
        #[serde(default)]
        confirm_account: bool,
    },
    PreviewGroup {
        instance_id: u32,
        group_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        app_ids: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[ts(as = "Option<bool>", optional)]
        rerun_completed: bool,
    },
    StartGroupTrial {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        combat_revision: Option<String>,
        instance_id: u32,
        group_id: String,
        revision: u32,
        game_day: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        app_ids: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[ts(as = "Option<bool>", optional)]
        rerun_completed: bool,
        #[serde(default)]
        allow_input: bool,
        #[serde(default)]
        confirm_account: bool,
    },
    RunHistory {
        instance_id: u32,
    },
    QueryRunHistory {
        instance_id: u32,
        query: od_core::run_history::HistoryQuery,
    },
    ExportRunHistory {
        instance_id: u32,
        query: od_core::run_history::HistoryQuery,
        content_id: String,
    },
    RetryRunSave,
    AccountConfig,
    ConfigBackups,
    PreviewConfigBackup {
        revision: u32,
        id: String,
    },
    RestoreConfigBackup {
        revision: u32,
        id: String,
        preview_id: String,
        #[serde(default)]
        confirm: bool,
    },
    PreviewConfigImport {
        source: String,
    },
    ApplyConfigImport {
        source: String,
        preview_id: String,
    },
    UpdateAccountConfig {
        revision: u32,
        change: od_core::config::ConfigChange,
    },
    StartMail {
        config: od_core::mail::MailConfig,
        #[serde(default)]
        allow_input: bool,
    },
    StartMap {
        config: od_core::map_transport::MapConfig,
        #[serde(default)]
        allow_input: bool,
    },
    StartCoffee {
        config: od_core::coffee_run::CoffeeConfig,
        #[serde(default)]
        allow_input: bool,
    },
    StartCoffeeFromWorld {
        config: od_core::coffee_run::CoffeeConfig,
        #[serde(default)]
        allow_input: bool,
    },
    Pause {
        run_id: Uuid,
    },
    Resume {
        run_id: Uuid,
    },
    Snapshot,
    Applications,
    Windows,
    Attach {
        hwnd: String,
        #[serde(default)]
        capture_method: CaptureMethod,
    },
    Screenshot,
    StartProbe,
    StartInputProbe {
        config: InputProbeConfig,
        #[serde(default)]
        allow_input: bool,
    },
    StartInputPipeline {
        config: InputPipelineConfig,
        #[serde(default)]
        allow_input: bool,
    },
    StartApplication {
        app_id: String,
    },
    Stop {
        run_id: Uuid,
    },
    Events {
        #[ts(type = "number")]
        after: u64,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[allow(
    clippy::large_enum_variant,
    reason = "协议在有界队列内传递完整运行快照，保留统一的拥有值接口"
)]
pub enum Payload {
    ChargePlanPreview(od_core::charge_plan::ChargeGroupPreview),
    CombatScripts(od_core::combat::CombatScriptCatalog),
    CombatScriptImport(od_core::combat_scripts::CombatScriptImportPreview),
    GameLaunched(od_core::game_launch::GameLaunchResult),
    ModelResources(od_core::model_resources::ModelResources),
    GroupPlan(od_core::run_plan::GroupPlan),
    RunHistory(od_core::run_history::RunHistory),
    HistoryPage(od_core::run_history::HistoryPage),
    HistoryExport(od_core::run_history::HistoryExport),
    AccountConfig(od_core::config::AccountConfig),
    ConfigBackups(od_core::config::ConfigBackups),
    ConfigBackupPreview(od_core::config::ConfigBackupPreview),
    ConfigImportPreview(od_core::config_import::ImportPreview),
    Snapshot(Box<Snapshot>),
    Applications(Vec<AppDefinition>),
    Windows(Vec<WindowInfo>),
    Screenshot(Screenshot),
    Run(RunRecord),
    Events(Vec<RunEvent>),
    Ack,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub details: Option<String>,
}

impl ApiError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Response {
    pub version: u32,
    pub id: Uuid,
    #[ts(type = "Payload | null")]
    pub result: Option<Payload>,
    pub error: Option<ApiError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum DesktopUpdate {
    RunNotifications(od_core::run_notifications::RunNotifications),
    OpenRunNotifications,
    AfterDone(Option<od_core::after_done::AfterDoneStatus>),
    Snapshot(Box<Snapshot>),
    Offline(ApiError),
    Privilege(EnginePrivilege),
    Shortcuts(od_core::shortcuts::ShortcutStatus),
}

// 在原生按键到达时冻结运行 ID；发送前不重新选择其他任务。
pub fn shortcut_command(
    action: od_core::shortcuts::ShortcutAction,
    run: Option<&RunRecord>,
) -> Option<Command> {
    use od_core::{RunState, shortcuts::ShortcutAction};
    let run = run?;
    if run.state.is_terminal() || run.state == RunState::Stopping {
        return None;
    }
    let run_id = run.id;
    match action {
        ShortcutAction::Stop => Some(Command::Stop { run_id }),
        ShortcutAction::PauseResume
            if run.mail.is_some()
                || run.map.is_some()
                || run.coffee.is_some()
                || run.combat.is_some()
                || !run.daily.is_empty() =>
        {
            match run.state {
                RunState::Running => Some(Command::Pause { run_id }),
                RunState::Paused => Some(Command::Resume { run_id }),
                _ => None,
            }
        }
        _ => None,
    }
}

impl Response {
    pub fn from_result(id: Uuid, value: Result<Payload, ApiError>) -> Self {
        let (result, error) = match value {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };
        Self {
            version: PROTOCOL_VERSION,
            id,
            result,
            error,
        }
    }
}

pub async fn read_frame<T: serde::de::DeserializeOwned>(
    reader: &mut (impl AsyncRead + Unpin),
    limit: usize,
) -> std::io::Result<T> {
    let size = reader.read_u32_le().await? as usize;
    if size == 0 || size > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "消息长度超出限制",
        ));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub async fn write_frame<T: Serialize>(
    writer: &mut (impl AsyncWrite + Unpin),
    value: &T,
    limit: usize,
) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "消息长度超出限制",
        ));
    }
    writer.write_u32_le(bytes.len() as u32).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await
}
