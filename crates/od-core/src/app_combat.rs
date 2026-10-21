//! 应用内界面与战斗共用的输入恢复清单；不开放额外输入动作。
use crate::{InputMode, InputProbeAction, InputProbeConfig, combat::CombatConfig};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AppCombatPlan {
    pub config: CombatConfig,
    pub script_revision: String,
}
impl AppCombatPlan {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate()?;
        if self.config.dodge_only
            || self.config.start_delay_ms != 0
            || self.script_revision.len() != 64
            || !self
                .script_revision
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("应用内战斗须冻结完整战斗脚本、有效修订及零启动延时");
        }
        Ok(())
    }
}

pub fn intel_settings(
    instance: &crate::config::AccountInstance,
    settings: &crate::intel_board::IntelBoardConfig,
) -> Result<CombatConfig, &'static str> {
    let mut combat = instance.combat.clone();
    combat.script = settings.script(&instance.predefined_teams)?.into();
    combat.dodge_only = false;
    combat.start_delay_ms = 0;
    combat.validate()?;
    Ok(combat)
}

/// UI Pipeline、选队和战前移动固定使用前台键鼠，覆盖应用组的全部输入。
pub fn interface_recovery_inputs() -> Vec<InputProbeConfig> {
    let mut inputs = crate::redemption::recovery_inputs();
    for key in [83, 68] {
        inputs.push(InputProbeConfig {
            mode: InputMode::Foreground,
            action: InputProbeAction::Key { key },
            hold_ms: 1,
            start_delay_ms: 0,
        });
    }
    inputs
}

/// 战斗绑定按冻结配置展开；相同动作仅在输入方式也相同时去重。
pub fn recovery_inputs(combat: &CombatConfig) -> Result<Vec<InputProbeConfig>, &'static str> {
    let mut inputs = interface_recovery_inputs();
    for input in combat.recovery_inputs()? {
        if !inputs.contains(&input) {
            inputs.push(input);
        }
    }
    Ok(inputs)
}
