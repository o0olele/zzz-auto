use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

pub const BASE_WIDTH: i32 = 1920;
pub const BASE_HEIGHT: i32 = 1080;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AppDefinition {
    pub id: String,
    pub name: String,
    pub execution: String,
    pub reason: String,
    pub factory: String,
    pub configs: Vec<String>,
    pub settings: Vec<String>,
    pub run_records: Vec<String>,
    pub source_files: Vec<String>,
    pub priority: i32,
    pub default_group: bool,
    pub implemented: bool,
}

pub fn applications() -> Result<Vec<AppDefinition>, serde_json::Error> {
    serde_json::from_str(include_str!("../../../resources/catalog/applications.json"))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    Stopping,
    Succeeded,
    Failed,
    Stopped,
}

impl RunState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Stopped)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RunRecord {
    pub id: Uuid,
    pub app_id: String,
    pub state: RunState,
    #[ts(type = "string")]
    pub started_at: DateTime<Utc>,
    #[ts(type = "string | null")]
    pub finished_at: Option<DateTime<Utc>>,
    pub message: String,
    #[serde(default)]
    pub input_probe: Option<InputProbeResult>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    #[default]
    Foreground,
    ForegroundLegacy,
    Background,
    BackgroundPost,
    Xbox,
    Ds4,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
pub struct InputProbeConfig {
    pub mode: InputMode,
    pub key: i32,
    pub hold_ms: u32,
    pub start_delay_ms: u32,
}

impl InputProbeConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=2000).contains(&self.hold_ms) || self.start_delay_ms > 10_000 {
            return Err("按住时间须为 1..2000ms，启动延时须为 0..10000ms");
        }
        let valid = match self.mode {
            InputMode::Foreground
            | InputMode::ForegroundLegacy
            | InputMode::Background
            | InputMode::BackgroundPost => (1..=254).contains(&self.key),
            // 固定 SDK 的 XUSB 数字按键，DS4 另含 PS 与触摸板按键。
            InputMode::Xbox => self.key > 0 && self.key & !0xf7ff == 0,
            InputMode::Ds4 => self.key > 0 && self.key & !0x3f7ff == 0,
        };
        if !valid {
            return Err("输入模式与 VK / 手柄按键位掩码不匹配");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct InputProbeResult {
    pub config: InputProbeConfig,
    pub pressed: bool,
    pub released: bool,
    pub stopped: bool,
    #[ts(type = "number")]
    pub enqueue_us: u64,
    #[ts(type = "number")]
    pub key_down_ms: u64,
    #[ts(type = "number")]
    pub held_ms: u64,
    #[ts(type = "number")]
    pub release_ms: u64,
    #[ts(type = "number | null")]
    pub stop_ms: Option<u64>,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RunEvent {
    #[ts(type = "number")]
    pub sequence: u64,
    pub run_id: Option<Uuid>,
    #[ts(type = "string")]
    pub at: DateTime<Utc>,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct RunSlot {
    current: Option<RunRecord>,
}

impl RunSlot {
    pub fn current(&self) -> Option<&RunRecord> {
        self.current.as_ref()
    }

    pub fn busy(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|run| !run.state.is_terminal())
    }

    pub fn begin(&mut self, app_id: &str) -> Result<RunRecord, &'static str> {
        if self.busy() {
            return Err("已有任务占用游戏控制权，请先等待停止完成");
        }
        let run = RunRecord {
            id: Uuid::new_v4(),
            app_id: app_id.into(),
            state: RunState::Running,
            started_at: Utc::now(),
            finished_at: None,
            message: "任务已启动".into(),
            input_probe: None,
        };
        self.current = Some(run.clone());
        Ok(run)
    }

    pub fn request_stop(&mut self, id: Uuid) -> Result<RunRecord, &'static str> {
        let run = self.current.as_mut().ok_or("当前没有任务")?;
        if run.id != id {
            return Err("运行 ID 已过期");
        }
        if !run.state.is_terminal() {
            run.state = RunState::Stopping;
            run.message = "已请求停止，正在等待原生队列及输入释放".into();
        }
        Ok(run.clone())
    }

    pub fn finish(
        &mut self,
        id: Uuid,
        state: RunState,
        message: String,
    ) -> Result<RunRecord, &'static str> {
        if !state.is_terminal() {
            return Err("完成状态必须是终态");
        }
        let run = self.current.as_mut().ok_or("当前没有任务")?;
        if run.id != id {
            return Err("运行 ID 已过期");
        }
        if run.state.is_terminal() {
            return Err("任务已结束，不能覆盖结果");
        }
        run.state = state;
        run.message = message;
        run.finished_at = Some(Utc::now());
        Ok(run.clone())
    }

    pub fn finish_input_probe(
        &mut self,
        id: Uuid,
        result: InputProbeResult,
    ) -> Result<RunRecord, &'static str> {
        if !result.released {
            return Err("输入尚未确认释放，不能完成输入诊断");
        }
        let state = if result.stopped {
            RunState::Stopped
        } else {
            RunState::Succeeded
        };
        self.finish(
            id,
            state,
            if result.stopped {
                "输入诊断已停止，输入已释放"
            } else {
                "输入诊断完成，输入已释放"
            }
            .into(),
        )?;
        let run = self.current.as_mut().ok_or("当前没有任务")?;
        run.input_probe = Some(result);
        Ok(run.clone())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct Roi(pub i32, pub i32, pub i32, pub i32);

impl Roi {
    pub fn from_legacy(rect: [i32; 4]) -> Result<Self, &'static str> {
        let [x1, y1, x2, y2] = rect;
        if x1 < 0 || y1 < 0 || x2 > BASE_WIDTH || y2 > BASE_HEIGHT || x1 >= x2 || y1 >= y2 {
            return Err("区域必须位于 1920×1080 画面中，且宽高为正数");
        }
        Ok(Self(x1, y1, x2 - x1, y2 - y1))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WindowInfo {
    pub hwnd: String,
    pub pid: u32,
    pub title: String,
    pub executable: String,
    pub width: i32,
    pub height: i32,
    #[serde(default)]
    pub keyboard_permission: Option<KeyboardPermission>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KeyboardPermission {
    pub engine_integrity: u32,
    pub game_integrity: u32,
    pub compatible: bool,
}

impl KeyboardPermission {
    pub fn from_integrity(engine_integrity: u32, game_integrity: u32) -> Self {
        Self {
            engine_integrity,
            game_integrity,
            compatible: engine_integrity >= game_integrity,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SdkStatus {
    pub ready: bool,
    pub version: Option<String>,
    pub expected_version: String,
    pub binding_version: String,
    pub directory: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMethod {
    #[default]
    FramePool,
    PrintWindow,
    Dxgi,
    ScreenDc,
    Gdi,
}

impl CaptureMethod {
    pub fn name(self) -> &'static str {
        match self {
            Self::FramePool => "FramePool",
            Self::PrintWindow => "PrintWindow",
            Self::Dxgi => "DXGI",
            Self::ScreenDc => "ScreenDC",
            Self::Gdi => "GDI",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Snapshot {
    pub engine_version: String,
    pub sdk: SdkStatus,
    pub window: Option<WindowInfo>,
    pub capture_method: Option<CaptureMethod>,
    pub run: Option<RunRecord>,
    pub events: Vec<RunEvent>,
    #[ts(type = "number")]
    pub event_sequence: u64,
    #[ts(type = "number")]
    pub dropped_events: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ScreenshotQuality {
    pub mean_luma: f64,
    pub dark_pixel_ratio: f64,
    pub near_black: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Screenshot {
    pub png_base64: String,
    pub width: i32,
    pub height: i32,
    pub color_order: String,
    pub capture_method: Option<CaptureMethod>,
    pub quality: ScreenshotQuality,
    #[ts(type = "string")]
    pub captured_at: DateTime<Utc>,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
}
