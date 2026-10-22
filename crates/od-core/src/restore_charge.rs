//! 副本恢复电量：消费与确认分别保存意图，返回证明由当前副本提供。
use crate::charge_plan::RestoreCharge;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreSource {
    Backup,
    Ether,
}
impl RestoreSource {
    pub fn name(self) -> &'static str {
        match self {
            Self::Backup => "储蓄电量",
            Self::Ether => "以太电池",
        }
    }
    pub fn quantity(self, deficit: u32) -> u32 {
        match self {
            Self::Backup => deficit,
            Self::Ether => deficit.div_ceil(60),
        }
    }
    pub fn gained(self, quantity: u32) -> u32 {
        match self {
            Self::Backup => quantity,
            Self::Ether => quantity.saturating_mul(60),
        }
    }
    fn allowed(self, policy: RestoreCharge) -> bool {
        policy == RestoreCharge::Both
            || matches!(
                (self, policy),
                (Self::Backup, RestoreCharge::BackupOnly) | (Self::Ether, RestoreCharge::EtherOnly)
            )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreRequest {
    pub policy: RestoreCharge,
    pub current_charge: u32,
    pub required_charge: u32,
    pub timeout_ms: u32,
}
impl RestoreRequest {
    pub fn validate(self) -> Result<(), &'static str> {
        if self.current_charge >= self.required_charge
            || self.required_charge > 240
            || !(1000..=300_000).contains(&self.timeout_ms)
        {
            return Err("恢复电量须冻结有效缺口和 1000..300000ms 超时");
        }
        Ok(())
    }
}

/// 仅由父流程在同帧明确识别无遮挡返回画面后提供，不能从弹窗消失推断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RestoreReturn {
    pub current_charge: u32,
    pub required_charge: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RestoreView {
    Unknown,
    UnreadablePopup,
    Sources {
        backup: u32,
        ether: u32,
        selected: Option<RestoreSource>,
    },
    QuickUse {
        source: RestoreSource,
        owned: u32,
        quantity: u32,
    },
    Reward {
        charge: u32,
    },
    Returned(RestoreReturn),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreSpend {
    pub source: RestoreSource,
    pub owned: u32,
    pub quantity: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreAction {
    Select(RestoreSource),
    OpenQuick(RestoreSource),
    Submit(RestoreSpend),
    ConfirmReward,
    CancelSources,
    CloseQuick,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Select,
    Quick,
    AwaitRestore,
    AwaitReturn,
    CancelQuick,
    CancelSources,
    Return,
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
pub enum RestoreOutcome {
    Pending,
    Restored,
    Disabled,
    Insufficient,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreFlow {
    request: RestoreRequest,
    phase: Phase,
    source: Option<RestoreSource>,
    spend: Option<RestoreSpend>,
    submit: Option<Receipt>,
    reward_seen: bool,
    reward_confirm: Option<Receipt>,
    outcome: RestoreOutcome,
    charge_after: Option<u32>,
}
impl RestoreFlow {
    pub fn new(request: RestoreRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            phase: Phase::Select,
            source: None,
            spend: None,
            submit: None,
            reward_seen: false,
            reward_confirm: None,
            outcome: RestoreOutcome::Pending,
            charge_after: None,
        })
    }
    pub fn request(&self) -> RestoreRequest {
        self.request
    }
    pub fn outcome(&self) -> RestoreOutcome {
        self.outcome
    }
    pub fn spend(&self) -> Option<RestoreSpend> {
        self.spend
    }
    pub fn charge_after(&self) -> Option<u32> {
        self.charge_after
    }
    pub fn completed(&self) -> bool {
        self.phase == Phase::Done
    }
    fn valid_spend(&self, s: RestoreSpend) -> bool {
        s.source.allowed(self.request.policy)
            && s.quantity
                == s.source
                    .quantity(self.request.required_charge - self.request.current_charge)
            && s.quantity > 0
            && s.quantity < s.owned
            && s.owned <= 1_000_000
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.source.is_some_and(|s| !s.allowed(self.request.policy))
            || self
                .spend
                .is_some_and(|s| !self.valid_spend(s) || self.source != Some(s.source))
            || self.spend.is_some() != self.submit.is_some()
            || (self.reward_seen && self.spend.is_none())
            || (self.reward_confirm.is_some() && !self.reward_seen)
            || (matches!(self.phase, Phase::AwaitRestore | Phase::AwaitReturn)
                && self.spend.is_none())
            || (self.phase == Phase::AwaitReturn && self.reward_confirm.is_none())
            || (self.phase == Phase::AwaitRestore && self.reward_confirm.is_some())
            || (self.phase == Phase::Quick && self.source.is_none())
            || (matches!(
                self.phase,
                Phase::Select | Phase::Quick | Phase::AwaitRestore | Phase::AwaitReturn
            ) && self.outcome != RestoreOutcome::Pending)
            || (matches!(
                self.phase,
                Phase::CancelQuick | Phase::CancelSources | Phase::Return
            ) && !matches!(
                self.outcome,
                RestoreOutcome::Disabled | RestoreOutcome::Insufficient
            ))
            || (self.phase == Phase::CancelQuick && self.source.is_none())
            || (matches!(
                self.phase,
                Phase::Select
                    | Phase::Quick
                    | Phase::CancelQuick
                    | Phase::CancelSources
                    | Phase::Return
            ) && self.spend.is_some())
            || (matches!(
                self.outcome,
                RestoreOutcome::Disabled | RestoreOutcome::Insufficient
            ) && self.spend.is_some())
            || (self.outcome == RestoreOutcome::Disabled
                && self.request.policy != RestoreCharge::None)
            || (self.outcome == RestoreOutcome::Restored
                && (!self.completed() || self.spend.is_none()))
            || (self.completed()
                && (self.outcome == RestoreOutcome::Pending || self.charge_after.is_none()))
            || (!self.completed() && self.charge_after.is_some())
            || self.charge_after.is_some_and(|n| {
                n > 1_000_000
                    || n < self.request.current_charge
                    || (self.spend.is_some_and(|s| {
                        n < self.request.current_charge + s.source.gained(s.quantity)
                    }))
            })
        {
            return Err("恢复电量检查点不一致");
        }
        Ok(())
    }
    fn choose(&self, backup: u32, ether: u32) -> Option<RestoreSource> {
        [
            (RestoreSource::Backup, backup),
            (RestoreSource::Ether, ether),
        ]
        .into_iter()
        .find(|(s, n)| {
            s.allowed(self.request.policy)
                && *n <= 1_000_000
                && *n > s.quantity(self.request.required_charge - self.request.current_charge)
        })
        .map(|(s, _)| s)
    }
    pub fn next(&mut self, view: &RestoreView) -> Result<Option<RestoreAction>, &'static str> {
        self.validate()?;
        if self.completed() {
            return Ok(None);
        }
        if let RestoreView::Returned(v) = view {
            if v.required_charge != self.request.required_charge || v.current_charge > 1_000_000 {
                return Err("恢复返回画面与本次副本需求不符");
            }
            if matches!(self.phase, Phase::AwaitRestore | Phase::AwaitReturn) {
                let s = self.spend.ok_or("缺少本次消费意图")?;
                if v.current_charge < self.request.current_charge + s.source.gained(s.quantity) {
                    return Ok(None);
                }
                self.outcome = RestoreOutcome::Restored;
            } else if !matches!(self.phase, Phase::Return)
                || v.current_charge < self.request.current_charge
            {
                return Ok(None);
            }
            self.charge_after = Some(v.current_charge);
            self.phase = Phase::Done;
            self.validate()?;
            return Ok(None);
        }
        match self.phase {
            Phase::Select => {
                if let RestoreView::Sources {
                    backup,
                    ether,
                    selected,
                } = view
                {
                    if *backup > 1_000_000 || *ether > 1_000_000 {
                        return Ok(None);
                    }
                    self.source = self.choose(*backup, *ether);
                    if let Some(source) = self.source {
                        return Ok(Some(if *selected == Some(source) {
                            RestoreAction::OpenQuick(source)
                        } else {
                            RestoreAction::Select(source)
                        }));
                    }
                    self.outcome = if self.request.policy == RestoreCharge::None {
                        RestoreOutcome::Disabled
                    } else {
                        RestoreOutcome::Insufficient
                    };
                    self.phase = Phase::CancelSources;
                    return Ok(Some(RestoreAction::CancelSources));
                }
            }
            Phase::Quick => {
                if let RestoreView::QuickUse {
                    source,
                    owned,
                    quantity,
                } = view
                {
                    if self.source != Some(*source) {
                        return Ok(None);
                    }
                    let s = RestoreSpend {
                        source: *source,
                        owned: *owned,
                        quantity: *quantity,
                    };
                    if self.valid_spend(s) {
                        return Ok(Some(RestoreAction::Submit(s)));
                    }
                    self.outcome = RestoreOutcome::Insufficient;
                    self.phase = Phase::CancelQuick;
                    return Ok(Some(RestoreAction::CloseQuick));
                }
            }
            Phase::AwaitRestore => {
                if let (Some(s), RestoreView::Reward { charge }) = (self.spend, view)
                    && *charge == s.source.gained(s.quantity)
                {
                    self.reward_seen = true;
                    return Ok(Some(RestoreAction::ConfirmReward));
                }
            }
            Phase::CancelSources if matches!(view, RestoreView::Sources { .. }) => {
                return Ok(Some(RestoreAction::CancelSources));
            }
            Phase::CancelQuick if matches!(view, RestoreView::QuickUse { .. }) => {
                return Ok(Some(RestoreAction::CloseQuick));
            }
            _ => {}
        }
        Ok(None)
    }
    /// 在外部持久保存成功后才能调用原生输入。
    pub fn prepare(&mut self, action: &RestoreAction) -> Result<(), &'static str> {
        self.validate()?;
        match action {
            RestoreAction::Select(s) if self.phase == Phase::Select && self.source == Some(*s) => {}
            RestoreAction::OpenQuick(s)
                if self.phase == Phase::Select && self.source == Some(*s) =>
            {
                self.phase = Phase::Quick
            }
            RestoreAction::Submit(s)
                if self.phase == Phase::Quick
                    && self.source == Some(s.source)
                    && self.valid_spend(*s) =>
            {
                self.spend = Some(*s);
                self.submit = Some(Receipt::Intent);
                self.phase = Phase::AwaitRestore;
            }
            RestoreAction::ConfirmReward
                if self.phase == Phase::AwaitRestore && self.reward_seen =>
            {
                self.reward_confirm = Some(Receipt::Intent);
                self.phase = Phase::AwaitReturn;
            }
            RestoreAction::CancelSources if self.phase == Phase::CancelSources => {
                self.phase = Phase::Return
            }
            RestoreAction::CloseQuick if self.phase == Phase::CancelQuick => {
                self.phase = Phase::Return
            }
            _ => return Err("恢复电量动作不属于当前步骤"),
        }
        self.validate()
    }
    pub fn allows(&self, action: &RestoreAction, view: &RestoreView) -> bool {
        if self.validate().is_err() {
            return false;
        }
        match (action, view) {
            (RestoreAction::Select(s), RestoreView::Sources { backup, ether, .. }) => {
                self.phase == Phase::Select
                    && self.source == Some(*s)
                    && self.choose(*backup, *ether) == Some(*s)
            }
            (
                RestoreAction::OpenQuick(s),
                RestoreView::Sources {
                    backup,
                    ether,
                    selected,
                },
            ) => {
                self.phase == Phase::Quick
                    && self.source == Some(*s)
                    && *selected == Some(*s)
                    && self.choose(*backup, *ether) == Some(*s)
            }
            (
                RestoreAction::Submit(s),
                RestoreView::QuickUse {
                    source,
                    owned,
                    quantity,
                },
            ) => {
                self.phase == Phase::AwaitRestore
                    && self.submit == Some(Receipt::Intent)
                    && self.spend == Some(*s)
                    && s.source == *source
                    && s.owned == *owned
                    && s.quantity == *quantity
            }
            (RestoreAction::ConfirmReward, RestoreView::Reward { charge }) => {
                self.phase == Phase::AwaitReturn
                    && self.reward_confirm == Some(Receipt::Intent)
                    && self
                        .spend
                        .is_some_and(|s| *charge == s.source.gained(s.quantity))
            }
            (RestoreAction::CancelSources, RestoreView::Sources { .. }) => {
                self.phase == Phase::Return && self.outcome != RestoreOutcome::Pending
            }
            (RestoreAction::CloseQuick, RestoreView::QuickUse { source, .. }) => {
                self.phase == Phase::Return
                    && self.outcome == RestoreOutcome::Insufficient
                    && self.source == Some(*source)
            }
            _ => false,
        }
    }
    pub fn posted(&mut self, action: &RestoreAction) -> Result<(), &'static str> {
        match action {
            RestoreAction::Submit(s)
                if self.spend == Some(*s) && self.submit == Some(Receipt::Intent) =>
            {
                self.submit = Some(Receipt::Posted)
            }
            RestoreAction::ConfirmReward if self.reward_confirm == Some(Receipt::Intent) => {
                self.reward_confirm = Some(Receipt::Posted)
            }
            _ => {}
        }
        self.validate()
    }
    /// 仅在适配层明确证明没有发送鼠标按下时回退；异常不能调用。
    pub fn not_posted(&mut self, action: &RestoreAction) -> Result<(), &'static str> {
        match action {
            RestoreAction::OpenQuick(s)
                if self.phase == Phase::Quick && self.source == Some(*s) =>
            {
                self.phase = Phase::Select
            }
            RestoreAction::Submit(s)
                if self.phase == Phase::AwaitRestore
                    && self.spend == Some(*s)
                    && self.submit == Some(Receipt::Intent)
                    && !self.reward_seen =>
            {
                self.phase = Phase::Quick;
                self.spend = None;
                self.submit = None;
            }
            RestoreAction::ConfirmReward if self.reward_confirm == Some(Receipt::Intent) => {
                self.phase = Phase::AwaitRestore;
                self.reward_confirm = None;
            }
            RestoreAction::CancelSources if self.phase == Phase::Return => {
                self.phase = Phase::CancelSources
            }
            RestoreAction::CloseQuick if self.phase == Phase::Return => {
                self.phase = Phase::CancelQuick
            }
            RestoreAction::Select(_) => {}
            _ => return Err("无法撤回已投递的恢复动作"),
        }
        self.validate()
    }
}
