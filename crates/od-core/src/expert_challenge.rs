//! 普通专业挑战室的电量、详情与结算；目标来源是本次完整传送，保留独立类别。
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
pub struct ExpertRequest {
    pub plan: ChargePlanItem,
    pub restore_charge: RestoreCharge,
    pub teams: TeamConfig,
    pub timeout_ms: u32,
}
impl ExpertRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        TrainingTransportRequest {
            plan: self.plan.clone(),
            timeout_ms: self.timeout_ms,
        }
        .validate()?;
        if self.plan.category_name != "专业挑战室" || self.plan.level != "默认等级" {
            return Err("专业挑战室仅接通普通副本的默认等级");
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
    charge.validate().is_ok() && charge.required_charge == 40
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpertEntryStage {
    Inspect,
    ModeChange,
    AwaitPopup,
    Restore,
    Verify,
    AwaitDeployment,
    Ready,
    Insufficient,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpertEntryAction {
    DisableBurnout,
    ConfirmBurnoutOff,
    Next,
    OpenRestore,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpertView {
    Unknown,
    Blocked,
    Detail {
        charge: CardsCharge,
        burnout: BurnoutState,
    },
    ConfirmBurnoutOff,
    RestorePopup,
    Deployment,
    Settlement,
    Reward,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BurnoutState {
    On,
    Off,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Receipt {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertEntryFlow {
    request: ExpertRequest,
    route: TrainingTransportReceipt,
    stage: ExpertEntryStage,
    before: Option<CardsCharge>,
    open: Option<Receipt>,
    next: Option<Receipt>,
    restored: bool,
    mode_before: Option<CardsCharge>,
    mode_verified: bool,
    disable: Option<Receipt>,
    confirm_disable: Option<Receipt>,
    mode_confirm_seen: bool,
    unknown_mode_frames: u8,
    verification_required: bool,
}
impl ExpertEntryFlow {
    pub fn new(
        request: ExpertRequest,
        route: TrainingTransportReceipt,
    ) -> Result<Self, &'static str> {
        let flow = Self {
            request,
            route,
            stage: ExpertEntryStage::Inspect,
            before: None,
            open: None,
            next: None,
            restored: false,
            mode_before: None,
            mode_verified: false,
            disable: None,
            confirm_disable: None,
            mode_confirm_seen: false,
            unknown_mode_frames: 0,
            verification_required: false,
        };
        flow.validate()?;
        Ok(flow)
    }
    pub fn request(&self) -> &ExpertRequest {
        &self.request
    }
    pub fn stage(&self) -> ExpertEntryStage {
        self.stage
    }
    pub fn charge(&self) -> Option<CardsCharge> {
        self.before
    }
    pub fn ready(&self) -> bool {
        self.stage == ExpertEntryStage::Ready
    }
    pub fn insufficient(&self) -> bool {
        self.stage == ExpertEntryStage::Insufficient
    }
    pub fn needs_verification(&self) -> bool {
        self.verification_required
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.route.category_name != "专业挑战室"
            || self.route.mission_type != self.request.plan.execution_mission_type()
            || self.route.selection_verified
            || !matches!(self.route.arrival,TrainingArrival::ExpertDetail {current,required} if current<=1_000_000 && (1..=1000).contains(&required))
            || self.before.is_some_and(|c| !valid_charge(c))
            || (self.open.is_some() && self.request.restore_charge == RestoreCharge::None)
            || (self.restored && self.open.is_none())
            || (matches!(
                self.stage,
                ExpertEntryStage::AwaitPopup | ExpertEntryStage::Restore
            ) && self.open.is_none())
            || (matches!(
                self.stage,
                ExpertEntryStage::AwaitDeployment | ExpertEntryStage::Ready
            ) && self.next.is_none())
            || (self.ready() && self.before.is_none_or(|c| c.current_charge < 40))
            || (self.insufficient() && self.before.is_none_or(|c| c.current_charge >= 40))
            || (self.ready() && !self.mode_verified)
            || (self.insufficient() && !self.mode_verified)
            || (self.disable.is_some() && self.mode_before.is_none())
            || self.mode_before.is_some_and(|c| {
                c.current_charge > 1_000_000 || !(1..=1000).contains(&c.required_charge)
            })
            || (self.confirm_disable.is_some()
                && (!self.mode_confirm_seen || self.disable.is_none()))
            || (self.stage == ExpertEntryStage::ModeChange && self.disable.is_none())
            || self.unknown_mode_frames > 3
        {
            return Err("专业挑战室详情缺少本次传送、电量或动作意图");
        }
        Ok(())
    }
    pub fn next(&mut self, view: &ExpertView) -> Result<Option<ExpertEntryAction>, &'static str> {
        self.validate()?;
        use ExpertEntryStage as S;
        if self.ready() || self.insufficient() || self.needs_verification() {
            return Ok(None);
        }
        if matches!(self.stage, S::Inspect | S::Verify | S::ModeChange) {
            match view {
                ExpertView::Detail { charge, burnout } => {
                    if charge.current_charge > 1_000_000
                        || !(1..=1000).contains(&charge.required_charge)
                    {
                        return Err("专业入口电量读数无效");
                    }
                    match burnout {
                        BurnoutState::Unknown => {
                            self.unknown_mode_frames = (self.unknown_mode_frames + 1).min(3);
                            self.verification_required = self.unknown_mode_frames == 3;
                            return Ok(None);
                        }
                        BurnoutState::On => {
                            self.unknown_mode_frames = 0;
                            if self.stage == S::ModeChange {
                                return Ok(None);
                            }
                            if self.disable.is_some() {
                                return Err("已关闭的燃竭模式重新开启，不能重复投递");
                            }
                            self.mode_verified = false;
                            self.before = None;
                            self.mode_before = Some(*charge);
                            return Ok(Some(ExpertEntryAction::DisableBurnout));
                        }
                        BurnoutState::Off => {
                            if !valid_charge(*charge) {
                                self.mode_verified = false;
                                self.unknown_mode_frames = (self.unknown_mode_frames + 1).min(3);
                                self.verification_required = self.unknown_mode_frames == 3;
                                return Ok(None);
                            }
                            if self.mode_confirm_seen && self.confirm_disable.is_none() {
                                return Err("缺少本次燃竭关闭确认意图");
                            }
                            self.mode_verified = true;
                            self.unknown_mode_frames = 0;
                            if self.stage == S::ModeChange {
                                self.stage = if self.restored { S::Verify } else { S::Inspect };
                            }
                        }
                    }
                }
                ExpertView::ConfirmBurnoutOff => {
                    if self.stage != S::ModeChange || self.disable.is_none() {
                        return Err("不能采用已有燃竭确认");
                    }
                    self.mode_confirm_seen = true;
                    return Ok(self
                        .confirm_disable
                        .is_none()
                        .then_some(ExpertEntryAction::ConfirmBurnoutOff));
                }
                _ => (),
            }
        }
        match view {
            ExpertView::Unknown => return Ok(None),
            ExpertView::Blocked
            | ExpertView::Settlement
            | ExpertView::Reward
            | ExpertView::Failed => return Err("专业挑战室详情出现未支持的弹窗、结果或战斗画面"),
            _ => (),
        }
        match self.stage {
            S::Inspect | S::Verify => match view {
                ExpertView::Detail {
                    charge: c,
                    burnout: BurnoutState::Off,
                } if valid_charge(*c) && self.mode_verified => {
                    self.before = Some(*c);
                    if c.current_charge >= 40 {
                        Ok(Some(ExpertEntryAction::Next))
                    } else if self.request.restore_charge == RestoreCharge::None || self.restored {
                        self.stage = S::Insufficient;
                        Ok(None)
                    } else {
                        Ok(Some(ExpertEntryAction::OpenRestore))
                    }
                }
                _ => Err("没有本次下一步意图，不能采用已有出战页或恢复弹窗"),
            },
            S::AwaitPopup => match view {
                ExpertView::RestorePopup => {
                    self.stage = S::Restore;
                    Ok(None)
                }
                ExpertView::Deployment => Err("恢复意图后意外进入出战页，不能视为足额准备"),
                _ => Ok(None),
            },
            S::AwaitDeployment => match view {
                ExpertView::Deployment => {
                    self.stage = S::Ready;
                    Ok(None)
                }
                ExpertView::RestorePopup => Err("足额下一步后出现恢复弹窗，需要重新核对资源"),
                _ => Ok(None),
            },
            S::Restore | S::ModeChange => Ok(None),
            S::Ready | S::Insufficient => Ok(None),
        }
    }
    pub fn prepare(&mut self, a: ExpertEntryAction) -> Result<(), &'static str> {
        self.validate()?;
        match a {
            ExpertEntryAction::DisableBurnout => {
                if !matches!(
                    self.stage,
                    ExpertEntryStage::Inspect | ExpertEntryStage::Verify
                ) || self.mode_before.is_none()
                    || self.disable.is_some()
                {
                    return Err("没有待保存的燃竭关闭动作");
                }
                self.disable = Some(Receipt::Intent);
                self.stage = ExpertEntryStage::ModeChange;
                return self.validate();
            }
            ExpertEntryAction::ConfirmBurnoutOff => {
                if self.stage != ExpertEntryStage::ModeChange
                    || !self.mode_confirm_seen
                    || self.disable.is_none()
                    || self.confirm_disable.is_some()
                {
                    return Err("没有待保存的燃竭确认动作");
                }
                self.confirm_disable = Some(Receipt::Intent);
                return self.validate();
            }
            _ => (),
        }
        if !self.mode_verified {
            return Err("燃竭模式尚无明确关闭后态");
        }
        if !matches!(
            self.stage,
            ExpertEntryStage::Inspect | ExpertEntryStage::Verify
        ) {
            return Err("专业挑战室没有待保存的详情动作");
        }
        let c = self.before.ok_or("专业挑战室电量未知")?;
        match a {
            ExpertEntryAction::Next if c.current_charge >= 40 && self.next.is_none() => {
                self.next = Some(Receipt::Intent);
                self.stage = ExpertEntryStage::AwaitDeployment;
            }
            ExpertEntryAction::OpenRestore
                if c.current_charge < 40
                    && self.open.is_none()
                    && self.request.restore_charge != RestoreCharge::None =>
            {
                self.open = Some(Receipt::Intent);
                self.stage = ExpertEntryStage::AwaitPopup;
            }
            _ => return Err("专业挑战室详情动作与实际电量不一致"),
        };
        Ok(())
    }
    pub fn allows(&self, a: ExpertEntryAction, v: &ExpertView) -> bool {
        self.validate().is_ok()
            && match a {
                ExpertEntryAction::DisableBurnout => {
                    self.stage == ExpertEntryStage::ModeChange
                        && self.disable == Some(Receipt::Intent)
                        && matches!(v,ExpertView::Detail {charge,burnout:BurnoutState::On} if Some(*charge)==self.mode_before)
                }
                ExpertEntryAction::ConfirmBurnoutOff => {
                    self.stage == ExpertEntryStage::ModeChange
                        && self.confirm_disable == Some(Receipt::Intent)
                        && *v == ExpertView::ConfirmBurnoutOff
                }
                ExpertEntryAction::Next => {
                    self.stage == ExpertEntryStage::AwaitDeployment
                        && self.next == Some(Receipt::Intent)
                        && self.mode_verified
                        && matches!(v,ExpertView::Detail{charge,burnout:BurnoutState::Off} if Some(*charge)==self.before)
                }
                ExpertEntryAction::OpenRestore => {
                    self.stage == ExpertEntryStage::AwaitPopup
                        && self.open == Some(Receipt::Intent)
                        && self.mode_verified
                        && matches!(v,ExpertView::Detail{charge,burnout:BurnoutState::Off} if Some(*charge)==self.before)
                }
            }
    }
    fn receipt(&mut self, a: ExpertEntryAction) -> &mut Option<Receipt> {
        match a {
            ExpertEntryAction::DisableBurnout => &mut self.disable,
            ExpertEntryAction::ConfirmBurnoutOff => &mut self.confirm_disable,
            ExpertEntryAction::Next => &mut self.next,
            ExpertEntryAction::OpenRestore => &mut self.open,
        }
    }
    pub fn posted(&mut self, a: ExpertEntryAction) -> Result<(), &'static str> {
        if *self.receipt(a) != Some(Receipt::Intent) {
            return Err("专业挑战室详情投递不匹配");
        }
        *self.receipt(a) = Some(Receipt::Posted);
        self.validate()
    }
    pub fn not_posted(&mut self, a: ExpertEntryAction) -> Result<(), &'static str> {
        if *self.receipt(a) != Some(Receipt::Intent) {
            return Err("已投递专业挑战室详情不能撤销");
        }
        *self.receipt(a) = None;
        if a == ExpertEntryAction::ConfirmBurnoutOff {
            return self.validate();
        }
        if a == ExpertEntryAction::DisableBurnout {
            self.mode_before = None;
        }
        self.mode_verified = false;
        self.before = None;
        self.stage = if self.restored {
            ExpertEntryStage::Verify
        } else {
            ExpertEntryStage::Inspect
        };
        self.validate()
    }
    pub fn restore_request(&self) -> Result<RestoreRequest, &'static str> {
        self.validate()?;
        if self.stage != ExpertEntryStage::Restore {
            return Err("专业挑战室尚未见到本次恢复主弹窗");
        }
        let c = self.before.ok_or("专业挑战室电量未知")?;
        let r = RestoreRequest {
            policy: self.request.restore_charge,
            current_charge: c.current_charge,
            required_charge: 40,
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
            return Err("专业挑战室恢复子流程或返回证明不一致");
        }
        self.restored = true;
        self.mode_verified = false;
        self.before = None;
        self.stage = ExpertEntryStage::Verify;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.unknown_mode_frames = 0;
        self.verification_required = false;
        if matches!(
            self.stage,
            ExpertEntryStage::Inspect | ExpertEntryStage::Verify | ExpertEntryStage::ModeChange
        ) {
            self.mode_verified = false;
            self.before = None;
        }
        if self.ready() {
            self.stage = ExpertEntryStage::AwaitDeployment;
        } else if self.insufficient() {
            self.stage = ExpertEntryStage::Verify;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertDeployRequest {
    pub entry: ExpertEntryFlow,
    pub team: TeamsResult,
    pub timeout_ms: u32,
}
impl DeploymentContext for ExpertDeployRequest {
    fn validate(&self) -> Result<(), &'static str> {
        self.entry.validate()?;
        let expected = self.entry.request.team_request()?;
        if !self.entry.ready()
            || !(1000..=900_000).contains(&self.timeout_ms)
            || self.team.predefined_team_idx != expected.predefined_team_idx
            || self.team.script_name != expected.script_name
            || self.team.selection_verified
        {
            return Err("专业挑战室出战缺少本次详情或冻结选队证明");
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
pub struct ExpertSettlementFlow {
    battle: NormalBattleResult,
    timeout_ms: u32,
    finish: Option<Receipt>,
    settlement_seen: bool,
    reward_seen: bool,
    confirm: Option<Receipt>,
    returned: Option<CardsCharge>,
}
impl ExpertSettlementFlow {
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
            return Err("专业挑战室结算缺少本次战斗、完成或返回证明");
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
            SettlementView::Failed | SettlementView::Other => {
                Err("专业挑战室结算进入失败或无关页面")
            }
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
                    return Err("专业挑战室未完成本次结算或奖励确认");
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
            return Err("专业挑战室结算动作不能重复或跨步骤");
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
            return Err("专业挑战室结算投递不匹配");
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
