//! 模拟室单次战斗后的结算收尾；不登记次数，也不自动开始下一战。
use crate::{battle_result::NormalBattleResult, simulation_cards::CardsCharge};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementView {
    Unknown,
    Blocked,
    Settlement,
    Reward,
    Returned(CardsCharge),
    Failed,
    Other,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementAction {
    Finish,
    ConfirmReward,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Receipt {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementFlow {
    battle_result: NormalBattleResult,
    timeout_ms: u32,
    settlement_seen: bool,
    finish: Option<Receipt>,
    reward_seen: bool,
    confirm: Option<Receipt>,
    returned: Option<CardsCharge>,
}
impl SettlementFlow {
    /// 由同一父任务的已释放输入的战斗子流程调用。
    pub fn new(battle_result: NormalBattleResult, timeout_ms: u32) -> Result<Self, &'static str> {
        let flow = Self {
            battle_result,
            timeout_ms,
            settlement_seen: false,
            finish: None,
            reward_seen: false,
            confirm: None,
            returned: None,
        };
        flow.validate()?;
        Ok(flow)
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
        if self.battle_result != NormalBattleResult::Completed
            || !(1000..=300_000).contains(&self.timeout_ms)
            || (self.finish.is_some() && !self.settlement_seen)
            || (self.reward_seen && self.finish.is_none())
            || (self.confirm.is_some() && !self.reward_seen)
            || (self.returned.is_some() && self.confirm.is_none())
        {
            return Err("模拟室结算检查点或战斗结果无效");
        }
        if let Some(charge) = self.returned {
            charge.validate()?;
        }
        Ok(())
    }
    pub fn next(
        &mut self,
        view: &SettlementView,
    ) -> Result<Option<SettlementAction>, &'static str> {
        self.validate()?;
        if self.completed() {
            return Ok(None);
        }
        match view {
            SettlementView::Unknown | SettlementView::Blocked => Ok(None),
            SettlementView::Failed | SettlementView::Other => Err("模拟室结算进入失败或无关画面"),
            SettlementView::Settlement if self.finish.is_none() => {
                self.settlement_seen = true;
                Ok(Some(SettlementAction::Finish))
            }
            SettlementView::Settlement if self.reward_seen => Err("领奖后意外返回战斗结算"),
            SettlementView::Settlement => Ok(None),
            SettlementView::Reward if self.finish.is_none() => {
                Err("没有本次完成意图，不能收取既有奖励")
            }
            SettlementView::Reward => {
                self.reward_seen = true;
                Ok(self
                    .confirm
                    .is_none()
                    .then_some(SettlementAction::ConfirmReward))
            }
            SettlementView::Returned(charge) => {
                charge.validate()?;
                if self.confirm.is_none() {
                    return Err("未确认本次奖励，不能将副本详情当作领取完成");
                }
                self.returned = Some(*charge);
                Ok(None)
            }
        }
    }
    pub fn allows(&self, action: SettlementAction, view: &SettlementView) -> bool {
        self.validate().is_ok()
            && !self.completed()
            && match (action, view) {
                (SettlementAction::Finish, SettlementView::Settlement) => {
                    self.finish == Some(Receipt::Intent) && !self.reward_seen
                }
                (SettlementAction::ConfirmReward, SettlementView::Reward) => {
                    self.reward_seen && self.confirm == Some(Receipt::Intent)
                }
                _ => false,
            }
    }
    pub fn prepare(&mut self, action: SettlementAction) -> Result<(), &'static str> {
        self.validate()?;
        match action {
            SettlementAction::Finish
                if self.settlement_seen && self.finish.is_none() && !self.completed() =>
            {
                self.finish = Some(Receipt::Intent);
            }
            SettlementAction::ConfirmReward
                if self.reward_seen && self.confirm.is_none() && !self.completed() =>
            {
                self.confirm = Some(Receipt::Intent);
            }
            _ => return Err("结算输入不能重复准备或跨步骤提交"),
        }
        self.validate()
    }
    fn receipt(&mut self, action: SettlementAction) -> &mut Option<Receipt> {
        match action {
            SettlementAction::Finish => &mut self.finish,
            SettlementAction::ConfirmReward => &mut self.confirm,
        }
    }
    pub fn posted(&mut self, action: SettlementAction) -> Result<(), &'static str> {
        self.validate()?;
        if *self.receipt(action) != Some(Receipt::Intent) {
            return Err("结算投递缺少对应意图");
        }
        *self.receipt(action) = Some(Receipt::Posted);
        self.validate()
    }
    pub fn not_posted(&mut self, action: SettlementAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.reward_seen && action == SettlementAction::Finish
            || self.completed()
            || *self.receipt(action) != Some(Receipt::Intent)
        {
            return Err("只能撤销明确未投递的当前结算意图");
        }
        *self.receipt(action) = None;
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.returned = None;
        // 不确定意图继续保留，等新画面确认后态，不能重新点击。
        Ok(())
    }
}
