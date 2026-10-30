//! 从已验证的模拟室详情推进到出战页；不把投递当作到达证明。
use crate::{
    charge_plan::ChargePlanItem,
    predefined_teams::TeamConfig,
    simulation_cards::{CardsCharge, SimulationView},
    simulation_deploy::DeployRequest,
    simulation_mission::{MissionReceipt, MissionRequest},
    simulation_preparation::{PreparationOutcome, PreparationResult},
    simulation_teams::{TeamsRequest, TeamsResult},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryRequest {
    pub plan: ChargePlanItem,
    pub mission: MissionReceipt,
    pub prepared: PreparationResult,
    pub teams: TeamConfig,
    pub timeout_ms: u32,
}
impl EntryRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        let mission = MissionRequest::from_plan(&self.plan, self.timeout_ms)?;
        self.team_request()?;
        self.prepared.charge.validate()?;
        self.mission.charge.validate()?;
        if self.plan.level != "默认等级"
            || !self.mission.selection_verified
            || self.mission.mission_type != mission.mission_type
            || self.mission.mission_name != mission.mission_name
            || self.prepared.selection_verified
            || self.prepared.outcome != PreparationOutcome::Ready
            || self.prepared.charge.current_charge < self.prepared.charge.required_charge
            || self
                .plan
                .card_num
                .is_some_and(|n| self.prepared.charge.required_charge != u32::from(n) * 20)
        {
            return Err("详情推进缺少冻结关卡、实际电量或默认等级证明");
        }
        Ok(())
    }
    pub fn team_request(&self) -> Result<TeamsRequest, &'static str> {
        TeamsRequest::from_plan(&self.plan, &self.teams, self.timeout_ms)
    }
    pub fn deployment(&self, team: TeamsResult) -> Result<DeployRequest, &'static str> {
        self.validate()?;
        let expected = self.team_request()?;
        if team.predefined_team_idx != expected.predefined_team_idx
            || team.script_name != expected.script_name
            || team.selection_verified
        {
            return Err("详情推进与选队结果不属于同一冻结计划");
        }
        DeployRequest::from_plan(
            &self.plan,
            self.mission.clone(),
            self.prepared,
            team,
            self.timeout_ms,
        )
    }
    fn accepts_charge(&self, charge: CardsCharge) -> bool {
        charge.validate().is_ok()
            && charge.required_charge == self.prepared.charge.required_charge
            && charge.current_charge >= self.prepared.charge.current_charge
            && charge.current_charge >= charge.required_charge
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Receipt {
    Intent,
    Posted,
    Uncertain,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryFlow {
    request: EntryRequest,
    checked: bool,
    next: Option<Receipt>,
    arrived: bool,
}
impl EntryFlow {
    pub fn new(request: EntryRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            checked: false,
            next: None,
            arrived: false,
        })
    }
    pub fn request(&self) -> &EntryRequest {
        &self.request
    }
    pub fn completed(&self) -> bool {
        self.arrived
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if (self.next.is_some() && !self.checked) || (self.arrived && self.next.is_none()) {
            return Err("出战页证明缺少本次详情检查或推进意图");
        }
        Ok(())
    }
    /// true 表示需要保存下一步意图；只有意图之后的新出战页才能完成。
    pub fn observe(&mut self, view: &SimulationView) -> Result<bool, &'static str> {
        self.validate()?;
        match view {
            SimulationView::Challenge {
                current_charge,
                required_charge,
            } => {
                if !self.request.accepts_charge(CardsCharge {
                    current_charge: *current_charge,
                    required_charge: *required_charge,
                }) {
                    return Err("详情电量或卡片方案已变化，须重新准备");
                }
                self.checked = true;
            }
            SimulationView::Deployment if self.next.is_some() => self.arrived = true,
            SimulationView::Unknown => {}
            SimulationView::Blocked => return Err("详情推进出现弹窗，不能继续选队或出战"),
            _ => return Err("没有本次推进证明或已离开模拟室详情"),
        }
        Ok(
            matches!(view, SimulationView::Challenge { .. })
                && self.next.is_none()
                && !self.arrived,
        )
    }
    pub fn prepare_next(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if !self.checked || self.next.is_some() || self.arrived {
            return Err("没有待保存的详情推进意图");
        }
        self.next = Some(Receipt::Intent);
        Ok(())
    }
    pub fn allows_next(&self, view: &SimulationView) -> bool {
        self.validate().is_ok()
            && self.next == Some(Receipt::Intent)
            && !self.arrived
            && matches!(view, SimulationView::Challenge { current_charge, required_charge }
            if self.request.accepts_charge(CardsCharge {
                current_charge: *current_charge, required_charge: *required_charge,
            }))
    }
    pub fn posted_next(&mut self) -> Result<(), &'static str> {
        if self.next != Some(Receipt::Intent) {
            return Err("没有匹配的详情推进意图");
        }
        self.next = Some(Receipt::Posted);
        self.validate()
    }
    pub fn not_posted_next(&mut self) -> Result<(), &'static str> {
        if self.next != Some(Receipt::Intent) {
            return Err("没有可撤销的详情推进意图");
        }
        self.next = None;
        self.checked = false;
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        // 不确定投递也保留意图；继续只等待出战页，绝不重复点击。
        self.arrived = false;
        if self.next == Some(Receipt::Intent) {
            self.next = Some(Receipt::Uncertain);
        } else if self.next.is_none() {
            self.checked = false;
        }
        Ok(())
    }
}
