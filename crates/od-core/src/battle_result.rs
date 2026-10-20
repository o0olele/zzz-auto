//! 普通战斗的结束证据。进入战斗和连续两张新结果帧共同确认，不把失去战斗画面当成胜利。
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NormalBattleResult {
    Completed,
    Retreat,
}

#[derive(Debug, Default)]
pub struct BattleEndTracker {
    seen_battle: bool,
    previous: Option<NormalBattleResult>,
}
impl BattleEndTracker {
    pub fn observe(
        &mut self,
        in_battle: bool,
        result: Option<NormalBattleResult>,
        fresh: bool,
    ) -> Option<NormalBattleResult> {
        if !fresh || in_battle {
            self.seen_battle |= fresh && in_battle;
            self.previous = None;
            return None;
        }
        if !self.seen_battle {
            return None;
        }
        let confirmed = result.filter(|value| Some(*value) == self.previous);
        self.previous = result;
        confirmed
    }

    /// 暂停前的战斗进入证据保留，结果必须在继续后重新连续确认。
    pub fn reset_confirmation(&mut self) {
        self.previous = None;
    }
}
