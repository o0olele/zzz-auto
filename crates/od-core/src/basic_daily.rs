//! 首批日常的业务检查点；识别和输入只由 Maa Pipeline 执行。
use crate::mail::MailState;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const APP_IDS: [&str; 3] = ["engagement_reward", "city_fund", "scratch_card"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DailyConfig {
    pub timeout_ms: u32,
    pub start_delay_ms: u32,
}
impl Default for DailyConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 120_000,
            start_delay_ms: 3000,
        }
    }
}
impl DailyConfig {
    pub fn fields() -> serde_json::Value {
        serde_json::json!([
            {"key":"timeout_ms","label":"日常超时（ms）","min":1,"max":180000},
            {"key":"start_delay_ms","label":"日常启动 / 继续延时（ms）","min":0,"max":10000}
        ])
    }
    pub fn validate(self) -> Result<(), &'static str> {
        if !(1..=180_000).contains(&self.timeout_ms) || self.start_delay_ms > 10_000 {
            return Err("日常超时须为 1..180000ms，启动 / 继续延时须为 0..10000ms");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DailyProgress {
    pub app_id: String,
    pub config: DailyConfig,
    pub state: MailState,
    pub stage: String,
    pub attempted: bool,
    pub completion_proven: bool,
    pub returned_world: bool,
    pub inputs_released: bool,
    pub scratch_rows: u8,
    pub tasks_checked: bool,
    pub levels_selected: bool,
    pub route_confirmed: bool,
    pub activation_attempted: bool,
    pub transport_attempted: bool,
    pub approach_attempted: bool,
    #[ts(type = "number")]
    pub active_ms: u64,
}
impl DailyProgress {
    pub fn new(app_id: &str, config: DailyConfig) -> Result<Self, &'static str> {
        config.validate()?;
        if !APP_IDS.contains(&app_id) {
            return Err("日常应用未注册");
        }
        Ok(Self {
            app_id: app_id.into(),
            config,
            state: MailState::Running,
            stage: "Inspect".into(),
            attempted: false,
            completion_proven: false,
            returned_world: false,
            inputs_released: true,
            scratch_rows: 0,
            tasks_checked: false,
            levels_selected: false,
            route_confirmed: false,
            activation_attempted: false,
            transport_attempted: false,
            approach_attempted: false,
            active_ms: 0,
        })
    }
    pub fn prefix(&self) -> &'static str {
        match self.app_id.as_str() {
            "engagement_reward" => "Engagement",
            "city_fund" => "Fund",
            _ => "Scratch",
        }
    }
    pub fn entry(&self) -> String {
        let stage = if self.app_id == "scratch_card" && self.stage == "Dialogue" {
            "DialogueStep"
        } else {
            &self.stage
        };
        format!("{}.{}", self.prefix(), stage)
    }
    /// 先于投递发布；中断后只能重新识别后态，不能直接再次投递领取。
    pub fn before_step(&mut self) {
        if matches!(
            self.stage.as_str(),
            "Claim" | "ClaimTasks" | "ClaimLevels" | "SelectCard"
        ) {
            self.attempted = true;
        }
        if self.stage == "Activate" {
            self.activation_attempted = true;
        }
        if self.stage == "ConfirmTransport" {
            self.transport_attempted = true;
        }
        if self.stage == "Approach" {
            self.approach_attempted = true;
        }
        self.inputs_released = false;
    }
    pub fn completed(&self) -> bool {
        self.stage == "Done"
            && self.completion_proven
            && self.returned_world
            && self.inputs_released
    }
    pub fn resume(&mut self) {
        // 返回阶段不退回入口；刮层只重做当前未确认完成的拖动。
        if self.stage != "Return" && self.stage != "Done" {
            self.stage = if self.app_id == "scratch_card"
                && self.transport_attempted
                && !self.route_confirmed
            {
                "WaitArrival"
            } else {
                "Inspect"
            }
            .into();
        }
        self.state = MailState::Running;
    }
    /// 只接受当前步骤定义的终点，不以 SDK 的成功状态代替业务证明。
    pub fn observe(&mut self, node: &str) -> Result<(), &'static str> {
        let prefix = format!("{}.", self.prefix());
        let node = node.strip_prefix(&prefix).ok_or("日常节点不属于当前应用")?;
        let stage = self.stage.clone();
        let next = match (self.app_id.as_str(), stage.as_str(), node) {
            (_, "Return", "World") => {
                self.returned_world = true;
                if self.app_id == "scratch_card" && self.scratch_rows == 3 {
                    self.completion_proven = true;
                }
                "Done"
            }
            ("engagement_reward", "Inspect" | "AfterClaim" | "Verify", "Completed") => {
                self.completion_proven = true;
                "Return"
            }
            ("engagement_reward", "Inspect" | "AfterClaim", "Dialog") if self.attempted => {
                "Confirm"
            }
            ("engagement_reward", "Inspect" | "AfterClaim", "Preview") if self.attempted => {
                "ClosePreview"
            }
            ("engagement_reward", "Inspect", "Daily") if !self.attempted => "Claim",
            ("engagement_reward", "Inspect" | "AfterClaim" | "Verify", "Daily")
                if self.attempted =>
            {
                "Return"
            }
            ("engagement_reward", "Inspect", "Compendium") => "SelectDaily",
            ("engagement_reward", "Inspect", "World") => "OpenCompendium",
            ("engagement_reward", "Inspect", "Menu") => "CloseMenu",
            ("engagement_reward", "OpenCompendium" | "SelectDaily" | "CloseMenu", "Navigated") => {
                "Inspect"
            }
            ("engagement_reward", "Claim", "Claimed") => "AfterClaim",
            ("engagement_reward", "Confirm" | "ClosePreview", "Dismissed") => "Verify",
            ("city_fund", "Inspect", "World") => "OpenMenu",
            ("city_fund", "Inspect", "Menu") => "OpenFund",
            ("city_fund", "OpenMenu" | "OpenFund", "Navigated") => "Inspect",
            ("city_fund", "Inspect", "ActivationAvailable") => "Activate",
            ("city_fund", "Activate", "Activated") => "Inspect",
            ("city_fund", "Inspect", "ActivationConfirm")
                if self.activation_attempted && !self.tasks_checked =>
            {
                "ConfirmActivation"
            }
            ("city_fund", "ConfirmActivation", "Activated") => "Inspect",
            ("city_fund", "Inspect", "Page") if !self.tasks_checked => "SelectTasks",
            ("city_fund", "SelectTasks", "TaskSelected") => "InspectTasks",
            ("city_fund", "InspectTasks", "TaskClaim") => "ClaimTasks",
            ("city_fund", "ClaimTasks", "Claimed") => "AfterTasks",
            ("city_fund", "AfterTasks" | "Inspect", "Dialog")
                if !self.tasks_checked && self.attempted =>
            {
                "ConfirmTasks"
            }
            ("city_fund", "ConfirmTasks", "Dismissed") => "AfterTasks",
            ("city_fund", "AfterTasks" | "InspectTasks", "TasksDone") => {
                self.tasks_checked = true;
                "SelectLevels"
            }
            ("city_fund", "SelectLevels", "LevelSelected") => {
                self.levels_selected = true;
                "InspectLevels"
            }
            ("city_fund", "Inspect", "Page") if self.tasks_checked => "SelectLevels",
            ("city_fund", "Inspect" | "InspectLevels", "Dialog") if self.tasks_checked => {
                "ConfirmLevels"
            }
            ("city_fund", "InspectLevels", "LevelClaim") => "ClaimLevels",
            ("city_fund", "ClaimLevels", "Claimed") => "InspectLevels",
            ("city_fund", "ConfirmLevels", "Dismissed") => "InspectLevels",
            ("city_fund", "InspectLevels", "LevelsDone")
                if self.tasks_checked && self.levels_selected =>
            {
                self.completion_proven = true;
                "Return"
            }
            ("scratch_card", "Inspect", "World") if !self.route_confirmed => "OpenMap",
            ("scratch_card", "Inspect", "World") if self.scratch_rows == 3 => {
                self.completion_proven = true;
                "Return"
            }
            ("scratch_card", "Inspect", "World") if self.route_confirmed && !self.attempted => {
                if self.approach_attempted {
                    "WaitInteraction"
                } else {
                    "Approach"
                }
            }
            ("scratch_card", "Inspect", "MapOpened") => "SelectArea",
            ("scratch_card", "Inspect", "AreaSelected") => "SelectPoint",
            ("scratch_card", "Inspect", "PointSelected") if !self.transport_attempted => {
                "ConfirmTransport"
            }
            ("scratch_card", "OpenMap", "MapOpened") => "SelectArea",
            ("scratch_card", "SelectArea", "AreaSelected") => "SelectPoint",
            ("scratch_card", "SelectPoint", "PointSelected") => "ConfirmTransport",
            ("scratch_card", "ConfirmTransport", "Transported") => "WaitArrival",
            ("scratch_card", "WaitArrival", "Arrived") => {
                self.route_confirmed = true;
                "Approach"
            }
            ("scratch_card", "Approach", "Interacted") => "Inspect",
            (
                "scratch_card",
                "Inspect" | "WaitInteraction",
                "Wake" | "Dialogue" | "Awake" | "Npc",
            ) => "Dialogue",
            ("scratch_card", "Dialogue", "Advanced") => "Inspect",
            ("scratch_card", "Inspect" | "WaitInteraction", "CardButton")
                if self.scratch_rows < 3 =>
            {
                "SelectCard"
            }
            ("scratch_card", "SelectCard", "Selected") => "Inspect",
            ("scratch_card", "Inspect" | "WaitInteraction", "Card") if self.scratch_rows < 3 => {
                "ScratchRow"
            }
            ("scratch_card", "ScratchRow", "Scratched") => {
                self.scratch_rows += 1;
                if self.scratch_rows == 3 {
                    "Return"
                } else {
                    "Inspect"
                }
            }
            ("scratch_card", "AfterScratch" | "Inspect", "Used") => {
                self.completion_proven = true;
                "ConfirmUsed"
            }
            ("scratch_card", "ConfirmUsed", "Dismissed") => "Return",
            ("scratch_card", "AfterScratch" | "Inspect", "Result") if self.scratch_rows == 3 => {
                self.completion_proven = true;
                "Return"
            }
            _ => return Err("日常流程未到达当前检查点允许的业务节点"),
        };
        self.stage = next.into();
        Ok(())
    }
}

/// 新版本独立登记，旧恢复记录的输入列表保持原义。
pub fn recovery_inputs() -> Vec<crate::InputProbeConfig> {
    let mut inputs = crate::group_run::recovery_inputs();
    for key in [27, 87] {
        inputs.push(crate::InputProbeConfig {
            mode: crate::InputMode::Foreground,
            action: crate::InputProbeAction::Key { key },
            hold_ms: 1,
            start_delay_ms: 0,
        });
    }
    inputs
}

#[path = "basic_daily_pipeline.rs"]
mod definition;
pub use definition::pipeline;

pub fn templates() -> [(&'static str, &'static [u8]); 7] {
    [
        (
            "world_function",
            include_bytes!("../../../resources/source/daily/world_function.png"),
        ),
        (
            "world_preset",
            include_bytes!("../../../resources/source/daily/world_preset.png"),
        ),
        (
            "back",
            include_bytes!("../../../resources/source/daily/back.png"),
        ),
        (
            "menu_wallpaper",
            include_bytes!("../../../resources/source/daily/menu_wallpaper.png"),
        ),
        (
            "completed",
            include_bytes!("../../../resources/source/daily/completed.png"),
        ),
        (
            "compendium",
            include_bytes!("../../../resources/source/daily/compendium.png"),
        ),
        (
            "close",
            include_bytes!("../../../resources/source/daily/close.png"),
        ),
    ]
}
