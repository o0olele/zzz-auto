//! 原计划保持代理人身份，具体目标由本次导航解析。
use crate::{
    agent_transport::AgentTransportRequest,
    charge_plan::{ChargePlanItem, RestoreCharge},
    predefined_teams::TeamConfig,
    simulation_teams::TeamsRequest,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentChallengeResult {
    Completed {
        charge_after: crate::simulation_cards::CardsCharge,
        goal: crate::agent_goal::AgentGoalStatus,
    },
    Unavailable,
    Insufficient {
        charge: crate::simulation_cards::CardsCharge,
    },
    WeeklyRewardsAvailable {
        remaining: u8,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentChallengeRequest {
    pub plan: ChargePlanItem,
    pub restore_charge: RestoreCharge,
    pub teams: TeamConfig,
    pub timeout_ms: u32,
}
impl AgentChallengeRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        AgentTransportRequest {
            plan: self.plan.clone(),
            timeout_ms: self.timeout_ms,
        }
        .validate()?;
        if self.plan.category_name == "实战模拟室" {
            return Err("模拟室代理人挑战尚需生产选中证明；目标投递不能授权消费");
        }
        TeamsRequest {
            predefined_team_idx: self.plan.predefined_team_idx,
            teams: self.teams.clone(),
            script_name: self.plan.script(&self.teams)?.into(),
            timeout_ms: self.timeout_ms,
        }
        .validate()
    }
}
