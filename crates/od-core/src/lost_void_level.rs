//! 层内实际交接结果；本身不提供整场完成或日周记录。
use crate::{
    lost_void::ArtifactSelection,
    lost_void_battle::{BattleOutcome, BattleReceipt},
    lost_void_scene::LostVoidScreen as S,
    lost_void_transition::ArrivalProof,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LevelOutcome {
    NextLevel {
        arrival: ArrivalProof,
    },
    ObservedResult {
        screen: S,
        battle: Option<BattleReceipt>,
    },
    AgentDead {
        battle: BattleReceipt,
    },
    Aborted {
        failure: crate::lost_void_recovery::LayerFailure,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidLevelReceipt {
    pub run_id: Uuid,
    pub level_id: Uuid,
    pub region: String,
    pub options: ArtifactSelection,
    pub outcome: LevelOutcome,
}
impl LostVoidLevelReceipt {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.run_id.is_nil()
            || self.level_id.is_nil()
            || !crate::lost_void::REGIONS.contains(&self.region.as_str())
        {
            return Err("层回执身份或区域无效");
        }
        let battle = match &self.outcome {
            LevelOutcome::Aborted { failure } => {
                if failure.id.is_nil()
                    || failure.run_id != self.run_id
                    || failure.attempt_id != self.level_id
                {
                    return Err("恢复终止结果缺少本层实际失败来源");
                }
                None
            }
            LevelOutcome::NextLevel { arrival } => {
                if arrival.level_id.is_nil() || arrival.level_id == self.level_id {
                    return Err("下一层身份无效");
                }
                None
            }
            LevelOutcome::ObservedResult { screen, battle } => {
                if !matches!(screen, S::BattleResult | S::BattleFail) {
                    return Err("层结果不是结算页面");
                }
                battle.as_ref()
            }
            LevelOutcome::AgentDead { battle } => {
                if battle.outcome != BattleOutcome::AgentDead {
                    return Err("层阵亡缺少对应战斗结果");
                }
                Some(battle)
            }
        };
        if battle.is_some_and(|b| {
            b.run_id != self.run_id || b.level_id != self.level_id || b.encounter_id.is_nil()
        }) {
            return Err("层回执战斗身份不符");
        }
        Ok(())
    }
}
