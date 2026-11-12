//! 周常免费挑战的出战前置和实际余量结算；不使用深度追猎电量证明。
use crate::{
    battle_result::NormalBattleResult,
    charge_transport::canonical_name,
    deep_hunt::same_mission,
    notorious_hunt_preparation::{WeeklyPreparationFlow, WeeklyPreparationStage},
    simulation_deploy::DeploymentContext,
    simulation_settlement::SettlementAction,
    simulation_teams::TeamsResult,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyDeployRequest {
    pub preparation: WeeklyPreparationFlow,
    pub team: TeamsResult,
    pub timeout_ms: u32,
}
impl DeploymentContext for WeeklyDeployRequest {
    fn validate(&self) -> Result<(), &'static str> {
        self.preparation.validate()?;
        let expected = self.preparation.request().team_request()?;
        if self.preparation.stage() != WeeklyPreparationStage::Ready
            || !(1000..=900_000).contains(&self.timeout_ms)
            || self.team.predefined_team_idx != expected.predefined_team_idx
            || self.team.script_name != expected.script_name
            || self.team.selection_verified
        {
            return Err("周常出战缺少本次免费准备与选队证明");
        }
        Ok(())
    }
    fn team(&self) -> &TeamsResult {
        &self.team
    }
    fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }
    fn requires_forward(&self) -> bool {
        false
    }
    fn accepts_interaction_entry(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklySettlementView {
    Unknown,
    Blocked,
    Failed,
    Other,
    Settlement,
    Reward,
    Returned { mission_type: String, rewards: u8 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Delivery {
    Intent,
    Posted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklySettlementFlow {
    battle: NormalBattleResult,
    timeout_ms: u32,
    mission_type: String,
    rewards_before: u8,
    finish: Option<Delivery>,
    settlement_seen: bool,
    reward_seen: bool,
    confirm: Option<Delivery>,
    returned: Option<u8>,
    candidate: Option<u8>,
    stable: u8,
}
impl WeeklySettlementFlow {
    pub fn new(
        battle: NormalBattleResult,
        timeout_ms: u32,
        mission_type: String,
        rewards_before: u8,
    ) -> Result<Self, &'static str> {
        let flow = Self {
            battle,
            timeout_ms,
            mission_type,
            rewards_before,
            finish: None,
            settlement_seen: false,
            reward_seen: false,
            confirm: None,
            returned: None,
            candidate: None,
            stable: 0,
        };
        flow.validate()?;
        Ok(flow)
    }
    pub fn mission_type(&self) -> &str {
        &self.mission_type
    }
    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }
    pub fn rewards_before(&self) -> u8 {
        self.rewards_before
    }
    pub fn rewards_after(&self) -> Option<u8> {
        self.returned
    }
    pub fn completed(&self) -> bool {
        self.returned.is_some()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let settled = self.finish.is_some() && (!self.reward_seen || self.confirm.is_some());
        if canonical_name("恶名狩猎", &self.mission_type).is_none()
            || self.battle != NormalBattleResult::Completed
            || !(1000..=300_000).contains(&self.timeout_ms)
            || !(1..=3).contains(&self.rewards_before)
            || (self.finish.is_some() && !self.settlement_seen)
            || (self.reward_seen && self.finish.is_none())
            || (self.confirm.is_some() && !self.reward_seen)
            || self.stable > 2
            || (self.candidate.is_some() != (self.stable > 0))
            || self
                .candidate
                .is_some_and(|n| !settled || n.checked_add(1) != Some(self.rewards_before))
            || self
                .returned
                .is_some_and(|n| !settled || self.stable != 2 || self.candidate != Some(n))
        {
            return Err("周常结算缺少本次战斗、完成或免费余量减少证明");
        }
        Ok(())
    }
    pub fn next(
        &mut self,
        view: &WeeklySettlementView,
    ) -> Result<Option<SettlementAction>, &'static str> {
        use WeeklySettlementView as V;
        self.validate()?;
        if self.completed() {
            return Ok(None);
        }
        if !matches!(view, V::Returned { .. }) {
            self.candidate = None;
            self.stable = 0;
        }
        match view {
            V::Unknown | V::Blocked => Ok(None),
            V::Failed | V::Other => Err("周常结算进入失败或无关页面"),
            V::Settlement if self.reward_seen => Err("领奖后返回旧结算"),
            V::Settlement if self.finish.is_none() => {
                self.settlement_seen = true;
                Ok(Some(SettlementAction::Finish))
            }
            V::Settlement => Ok(None),
            V::Reward if self.finish.is_none() => Err("没有本次完成意图，不能采用既有奖励"),
            V::Reward => {
                self.reward_seen = true;
                Ok(self
                    .confirm
                    .is_none()
                    .then_some(SettlementAction::ConfirmReward))
            }
            V::Returned {
                mission_type,
                rewards,
            } => {
                if !same_mission(&self.mission_type, mission_type)
                    || rewards.checked_add(1) != Some(self.rewards_before)
                    || self.finish.is_none()
                    || (self.reward_seen && self.confirm.is_none())
                {
                    return Err("周常结算目标或免费余量未对应本次挑战");
                }
                self.stable = if self.candidate == Some(*rewards) {
                    (self.stable + 1).min(2)
                } else {
                    1
                };
                self.candidate = Some(*rewards);
                if self.stable == 2 {
                    self.returned = Some(*rewards);
                }
                Ok(None)
            }
        }
    }
    fn delivery(&mut self, a: SettlementAction) -> &mut Option<Delivery> {
        match a {
            SettlementAction::Finish => &mut self.finish,
            SettlementAction::ConfirmReward => &mut self.confirm,
        }
    }
    pub fn prepare(&mut self, a: SettlementAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.completed()
            || match a {
                SettlementAction::Finish => !self.settlement_seen || self.finish.is_some(),
                SettlementAction::ConfirmReward => !self.reward_seen || self.confirm.is_some(),
            }
        {
            return Err("周常结算动作不能重复或跨步骤");
        }
        *self.delivery(a) = Some(Delivery::Intent);
        Ok(())
    }
    pub fn allows(&self, a: SettlementAction, view: &WeeklySettlementView) -> bool {
        self.validate().is_ok()
            && !self.completed()
            && match (a, view) {
                (SettlementAction::Finish, WeeklySettlementView::Settlement) => {
                    self.finish == Some(Delivery::Intent) && !self.reward_seen
                }
                (SettlementAction::ConfirmReward, WeeklySettlementView::Reward) => {
                    self.reward_seen && self.confirm == Some(Delivery::Intent)
                }
                _ => false,
            }
    }
    pub fn posted(&mut self, a: SettlementAction) -> Result<(), &'static str> {
        if *self.delivery(a) != Some(Delivery::Intent) {
            return Err("周常结算投递不匹配");
        }
        *self.delivery(a) = Some(Delivery::Posted);
        Ok(())
    }
    pub fn not_posted(&mut self, a: SettlementAction) -> Result<(), &'static str> {
        if *self.delivery(a) != Some(Delivery::Intent) {
            return Err("已投递结算不能撤销");
        }
        *self.delivery(a) = None;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.returned = None;
        self.candidate = None;
        self.stable = 0;
        Ok(())
    }
}
