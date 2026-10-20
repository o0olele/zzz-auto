use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct MailConfig {
    pub timeout_ms: u32,
    pub start_delay_ms: u32,
}
impl Default for MailConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 60_000,
            start_delay_ms: 3000,
        }
    }
}
impl MailConfig {
    pub const MAX_TIMEOUT_MS: u32 = 60_000;
    pub const MAX_START_DELAY_MS: u32 = 10_000;

    pub fn validate(self) -> Result<(), &'static str> {
        if !(1..=Self::MAX_TIMEOUT_MS).contains(&self.timeout_ms)
            || self.start_delay_ms > Self::MAX_START_DELAY_MS
        {
            return Err("邮件超时须为 1..60000ms，启动 / 继续延时须为 0..10000ms");
        }
        Ok(())
    }
    pub fn recovery_inputs() -> [crate::InputProbeConfig; 2] {
        use crate::{InputMode, InputProbeAction, InputProbeConfig, MouseButton};
        [
            InputProbeConfig {
                mode: InputMode::Foreground,
                action: InputProbeAction::Key { key: 18 },
                hold_ms: 1,
                start_delay_ms: 0,
            },
            InputProbeConfig {
                mode: InputMode::Foreground,
                action: InputProbeAction::MouseButton {
                    button: MouseButton::Left,
                    x: 0,
                    y: 0,
                },
                hold_ms: 1,
                start_delay_ms: 0,
            },
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MailState {
    Running,
    Paused,
    Succeeded,
    Failed,
    Stopped,
    TimedOut,
}
impl MailState {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running | Self::Paused)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MailPhase {
    Navigate,
    Inspect,
    Claim,
    AfterClaim,
    Confirm,
    AfterConfirm,
    ReturnList,
    ReturnWorld,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct MailProgress {
    pub config: MailConfig,
    pub state: MailState,
    pub phase: MailPhase,
    pub claim_attempted: bool,
    pub reward_observed: bool,
    pub inputs_released: bool,
    #[ts(type = "number")]
    pub active_ms: u64,
}

impl MailProgress {
    pub fn new(config: MailConfig) -> Self {
        Self {
            config,
            state: MailState::Running,
            phase: MailPhase::Navigate,
            claim_attempted: false,
            reward_observed: false,
            inputs_released: false,
            active_ms: 0,
        }
    }
}

/// 领取按钮可能先变灰、稍后才出现奖励；返回列表期间也要优先检查弹窗。
/// 通过任务覆盖保留既有邮件包的固定定义与哈希校验。
pub fn pipeline_overrides() -> serde_json::Value {
    serde_json::json!({
        "Mail.ReturnList": {"next": ["Mail.Dialog", "Mail.CloseList"]},
        "Mail.CloseList": {"next": ["Mail.Dialog", "Mail.Menu"]}
    })
}
