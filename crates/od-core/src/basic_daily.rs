//! 首批日常的业务检查点；识别和输入只由 Maa Pipeline 执行。
use crate::mail::MailState;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const LEGACY_APP_IDS: [&str; 3] = ["engagement_reward", "city_fund", "scratch_card"];
pub const SCHEMA_TWO_APP_IDS: [&str; 6] = [
    "engagement_reward",
    "city_fund",
    "scratch_card",
    "ridu_weekly",
    "trigrams_collection",
    "hou_hou_bakery",
];

pub const SCHEMA_THREE_APP_IDS: [&str; 7] = [
    "engagement_reward",
    "city_fund",
    "scratch_card",
    "ridu_weekly",
    "trigrams_collection",
    "hou_hou_bakery",
    "drive_disc_dismantle",
];

pub const SCHEMA_FOUR_APP_IDS: [&str; 8] = [
    "engagement_reward",
    "city_fund",
    "scratch_card",
    "ridu_weekly",
    "trigrams_collection",
    "hou_hou_bakery",
    "drive_disc_dismantle",
    "redemption_code",
];
pub const SCHEMA_FIVE_APP_IDS: [&str; 9] = [
    "engagement_reward",
    "city_fund",
    "scratch_card",
    "ridu_weekly",
    "trigrams_collection",
    "hou_hou_bakery",
    "drive_disc_dismantle",
    "redemption_code",
    "random_play",
];
pub const SCHEMA_SIX_APP_IDS: [&str; 10] = [
    "engagement_reward",
    "city_fund",
    "scratch_card",
    "ridu_weekly",
    "trigrams_collection",
    "hou_hou_bakery",
    "drive_disc_dismantle",
    "redemption_code",
    "random_play",
    "predefined_team_checker",
];
pub const APP_IDS: [&str; 10] = SCHEMA_SIX_APP_IDS;
/// 资源包先提供情报板内部节点，运行注册仍由 APP_IDS 单独控制。
pub const RESOURCE_APP_IDS: [&str; 11] = [
    "engagement_reward",
    "city_fund",
    "scratch_card",
    "ridu_weekly",
    "trigrams_collection",
    "hou_hou_bakery",
    "drive_disc_dismantle",
    "redemption_code",
    "random_play",
    "predefined_team_checker",
    "intel_board",
];

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub intel_board: Option<crate::intel_board::IntelLedger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub team_check: Option<crate::team_check::TeamCheckProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub random_play: Option<crate::random_play::RandomPlayProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub redemption: Option<crate::redemption::RedemptionProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub dismantle: Option<crate::dismantle::DismantleProgress>,
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
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub reward_checked: bool,
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub reward_confirmed: bool,
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub reveal_observed: bool,
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
            intel_board: None,
            team_check: if app_id == "predefined_team_checker" {
                Some(crate::team_check::TeamCheckProgress::new(
                    Default::default(),
                )?)
            } else {
                None
            },
            random_play: if app_id == "random_play" {
                Some(crate::random_play::RandomPlayProgress::new(
                    Default::default(),
                    "1970-01-01".into(),
                )?)
            } else {
                None
            },
            app_id: app_id.into(),
            redemption: (app_id == "redemption_code").then(Default::default),
            dismantle: (app_id == "drive_disc_dismantle")
                .then(|| crate::dismantle::DismantleProgress::new(Default::default())),
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
            reward_checked: false,
            reward_confirmed: false,
            reveal_observed: false,
            active_ms: 0,
        })
    }
    pub fn prefix(&self) -> &'static str {
        match self.app_id.as_str() {
            "engagement_reward" => "Engagement",
            "city_fund" => "Fund",
            "drive_disc_dismantle" => "Disc",
            "redemption_code" => "Redeem",
            "random_play" => "Shop",
            "predefined_team_checker" => "Teams",
            "ridu_weekly" => "Weekly",
            "trigrams_collection" => "Trigram",
            "hou_hou_bakery" => "Bakery",
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
        crate::dismantle::before_step(self);
        crate::redemption::before_step(self);
        crate::random_play::before_step(self);
        if matches!(
            self.stage.as_str(),
            "Claim" | "ClaimTasks" | "ClaimLevels" | "SelectCard" | "SelectGift" | "ClaimScore"
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
        if self.stage == "ClaimReward" {
            self.reward_checked = true;
        }
        self.inputs_released = false;
    }
    pub fn completed(&self) -> bool {
        self.navigation_completed()
            && self
                .team_check
                .as_ref()
                .is_none_or(|scan| scan.saved_revision.is_some())
            && self.stage == "Done"
    }
    pub fn timeout_ms(&self) -> u32 {
        self.team_check
            .as_ref()
            .map_or(self.config.timeout_ms, |scan| scan.config.check_timeout_ms)
    }
    pub fn navigation_completed(&self) -> bool {
        (self.stage == "Done" || (self.app_id == "predefined_team_checker" && self.stage == "Save"))
            && (self.app_id != "predefined_team_checker"
                || self
                    .team_check
                    .as_ref()
                    .is_some_and(|s| s.validate().is_ok() && s.scan_completed()))
            && (self.app_id != "random_play"
                || self
                    .random_play
                    .as_ref()
                    .is_some_and(|s| s.validate().is_ok() && s.outcome.is_some()))
            && (self.app_id != "redemption_code"
                || self
                    .redemption
                    .as_ref()
                    .is_some_and(crate::redemption::RedemptionProgress::completed))
            && (self.app_id != "drive_disc_dismantle"
                || self
                    .dismantle
                    .as_ref()
                    .is_some_and(crate::dismantle::DismantleProgress::completed))
            && self.completion_proven
            && self.returned_world
            && self.inputs_released
    }
    pub fn resume(&mut self) {
        if self.app_id == "predefined_team_checker" {
            if self.stage != "Done" {
                if let Some(scan) = &mut self.team_check {
                    let _ = scan.restart();
                }
                self.stage = "Inspect".into();
                self.returned_world = false;
                self.completion_proven = false;
            }
            self.state = MailState::Running;
            return;
        }
        if self.app_id == "random_play" {
            if self.stage != "Done" {
                crate::random_play::resume(self);
            }
            self.state = MailState::Running;
            return;
        }
        if self.app_id == "redemption_code" {
            if self.stage != "Done" {
                crate::redemption::resume(self);
            }
            self.state = MailState::Running;
            return;
        }
        if self.app_id == "drive_disc_dismantle" {
            if self.stage != "Done" {
                crate::dismantle::resume(self);
            }
            self.state = MailState::Running;
            return;
        }
        // 返回阶段不退回入口；刮层只重做当前未确认完成的拖动。
        if self.stage != "Return" && self.stage != "Done" {
            self.stage = if matches!(
                self.app_id.as_str(),
                "scratch_card" | "trigrams_collection" | "hou_hou_bakery"
            ) && self.transport_attempted
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
        if self.app_id == "predefined_team_checker" {
            return crate::team_check::observe(self, node);
        }
        if self.app_id == "random_play" {
            return crate::random_play::observe(self, node);
        }
        if self.app_id == "redemption_code" {
            return crate::redemption::observe(self, node);
        }
        if self.app_id == "drive_disc_dismantle" {
            return crate::dismantle::observe(self, node);
        }
        if !LEGACY_APP_IDS.contains(&self.app_id.as_str()) {
            return super::remaining_daily::observe(self, node);
        }
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
pub use definition::pipeline as legacy_pipeline;

/// schema 1..6 保持冻结；schema 7 加入情报板界面节点及距离检测资源。
pub fn current_pipeline() -> serde_json::Value {
    super::intel_board::pipeline()
}

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
