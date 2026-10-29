//! 普通区域巡防的电量、详情与结算；目标来源是本次完整传送，保留独立类别。
use crate::{
    battle_result::NormalBattleResult,
    charge_plan::{ChargePlanItem, RestoreCharge},
    charge_transport::{TrainingArrival, TrainingTransportReceipt, TrainingTransportRequest},
    predefined_teams::TeamConfig,
    restore_charge::{RestoreFlow, RestoreRequest},
    simulation_cards::CardsCharge,
    simulation_deploy::DeploymentContext,
    simulation_settlement::{SettlementAction, SettlementView},
    simulation_teams::{TeamsRequest, TeamsResult},
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatrolRequest {
    pub plan: ChargePlanItem,
    pub restore_charge: RestoreCharge,
    pub teams: TeamConfig,
    pub timeout_ms: u32,
}
impl PatrolRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        TrainingTransportRequest {
            plan: self.plan.clone(),
            timeout_ms: self.timeout_ms,
        }
        .validate()?;
        if self.plan.category_name != "区域巡防" || self.plan.level != "默认等级" {
            return Err("区域巡防仅接通普通副本的默认等级");
        }
        self.team_request()?.validate()
    }
    pub fn team_request(&self) -> Result<TeamsRequest, &'static str> {
        Ok(TeamsRequest {
            predefined_team_idx: self.plan.predefined_team_idx,
            teams: self.teams.clone(),
            script_name: self.plan.script(&self.teams)?.into(),
            timeout_ms: self.timeout_ms,
        })
    }
}
pub fn valid_charge(charge: CardsCharge) -> bool {
    charge.validate().is_ok() && charge.required_charge == 60
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatrolEntryStage {
    Inspect,
    AwaitPopup,
    Restore,
    Verify,
    AwaitDeployment,
    Ready,
    Insufficient,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatrolEntryAction {
    Next,
    OpenRestore,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatrolView {
    Unknown,
    Blocked,
    Detail(CardsCharge),
    RestorePopup,
    Deployment,
    Settlement,
    Reward,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Receipt {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatrolEntryFlow {
    request: PatrolRequest,
    route: TrainingTransportReceipt,
    stage: PatrolEntryStage,
    before: Option<CardsCharge>,
    open: Option<Receipt>,
    next: Option<Receipt>,
    restored: bool,
}
impl PatrolEntryFlow {
    pub fn new(
        request: PatrolRequest,
        route: TrainingTransportReceipt,
    ) -> Result<Self, &'static str> {
        let flow = Self {
            request,
            route,
            stage: PatrolEntryStage::Inspect,
            before: None,
            open: None,
            next: None,
            restored: false,
        };
        flow.validate()?;
        Ok(flow)
    }
    pub fn request(&self) -> &PatrolRequest {
        &self.request
    }
    pub fn stage(&self) -> PatrolEntryStage {
        self.stage
    }
    pub fn charge(&self) -> Option<CardsCharge> {
        self.before
    }
    pub fn ready(&self) -> bool {
        self.stage == PatrolEntryStage::Ready
    }
    pub fn insufficient(&self) -> bool {
        self.stage == PatrolEntryStage::Insufficient
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.route.category_name != "区域巡防"
            || self.route.mission_type != self.request.plan.execution_mission_type()
            || self.route.selection_verified
            || !matches!(self.route.arrival,TrainingArrival::PatrolDetail {current,required:60} if current<=1_000_000)
            || self.before.is_some_and(|c| !valid_charge(c))
            || (self.open.is_some() && self.request.restore_charge == RestoreCharge::None)
            || (self.restored && self.open.is_none())
            || (matches!(
                self.stage,
                PatrolEntryStage::AwaitPopup | PatrolEntryStage::Restore
            ) && self.open.is_none())
            || (matches!(
                self.stage,
                PatrolEntryStage::AwaitDeployment | PatrolEntryStage::Ready
            ) && self.next.is_none())
            || (self.ready() && self.before.is_none_or(|c| c.current_charge < 60))
            || (self.insufficient() && self.before.is_none_or(|c| c.current_charge >= 60))
        {
            return Err("区域巡防详情缺少本次传送、电量或动作意图");
        }
        Ok(())
    }
    pub fn next(&mut self, view: &PatrolView) -> Result<Option<PatrolEntryAction>, &'static str> {
        self.validate()?;
        use PatrolEntryStage as S;
        if self.ready() || self.insufficient() {
            return Ok(None);
        }
        match view {
            PatrolView::Unknown => return Ok(None),
            PatrolView::Blocked
            | PatrolView::Settlement
            | PatrolView::Reward
            | PatrolView::Failed => return Err("区域巡防详情出现未支持的弹窗、结果或战斗画面"),
            _ => (),
        }
        match self.stage {
            S::Inspect | S::Verify => match view {
                PatrolView::Detail(c) if valid_charge(*c) => {
                    self.before = Some(*c);
                    if c.current_charge >= 60 {
                        Ok(Some(PatrolEntryAction::Next))
                    } else if self.request.restore_charge == RestoreCharge::None || self.restored {
                        self.stage = S::Insufficient;
                        Ok(None)
                    } else {
                        Ok(Some(PatrolEntryAction::OpenRestore))
                    }
                }
                _ => Err("没有本次下一步意图，不能采用已有出战页或恢复弹窗"),
            },
            S::AwaitPopup => match view {
                PatrolView::RestorePopup => {
                    self.stage = S::Restore;
                    Ok(None)
                }
                PatrolView::Deployment => Err("恢复意图后意外进入出战页，不能视为足额准备"),
                _ => Ok(None),
            },
            S::AwaitDeployment => match view {
                PatrolView::Deployment => {
                    self.stage = S::Ready;
                    Ok(None)
                }
                PatrolView::RestorePopup => Err("足额下一步后出现恢复弹窗，需要重新核对资源"),
                _ => Ok(None),
            },
            S::Restore => Ok(None),
            S::Ready | S::Insufficient => Ok(None),
        }
    }
    pub fn prepare(&mut self, a: PatrolEntryAction) -> Result<(), &'static str> {
        self.validate()?;
        if !matches!(
            self.stage,
            PatrolEntryStage::Inspect | PatrolEntryStage::Verify
        ) {
            return Err("区域巡防没有待保存的详情动作");
        }
        let c = self.before.ok_or("区域巡防电量未知")?;
        match a {
            PatrolEntryAction::Next if c.current_charge >= 60 && self.next.is_none() => {
                self.next = Some(Receipt::Intent);
                self.stage = PatrolEntryStage::AwaitDeployment;
            }
            PatrolEntryAction::OpenRestore
                if c.current_charge < 60
                    && self.open.is_none()
                    && self.request.restore_charge != RestoreCharge::None =>
            {
                self.open = Some(Receipt::Intent);
                self.stage = PatrolEntryStage::AwaitPopup;
            }
            _ => return Err("区域巡防详情动作与实际电量不一致"),
        };
        Ok(())
    }
    pub fn allows(&self, a: PatrolEntryAction, v: &PatrolView) -> bool {
        self.validate().is_ok()
            && matches!(v,PatrolView::Detail(c) if Some(*c)==self.before)
            && match a {
                PatrolEntryAction::Next => {
                    self.stage == PatrolEntryStage::AwaitDeployment
                        && self.next == Some(Receipt::Intent)
                }
                PatrolEntryAction::OpenRestore => {
                    self.stage == PatrolEntryStage::AwaitPopup && self.open == Some(Receipt::Intent)
                }
            }
    }
    fn receipt(&mut self, a: PatrolEntryAction) -> &mut Option<Receipt> {
        match a {
            PatrolEntryAction::Next => &mut self.next,
            PatrolEntryAction::OpenRestore => &mut self.open,
        }
    }
    pub fn posted(&mut self, a: PatrolEntryAction) -> Result<(), &'static str> {
        if *self.receipt(a) != Some(Receipt::Intent) {
            return Err("区域巡防详情投递不匹配");
        }
        *self.receipt(a) = Some(Receipt::Posted);
        self.validate()
    }
    pub fn not_posted(&mut self, a: PatrolEntryAction) -> Result<(), &'static str> {
        if *self.receipt(a) != Some(Receipt::Intent) {
            return Err("已投递区域巡防详情不能撤销");
        }
        *self.receipt(a) = None;
        self.before = None;
        self.stage = if self.restored {
            PatrolEntryStage::Verify
        } else {
            PatrolEntryStage::Inspect
        };
        self.validate()
    }
    pub fn restore_request(&self) -> Result<RestoreRequest, &'static str> {
        self.validate()?;
        if self.stage != PatrolEntryStage::Restore {
            return Err("区域巡防尚未见到本次恢复主弹窗");
        }
        let c = self.before.ok_or("区域巡防电量未知")?;
        let r = RestoreRequest {
            policy: self.request.restore_charge,
            current_charge: c.current_charge,
            required_charge: 60,
            timeout_ms: self.request.timeout_ms.min(300_000),
        };
        r.validate()?;
        Ok(r)
    }
    pub fn restored(&mut self, flow: &RestoreFlow) -> Result<(), &'static str> {
        flow.validate()?;
        if !flow.completed()
            || flow.request() != self.restore_request()?
            || flow.charge_after().is_none()
        {
            return Err("区域巡防恢复子流程或返回证明不一致");
        }
        self.restored = true;
        self.before = None;
        self.stage = PatrolEntryStage::Verify;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.ready() {
            self.stage = PatrolEntryStage::AwaitDeployment;
        } else if self.insufficient() {
            self.stage = PatrolEntryStage::Verify;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatrolDeployRequest {
    pub entry: PatrolEntryFlow,
    pub team: TeamsResult,
    pub timeout_ms: u32,
}
impl DeploymentContext for PatrolDeployRequest {
    fn validate(&self) -> Result<(), &'static str> {
        self.entry.validate()?;
        let expected = self.entry.request.team_request()?;
        if !self.entry.ready()
            || !(1000..=900_000).contains(&self.timeout_ms)
            || self.team.predefined_team_idx != expected.predefined_team_idx
            || self.team.script_name != expected.script_name
            || self.team.selection_verified
        {
            return Err("区域巡防出战缺少本次详情或冻结选队证明");
        }
        Ok(())
    }
    fn team(&self) -> &TeamsResult {
        &self.team
    }
    fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatrolSettlementFlow {
    battle: NormalBattleResult,
    timeout_ms: u32,
    finish: Option<Receipt>,
    settlement_seen: bool,
    reward_seen: bool,
    confirm: Option<Receipt>,
    returned: Option<CardsCharge>,
}
impl PatrolSettlementFlow {
    pub fn new(battle: NormalBattleResult, timeout_ms: u32) -> Result<Self, &'static str> {
        let f = Self {
            battle,
            timeout_ms,
            finish: None,
            settlement_seen: false,
            reward_seen: false,
            confirm: None,
            returned: None,
        };
        f.validate()?;
        Ok(f)
    }
    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }
    pub fn completed(&self) -> bool {
        self.returned.is_some()
    }
    pub fn charge(&self) -> Option<CardsCharge> {
        self.returned
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.battle != NormalBattleResult::Completed
            || !(1000..=300_000).contains(&self.timeout_ms)
            || (self.finish.is_some() && !self.settlement_seen)
            || (self.reward_seen && self.finish.is_none())
            || (self.confirm.is_some() && !self.reward_seen)
            || (self.returned.is_some()
                && (self.finish.is_none() || (self.reward_seen && self.confirm.is_none())))
            || self.returned.is_some_and(|c| !valid_charge(c))
        {
            return Err("区域巡防结算缺少本次战斗、完成或返回证明");
        }
        Ok(())
    }
    pub fn next(&mut self, v: &SettlementView) -> Result<Option<SettlementAction>, &'static str> {
        self.validate()?;
        if self.completed() {
            return Ok(None);
        }
        match v {
            SettlementView::Unknown | SettlementView::Blocked => Ok(None),
            SettlementView::Failed | SettlementView::Other => Err("区域巡防结算进入失败或无关页面"),
            SettlementView::Settlement if self.reward_seen => Err("领奖后返回旧结算"),
            SettlementView::Settlement if self.finish.is_none() => {
                self.settlement_seen = true;
                Ok(Some(SettlementAction::Finish))
            }
            SettlementView::Settlement => Ok(None),
            SettlementView::Reward if self.finish.is_none() => {
                Err("没有本次完成意图，不能采用既有奖励")
            }
            SettlementView::Reward => {
                self.reward_seen = true;
                Ok(self
                    .confirm
                    .is_none()
                    .then_some(SettlementAction::ConfirmReward))
            }
            SettlementView::Returned(c) => {
                if !valid_charge(*c)
                    || self.finish.is_none()
                    || (self.reward_seen && self.confirm.is_none())
                {
                    return Err("区域巡防未完成本次结算或奖励确认");
                }
                self.returned = Some(*c);
                Ok(None)
            }
        }
    }
    fn receipt(&mut self, a: SettlementAction) -> &mut Option<Receipt> {
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
            return Err("区域巡防结算动作不能重复或跨步骤");
        }
        *self.receipt(a) = Some(Receipt::Intent);
        Ok(())
    }
    pub fn allows(&self, a: SettlementAction, v: &SettlementView) -> bool {
        self.validate().is_ok()
            && !self.completed()
            && match (a, v) {
                (SettlementAction::Finish, SettlementView::Settlement) => {
                    self.finish == Some(Receipt::Intent) && !self.reward_seen
                }
                (SettlementAction::ConfirmReward, SettlementView::Reward) => {
                    self.reward_seen && self.confirm == Some(Receipt::Intent)
                }
                _ => false,
            }
    }
    pub fn posted(&mut self, a: SettlementAction) -> Result<(), &'static str> {
        if *self.receipt(a) != Some(Receipt::Intent) {
            return Err("区域巡防结算投递不匹配");
        }
        *self.receipt(a) = Some(Receipt::Posted);
        Ok(())
    }
    pub fn not_posted(&mut self, a: SettlementAction) -> Result<(), &'static str> {
        if *self.receipt(a) != Some(Receipt::Intent) {
            return Err("已投递结算不能撤销");
        }
        *self.receipt(a) = None;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.returned = None;
        Ok(())
    }
}
