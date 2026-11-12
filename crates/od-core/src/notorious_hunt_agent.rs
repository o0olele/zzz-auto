//! 免费周常培养保留原计划，实际副本只取自本次导航目标。
use crate::{
    agent_transport::{AgentTransportReceipt, AgentTransportRequest},
    charge_plan::ChargePlanItem,
    notorious_hunt::NotoriousHuntConfig,
    notorious_hunt_preparation::WeeklyPreparationRequest,
    predefined_teams::TeamConfig,
    simulation_teams::TeamsRequest,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyAgentRequest {
    pub plan: ChargePlanItem,
    pub teams: TeamConfig,
    pub timeout_ms: u32,
}
impl WeeklyAgentRequest {
    pub fn navigation(&self) -> AgentTransportRequest {
        AgentTransportRequest {
            plan: self.plan.clone(),
            timeout_ms: self.timeout_ms,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.navigation().validate()?;
        NotoriousHuntConfig {
            plan_list: vec![self.plan.clone()],
            ..Default::default()
        }
        .validate()?;
        TeamsRequest {
            predefined_team_idx: self.plan.predefined_team_idx,
            teams: self.teams.clone(),
            script_name: self.plan.script(&self.teams)?.into(),
            timeout_ms: self.timeout_ms,
        }
        .validate()
    }
    pub fn resolve(
        &self,
        navigation: &AgentTransportReceipt,
    ) -> Result<WeeklyPreparationRequest, &'static str> {
        self.validate()?;
        navigation.validate()?;
        if navigation.plan != self.plan {
            return Err("周常培养导航不属于冻结的原计划");
        }
        let request = WeeklyPreparationRequest {
            plan: navigation.resolved_plan()?,
            teams: self.teams.clone(),
            timeout_ms: self.timeout_ms,
        };
        request.validate()?;
        Ok(request)
    }
}
