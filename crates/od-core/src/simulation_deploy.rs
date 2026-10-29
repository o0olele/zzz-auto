//! 模拟室出战与战斗进入；关卡证明必须由父任务提供。
use crate::{
    charge_plan::ChargePlanItem,
    simulation_mission::{MissionReceipt, MissionRequest},
    simulation_preparation::{PreparationOutcome, PreparationResult},
    simulation_teams::TeamsResult,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployRequest {
    pub mission: MissionReceipt,
    pub team: TeamsResult,
    pub timeout_ms: u32,
}
/// 共享出战步骤只依赖已核对的前置、选队和预算；各副本保留自己的身份类型。
pub trait DeploymentContext: std::fmt::Debug + Clone + PartialEq {
    fn validate(&self) -> Result<(), &'static str>;
    fn team(&self) -> &TeamsResult;
    fn timeout_ms(&self) -> u32;
    fn requires_forward(&self) -> bool {
        true
    }
}
impl DeploymentContext for DeployRequest {
    fn validate(&self) -> Result<(), &'static str> {
        DeployRequest::validate(self)
    }
    fn team(&self) -> &TeamsResult {
        &self.team
    }
    fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }
}
impl DeployRequest {
    pub fn from_plan(
        plan: &ChargePlanItem,
        mission: MissionReceipt,
        prepared: PreparationResult,
        team: TeamsResult,
        timeout_ms: u32,
    ) -> Result<Self, &'static str> {
        let expected = MissionRequest::from_plan(plan, timeout_ms)?;
        prepared.charge.validate()?;
        if plan.level != "默认等级"
            || mission.mission_type != expected.mission_type
            || mission.mission_name != expected.mission_name
            || prepared.outcome != PreparationOutcome::Ready
            || prepared.charge.current_charge < prepared.charge.required_charge
            || plan
                .card_num
                .is_some_and(|n| prepared.charge.required_charge != u32::from(n) * 20)
            || team.predefined_team_idx != plan.predefined_team_idx
            || (plan.predefined_team_idx < 0 && team.script_name != plan.auto_battle_config)
        {
            return Err("模拟室出战前置与冻结计划不一致或电量不足");
        }
        let mut mission = mission;
        mission.charge = prepared.charge;
        let request = Self {
            mission,
            team,
            timeout_ms,
        };
        request.validate()?;
        Ok(request)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        MissionRequest {
            mission_type: self.mission.mission_type.clone(),
            mission_name: self.mission.mission_name.clone(),
            timeout_ms: self.timeout_ms,
        }
        .validate()?;
        self.mission.charge.validate()?;
        if !self.mission.selection_verified
            || self.mission.charge.current_charge < self.mission.charge.required_charge
            || !(-1..20).contains(&self.team.predefined_team_idx)
        {
            return Err("缺少关卡选中证明、电量或有效选队前置，不能消费电量");
        }
        crate::intel_board::IntelBoardConfig {
            predefined_team_idx: self.team.predefined_team_idx,
            auto_battle_config: self.team.script_name.clone(),
            exp_grind_mode: false,
        }
        .validate()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployView {
    Unknown,
    Deployment,
    LowLevel,
    Blocked,
    InventoryFull,
    Battle,
    Result,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployAction {
    Deploy,
    ConfirmLevel,
    Forward,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Receipt {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployFlow<R = DeployRequest> {
    request: R,
    deploy: Option<Receipt>,
    confirm: Option<Receipt>,
    forward: Option<Receipt>,
    entered: bool,
}
impl<R: DeploymentContext> DeployFlow<R> {
    pub fn new(request: R) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            deploy: None,
            confirm: None,
            forward: None,
            entered: false,
        })
    }
    pub fn request(&self) -> &R {
        &self.request
    }
    pub fn completed(&self) -> bool {
        self.entered
    }
    /// 意图保存后无法排除已经投递；等待战斗时不能重复点击出战。
    pub fn deployment_started(&self) -> bool {
        self.deploy.is_some()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if (self.confirm.is_some() && self.deploy.is_none())
            || (self.forward.is_some() && self.deploy.is_none())
            || (self.entered && self.request.requires_forward() && self.forward.is_none())
        {
            return Err("模拟室出战检查点缺少投递或战斗进入证明");
        }
        Ok(())
    }
    pub fn next(&mut self, view: DeployView) -> Result<Option<DeployAction>, &'static str> {
        self.validate()?;
        if self.completed() {
            return Ok(None);
        }
        match view {
            DeployView::InventoryFull => Err("驱动盘数量已达到可拥有上限"),
            DeployView::Result => Err("没有本次战斗进入与子战斗结果，不能采用既有结算"),
            DeployView::Blocked => Err("出战出现未支持或无法确认身份的弹窗"),
            DeployView::Deployment if self.deploy.is_none() => Ok(Some(DeployAction::Deploy)),
            DeployView::LowLevel if self.deploy.is_none() => {
                Err("出战投递前出现确认弹窗，不能采用")
            }
            DeployView::LowLevel if self.forward.is_some() => Err("前移后意外返回出战确认"),
            DeployView::LowLevel if self.confirm.is_none() => Ok(Some(DeployAction::ConfirmLevel)),
            DeployView::Battle if self.deploy.is_none() => {
                Err("缺少本次出战意图，不能采用既有战斗")
            }
            DeployView::Battle if self.request.requires_forward() && self.forward.is_none() => {
                Ok(Some(DeployAction::Forward))
            }
            DeployView::Battle => {
                self.entered = true;
                Ok(None)
            }
            _ => Ok(None),
        }
    }
    fn receipt(&self, a: DeployAction) -> Option<Receipt> {
        match a {
            DeployAction::Deploy => self.deploy,
            DeployAction::ConfirmLevel => self.confirm,
            DeployAction::Forward => self.forward,
        }
    }
    fn set(&mut self, a: DeployAction, r: Option<Receipt>) {
        match a {
            DeployAction::Deploy => self.deploy = r,
            DeployAction::ConfirmLevel => self.confirm = r,
            DeployAction::Forward => self.forward = r,
        }
    }
    pub fn prepare(&mut self, a: DeployAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.completed()
            || self.receipt(a).is_some()
            || (a != DeployAction::Deploy && self.deploy.is_none())
            || (a == DeployAction::Forward && !self.request.requires_forward())
        {
            return Err("没有待保存的出战动作");
        }
        self.set(a, Some(Receipt::Intent));
        Ok(())
    }
    pub fn allows(&self, a: DeployAction, v: DeployView) -> bool {
        !self.completed()
            && (a != DeployAction::Forward || self.request.requires_forward())
            && self.receipt(a) == Some(Receipt::Intent)
            && matches!(
                (a, v),
                (DeployAction::Deploy, DeployView::Deployment)
                    | (DeployAction::ConfirmLevel, DeployView::LowLevel)
                    | (DeployAction::Forward, DeployView::Battle)
            )
    }
    pub fn posted(&mut self, a: DeployAction) -> Result<(), &'static str> {
        if self.receipt(a) != Some(Receipt::Intent) {
            return Err("出战动作没有意图");
        }
        self.set(a, Some(Receipt::Posted));
        Ok(())
    }
    pub fn not_posted(&mut self, a: DeployAction) -> Result<(), &'static str> {
        if self.receipt(a) != Some(Receipt::Intent) {
            return Err("已投递出战动作不能撤销");
        }
        self.set(a, None);
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.entered = false;
        Ok(())
    }
}
