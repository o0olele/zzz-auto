//! 串联卡片与电量恢复，只证明数量与电量，不提供关卡选中证明。
use crate::{
    charge_plan::{ChargePlanItem, RestoreCharge},
    restore_charge::{RestoreFlow, RestoreOutcome, RestoreRequest},
    simulation_cards::{CardsCharge, CardsFlow, CardsRequest},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationRequest {
    pub card_num: Option<u8>,
    pub restore_charge: RestoreCharge,
    pub timeout_ms: u32,
}
impl PreparationRequest {
    pub fn validate(self) -> Result<(), &'static str> {
        self.cards().validate()
    }
    pub fn cards(self) -> CardsRequest {
        CardsRequest {
            card_num: self.card_num,
            timeout_ms: self.timeout_ms,
        }
    }
    pub fn from_plan(
        plan: &ChargePlanItem,
        restore_charge: RestoreCharge,
        timeout_ms: u32,
    ) -> Result<Self, &'static str> {
        crate::simulation_mission::MissionRequest::from_plan(plan, timeout_ms)?;
        if plan.level != "默认等级" {
            return Err("模拟室非默认等级仍缺少画面依据");
        }
        let request = Self {
            card_num: plan.card_num,
            restore_charge,
            timeout_ms,
        };
        request.validate()?;
        Ok(request)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationStage {
    Cards,
    Inspect,
    AwaitPopup,
    Restore,
    Verify,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Receipt {
    Intent,
    Posted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationOutcome {
    Ready,
    Disabled,
    Insufficient,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationResult {
    pub outcome: PreparationOutcome,
    pub charge: CardsCharge,
    pub selection_verified: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationFlow {
    request: PreparationRequest,
    stage: PreparationStage,
    cards: CardsFlow,
    before: Option<CardsCharge>,
    next: Option<Receipt>,
    restore: Option<RestoreFlow>,
    result: Option<PreparationResult>,
}
impl PreparationFlow {
    pub fn new(request: PreparationRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            stage: PreparationStage::Cards,
            cards: CardsFlow::new(request.cards())?,
            before: None,
            next: None,
            restore: None,
            result: None,
        })
    }
    pub fn request(&self) -> PreparationRequest {
        self.request
    }
    pub fn stage(&self) -> PreparationStage {
        self.stage
    }
    pub fn cards(&self) -> &CardsFlow {
        &self.cards
    }
    pub fn restore(&self) -> Option<&RestoreFlow> {
        self.restore.as_ref()
    }
    pub fn before(&self) -> Option<CardsCharge> {
        self.before
    }
    pub fn result(&self) -> Option<PreparationResult> {
        self.result
    }
    pub fn completed(&self) -> bool {
        self.stage == PreparationStage::Done
    }
    fn check_charge(&self, charge: CardsCharge) -> Result<(), &'static str> {
        charge.validate()?;
        if self
            .cards
            .charge()
            .is_none_or(|c| c.required_charge != charge.required_charge)
        {
            return Err("实际所需电量与本次卡片方案不符");
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        use PreparationStage as S;
        self.request.validate()?;
        self.cards.validate()?;
        if self.cards.request() != self.request.cards()
            || (self.stage != S::Cards && !self.cards.completed())
            || (self.completed() != self.result.is_some())
            || (matches!(self.stage, S::Cards | S::Inspect)
                && (self.next.is_some() || self.restore.is_some()))
            || (self.stage == S::Cards && self.before.is_some())
            || (matches!(self.stage, S::AwaitPopup | S::Restore | S::Verify) && self.next.is_none())
            || (self.next.is_some() && self.before.is_none())
            || (self.stage == S::AwaitPopup && self.restore.is_some())
            || (matches!(self.stage, S::Restore | S::Verify) && self.restore.is_none())
            || (self.restore.is_some() && self.next.is_none())
        {
            return Err("模拟室准备阶段或子流程检查点不一致");
        }
        if let Some(before) = self.before {
            self.check_charge(before)?;
            if self.next.is_some()
                && (before.current_charge >= before.required_charge
                    || self.request.restore_charge == RestoreCharge::None)
            {
                return Err("只有允许恢复的不足电量可以打开弹窗");
            }
        }
        if let Some(restore) = &self.restore {
            restore.validate()?;
            let before = self.before.ok_or("缺少冻结电量")?;
            if restore.request() != self.restore_request(before)
                || (matches!(self.stage, S::Verify | S::Done) && !restore.completed())
            {
                return Err("恢复子流程改写了冻结缺口或来源");
            }
        }
        if let Some(result) = self.result {
            self.check_charge(result.charge)?;
            if result.selection_verified {
                return Err("数量与电量不能代替关卡选中证明");
            }
            let expected = if let Some(restore) = &self.restore {
                if result.charge.current_charge
                    < restore.charge_after().ok_or("缺少恢复返回电量")?
                {
                    return Err("恢复返回后电量异常减少");
                }
                match restore.outcome() {
                    RestoreOutcome::Restored => PreparationOutcome::Ready,
                    RestoreOutcome::Insufficient => PreparationOutcome::Insufficient,
                    _ => return Err("恢复子流程尚未提供明确结果"),
                }
            } else {
                if self.next.is_some() || self.before != Some(result.charge) {
                    return Err("无恢复结果缺少本次实际电量");
                }
                if result.charge.current_charge >= result.charge.required_charge {
                    PreparationOutcome::Ready
                } else if self.request.restore_charge == RestoreCharge::None {
                    PreparationOutcome::Disabled
                } else {
                    return Err("允许恢复但不足的电量不能直接结束");
                }
            };
            if result.outcome != expected
                || (expected == PreparationOutcome::Ready
                    && result.charge.current_charge < result.charge.required_charge)
            {
                return Err("准备结果与实际电量或恢复结果不符");
            }
        }
        Ok(())
    }
    fn restore_request(&self, before: CardsCharge) -> RestoreRequest {
        RestoreRequest {
            policy: self.request.restore_charge,
            current_charge: before.current_charge,
            required_charge: before.required_charge,
            timeout_ms: self.request.timeout_ms.min(300_000),
        }
    }
    pub fn update_cards(&mut self, cards: CardsFlow) -> Result<(), &'static str> {
        if self.stage != PreparationStage::Cards || cards.request() != self.request.cards() {
            return Err("卡片检查点不属于当前准备任务");
        }
        cards.validate()?;
        self.cards = cards;
        self.validate()
    }
    pub fn cards_finished(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.stage != PreparationStage::Cards || !self.cards.completed() {
            return Err("卡片尚未完成");
        }
        self.stage = PreparationStage::Inspect;
        self.validate()
    }
    /// true 表示需要准备打开恢复弹窗；实际动作仍须先保存意图并重新读帧。
    pub fn observe_details(&mut self, charge: CardsCharge) -> Result<bool, &'static str> {
        self.validate()?;
        self.check_charge(charge)?;
        match self.stage {
            PreparationStage::Inspect => {
                self.before = Some(charge);
                let outcome = if charge.current_charge >= charge.required_charge {
                    PreparationOutcome::Ready
                } else if self.request.restore_charge == RestoreCharge::None {
                    PreparationOutcome::Disabled
                } else {
                    return Ok(true);
                };
                self.result = Some(PreparationResult {
                    outcome,
                    charge,
                    selection_verified: false,
                });
            }
            PreparationStage::Verify => {
                let restore = self.restore.as_ref().ok_or("缺少恢复检查点")?;
                if charge.current_charge < restore.charge_after().ok_or("缺少恢复实际电量")?
                {
                    return Err("恢复返回后电量异常减少");
                }
                let outcome = match restore.outcome() {
                    RestoreOutcome::Restored => PreparationOutcome::Ready,
                    RestoreOutcome::Insufficient => PreparationOutcome::Insufficient,
                    _ => return Err("恢复没有明确结果"),
                };
                self.result = Some(PreparationResult {
                    outcome,
                    charge,
                    selection_verified: false,
                });
            }
            _ => return Err("此阶段不能采用副本详情作为准备结果"),
        }
        self.stage = PreparationStage::Done;
        self.validate()?;
        Ok(false)
    }
    pub fn prepare_open(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.stage != PreparationStage::Inspect
            || self
                .before
                .is_none_or(|c| c.current_charge >= c.required_charge)
            || self.request.restore_charge == RestoreCharge::None
        {
            return Err("不能打开恢复弹窗");
        }
        self.stage = PreparationStage::AwaitPopup;
        self.next = Some(Receipt::Intent);
        self.validate()
    }
    pub fn allows_open(&self, charge: CardsCharge) -> bool {
        self.validate().is_ok()
            && self.stage == PreparationStage::AwaitPopup
            && self.next == Some(Receipt::Intent)
            && self.before == Some(charge)
    }
    pub fn posted_open(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.stage != PreparationStage::AwaitPopup || self.next != Some(Receipt::Intent) {
            return Err("下一步投递缺少对应意图");
        }
        self.next = Some(Receipt::Posted);
        Ok(())
    }
    pub fn not_posted_open(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.stage != PreparationStage::AwaitPopup || self.next != Some(Receipt::Intent) {
            return Err("不能撤回可能已投递的下一步");
        }
        self.next = None;
        self.before = None;
        self.stage = PreparationStage::Inspect;
        self.validate()
    }
    /// 仅在读到实际恢复主弹窗后调用；详情或弹窗消失不能触发本步骤。
    pub fn start_restore(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.stage != PreparationStage::AwaitPopup {
            return Err("尚未打开本次恢复弹窗");
        }
        self.restore = Some(RestoreFlow::new(
            self.restore_request(self.before.ok_or("缺少冻结电量")?),
        )?);
        self.stage = PreparationStage::Restore;
        self.validate()
    }
    pub fn update_restore(&mut self, flow: RestoreFlow) -> Result<(), &'static str> {
        if self.stage != PreparationStage::Restore
            || self
                .restore
                .as_ref()
                .is_none_or(|r| r.request() != flow.request())
        {
            return Err("恢复检查点不属于当前准备任务");
        }
        flow.validate()?;
        self.restore = Some(flow);
        self.validate()
    }
    pub fn restore_finished(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.stage != PreparationStage::Restore
            || self.restore.as_ref().is_none_or(|r| !r.completed())
        {
            return Err("恢复尚未完成");
        }
        self.stage = PreparationStage::Verify;
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.completed() {
            self.result = None;
            self.stage = if self.restore.is_some() {
                PreparationStage::Verify
            } else {
                PreparationStage::Inspect
            };
        }
        if self.stage == PreparationStage::Cards {
            self.cards.resume()?;
        }
        if self.stage == PreparationStage::Inspect {
            self.before = None;
        }
        self.validate()
    }
}
