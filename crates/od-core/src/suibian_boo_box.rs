//! 邦巢的价格筛选、聘用和刷新均保存本次意图，点击不计作已获得。
use crate::{suibian_adventure::CommitState, suibian_temple::SuibianTempleConfig};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BooRole {
    Adventure,
    Craft,
    Sales,
}
impl BooRole {
    pub fn from_name(name: &str) -> Option<Self> {
        [
            ("[游历]", Self::Adventure),
            ("[制造]", Self::Craft),
            ("[售卖]", Self::Sales),
        ]
        .into_iter()
        .find_map(|(p, r)| {
            name.strip_prefix(p)
                .is_some_and(|n| !n.is_empty())
                .then_some(r)
        })
    }
    pub fn minimum(self, c: &SuibianTempleConfig) -> Option<u32> {
        match self {
            Self::Adventure => c.boo_box_adventure_price,
            Self::Craft => c.boo_box_craft_price,
            Self::Sales => c.boo_box_sell_price,
        }
        .minimum()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BooPhase {
    Enter,
    Inspect,
    Select,
    Hire,
    Refresh,
    Reward,
    Dismiss,
    Return,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BooOutcome {
    Pending,
    Exhausted,
    RefreshLimit,
    InsufficientFunds,
    CapacityLimit,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BooTarget {
    pub epoch: u8,
    pub slot: u8,
    pub price: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BooOffer {
    pub slot: u8,
    pub price: Option<u32>,
    pub sold_out: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BooSelection {
    pub slot: u8,
    pub name: String,
    pub price: u32,
    pub hire: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BooRefresh {
    pub counter: [u8; 2],
    pub cost: Option<u32>,
    pub active: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BooPage {
    pub balance: Option<u32>,
    pub offers: Vec<BooOffer>,
    pub list_readable: bool,
    pub selected: Option<BooSelection>,
    pub refresh: Option<BooRefresh>,
    pub exhausted: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BooView {
    Unknown,
    Entry,
    Shops,
    Page(BooPage),
    Animation { skip: bool },
    Reward { name: String, quantity: u32 },
    Rejection(BooOutcome),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BooSpend {
    Hire {
        target: BooTarget,
        name: String,
        role: BooRole,
        balance: u32,
    },
    Refresh {
        epoch: u8,
        refresh: BooRefresh,
        balance: u32,
    },
}
impl BooSpend {
    pub fn balance(&self) -> u32 {
        match self {
            Self::Hire { balance, .. } | Self::Refresh { balance, .. } => *balance,
        }
    }
    pub fn cost(&self) -> u32 {
        match self {
            Self::Hire { target, .. } => target.price,
            Self::Refresh { refresh, .. } => refresh.cost.unwrap_or(0),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BooTransaction {
    pub spend: BooSpend,
    pub reward_seen: bool,
    pub acknowledged: bool,
    pub balance_after: Option<u32>,
    pub counter_after: Option<[u8; 2]>,
    pub rejected: Option<BooOutcome>,
    pub rejection_dismissed: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BooCommitKind {
    Spend,
    ConfirmReward,
    DismissRejection,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BooCommit {
    pub transaction: u16,
    pub kind: BooCommitKind,
    pub state: CommitState,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BooVisitResult {
    Purchased,
    DisabledRole,
    BelowMinimum,
    UnknownRole,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BooVisit {
    pub target: BooTarget,
    pub result: BooVisitResult,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BooAction {
    OpenShops,
    OpenShop,
    Select(BooTarget),
    Spend(BooSpend),
    RevealSkip,
    Skip,
    ConfirmReward,
    DismissRejection,
    Back,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TempleBooBox {
    pub config: SuibianTempleConfig,
    pub phase: BooPhase,
    pub outcome: BooOutcome,
    pub epoch: u8,
    pub direction: Option<bool>,
    pub target: Option<BooTarget>,
    pub selection_posted: bool,
    pub unknown_role_scans: u8,
    pub exhausted_scans: u8,
    pub animation_step: u8,
    pub visits: Vec<BooVisit>,
    pub transactions: Vec<BooTransaction>,
    pub active: Option<u16>,
    pub commits: Vec<BooCommit>,
    pub pending: Option<u16>,
    pub returned_entry: bool,
}
fn valid_target(t: &BooTarget) -> bool {
    t.epoch <= 50 && (1..=6).contains(&t.slot) && [25000, 30000, 35000, 40000].contains(&t.price)
}
fn valid_counter(c: [u8; 2]) -> bool {
    c[1] > 0 && c[1] <= 50 && c[0] <= c[1]
}
fn step(before: [u8; 2], after: [u8; 2]) -> Option<bool> {
    (valid_counter(before) && valid_counter(after) && before[1] == after[1]).then_some(())?;
    if before[0].checked_add(1) == Some(after[0]) {
        Some(true)
    } else if after[0].checked_add(1) == Some(before[0]) {
        Some(false)
    } else {
        None
    }
}
impl TempleBooBox {
    pub fn new(config: SuibianTempleConfig) -> Result<Self, &'static str> {
        config.validate()?;
        if !config.boo_box_purchase_enabled {
            return Err("本次配置未开启邦巢购买");
        }
        Ok(Self {
            config,
            phase: BooPhase::Enter,
            outcome: BooOutcome::Pending,
            epoch: 0,
            direction: None,
            target: None,
            selection_posted: false,
            unknown_role_scans: 0,
            exhausted_scans: 0,
            animation_step: 0,
            visits: vec![],
            transactions: vec![],
            active: None,
            commits: vec![],
            pending: None,
            returned_entry: false,
        })
    }
    fn observed(&self, tx: usize, kind: BooCommitKind) -> bool {
        self.commits.iter().any(|c| {
            usize::from(c.transaction) == tx && c.kind == kind && c.state == CommitState::Observed
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate()?;
        if !self.config.boo_box_purchase_enabled
            || self.epoch > 50
            || self.visits.len() > 306
            || self.transactions.len() > 356
            || self.commits.len() > 4096
            || self.unknown_role_scans > 3
            || self.exhausted_scans > 2
            || self.animation_step > 2
            || self
                .target
                .as_ref()
                .is_some_and(|t| !valid_target(t) || t.epoch != self.epoch)
            || self.selection_posted && self.target.is_none()
            || self.returned_entry != (self.phase == BooPhase::Done)
            || (self.outcome != BooOutcome::Pending)
                != matches!(self.phase, BooPhase::Return | BooPhase::Done)
            || self
                .active
                .is_some_and(|i| usize::from(i) + 1 != self.transactions.len())
            || matches!(
                self.phase,
                BooPhase::Hire | BooPhase::Refresh | BooPhase::Reward | BooPhase::Dismiss
            ) && self.active.is_none()
            || self.phase == BooPhase::Select && self.target.is_none()
            || self.phase == BooPhase::Hire && self.target.is_none()
        {
            return Err("邦巢检查点缺少冻结设置、目标、阶段或有界记录");
        }
        for (i, v) in self.visits.iter().enumerate() {
            if !valid_target(&v.target)
                || v.target.epoch > self.epoch
                || self.visits[..i]
                    .iter()
                    .any(|o| o.target.epoch == v.target.epoch && o.target.slot == v.target.slot)
            {
                return Err("邦巢卡片记录重复或超出本次刷新");
            }
        }
        for (i, c) in self.commits.iter().enumerate() {
            if usize::from(c.transaction) >= self.transactions.len()
                || (c.state == CommitState::Intent) != (self.pending == Some(i as u16))
            {
                return Err("邦巢提交归属无效");
            }
        }
        if self.pending.is_some_and(|i| {
            usize::from(i) + 1 != self.commits.len()
                || self.active != Some(self.commits[usize::from(i)].transaction)
        }) {
            return Err("邦巢待确认提交不是当前事务");
        }
        let mut epochs = 0;
        let mut direction = None;
        let mut last_counter = None;
        for (i, t) in self.transactions.iter().enumerate() {
            let hire = match &t.spend {
                BooSpend::Hire {
                    target, name, role, ..
                } => {
                    if !valid_target(target)
                        || target.epoch != epochs
                        || BooRole::from_name(name) != Some(*role)
                        || name.chars().count() > 64
                        || role
                            .minimum(&self.config)
                            .is_none_or(|min| target.price < min)
                    {
                        return Err("邦巢聘用不符合冻结角色最低价格");
                    }
                    true
                }
                BooSpend::Refresh { epoch, refresh, .. } => {
                    if *epoch != epochs
                        || refresh.cost != Some(200)
                        || !refresh.active
                        || !valid_counter(refresh.counter)
                        || *epoch >= 50
                        || last_counter.is_some_and(|c| c != refresh.counter)
                    {
                        return Err("邦巢刷新参数无效");
                    }
                    false
                }
            };
            if t.spend.cost() > t.spend.balance()
                || t.balance_after
                    .is_some_and(|b| b < t.spend.balance() - t.spend.cost())
                || t.rejected.is_some_and(|r| {
                    !matches!(r, BooOutcome::InsufficientFunds | BooOutcome::CapacityLimit)
                        || !hire && r == BooOutcome::CapacityLimit
                })
                || t.reward_seen && (!hire || t.rejected.is_some())
                || t.acknowledged && !t.reward_seen
                || t.rejection_dismissed && t.rejected.is_none()
                || t.reward_seen && !self.observed(i, BooCommitKind::Spend)
                || t.rejected.is_some() && !self.observed(i, BooCommitKind::Spend)
                || t.acknowledged != self.observed(i, BooCommitKind::ConfirmReward)
                || t.rejection_dismissed != self.observed(i, BooCommitKind::DismissRejection)
                || self.observed(i, BooCommitKind::Spend)
                    && !t.reward_seen
                    && t.counter_after.is_none()
                    && t.rejected.is_none()
            {
                return Err("邦巢事务缺少实际获得、刷新或拒绝证明");
            }
            if let Some(after) = t.counter_after {
                let BooSpend::Refresh { refresh, .. } = &t.spend else {
                    return Err("聘用不能伪造刷新后态");
                };
                let d = step(refresh.counter, after).ok_or("邦巢刷新计数未恰好变化一次")?;
                if direction.is_some_and(|v| v != d)
                    || t.balance_after.is_none()
                    || t.rejected.is_some()
                    || !self.observed(i, BooCommitKind::Spend)
                {
                    return Err("邦巢刷新方向或余额后态不符");
                }
                direction = Some(d);
                last_counter = Some(after);
                epochs += 1;
            }
            if Some(i as u16) != self.active
                && !(t.counter_after.is_some()
                    || t.acknowledged && t.balance_after.is_some()
                    || t.rejection_dismissed)
            {
                return Err("邦巢事务未完成就交给后继");
            }
        }
        if epochs != self.epoch || direction != self.direction {
            return Err("邦巢刷新次数不能由点击累加");
        }
        if self.pending.is_some_and(|i| {
            let c = &self.commits[usize::from(i)];
            !match c.kind {
                BooCommitKind::Spend => matches!(self.phase, BooPhase::Hire | BooPhase::Refresh),
                BooCommitKind::ConfirmReward => self.phase == BooPhase::Reward,
                BooCommitKind::DismissRejection => self.phase == BooPhase::Dismiss,
            }
        }) || matches!(self.phase, BooPhase::Return | BooPhase::Done) && self.active.is_some()
        {
            return Err("邦巢阶段与待确认事务不符");
        }
        if let Some(i) = self.active {
            let t = &self.transactions[usize::from(i)];
            match &t.spend {
                BooSpend::Hire { target, .. }
                    if self.target.as_ref() != Some(target)
                        || !self.selection_posted
                        || !matches!(
                            self.phase,
                            BooPhase::Select
                                | BooPhase::Hire
                                | BooPhase::Reward
                                | BooPhase::Dismiss
                        ) =>
                {
                    return Err("当前邦巢聘用缺少已选中卡片");
                }
                BooSpend::Refresh { epoch, .. }
                    if *epoch != self.epoch
                        || !matches!(
                            self.phase,
                            BooPhase::Inspect | BooPhase::Refresh | BooPhase::Dismiss
                        ) =>
                {
                    return Err("当前邦巢刷新不属于本轮");
                }
                _ => {}
            }
            if matches!(self.phase, BooPhase::Hire | BooPhase::Refresh) && self.pending.is_none() {
                return Err("邦巢花费阶段缺少提交意图");
            }
        }
        for v in self
            .visits
            .iter()
            .filter(|v| v.result == BooVisitResult::Purchased)
        {
            if !self.transactions.iter().any(|t| {
                t.acknowledged
                    && t.balance_after.is_some()
                    && matches!(&t.spend, BooSpend::Hire { target, .. } if target == &v.target)
            }) {
                return Err("邦巢购买记录缺少实际获得与确认");
            }
        }
        if self.phase == BooPhase::Reward
            && self.active.is_none_or(|i| {
                !self.transactions[usize::from(i)].reward_seen
                    || self.transactions[usize::from(i)].acknowledged
            })
            || self.phase == BooPhase::Dismiss
                && self.active.is_none_or(|i| {
                    self.transactions[usize::from(i)].rejected.is_none()
                        || self.transactions[usize::from(i)].rejection_dismissed
                })
        {
            return Err("邦巢确认窗口不属于当前事务");
        }
        Ok(())
    }
    pub fn check_update(&self, n: &Self) -> Result<(), &'static str> {
        self.validate()?;
        n.validate()?;
        if self.config != n.config
            || self.completed() && self != n
            || self.outcome != BooOutcome::Pending && self.outcome != n.outcome
            || !n.visits.starts_with(&self.visits)
            || n.visits.len() > self.visits.len() + 1
            || n.transactions.len() < self.transactions.len()
            || n.transactions.len() > self.transactions.len() + 1
            || n.commits.len() < self.commits.len()
            || n.commits.len() > self.commits.len() + 1
        {
            return Err("不能改写邦巢冻结配置或既有结果");
        }
        let receipt = |tx: usize, kind| {
            self.pending.is_some_and(|p| {
                self.commits[usize::from(p)].transaction == tx as u16
                    && self.commits[usize::from(p)].kind == kind
                    && n.commits[usize::from(p)].state == CommitState::Observed
            })
        };
        if let Some(v) = n.visits.get(self.visits.len())
            && (self.target.as_ref() != Some(&v.target)
                || if v.result == BooVisitResult::Purchased {
                    self.active
                        .is_none_or(|i| !receipt(usize::from(i), BooCommitKind::ConfirmReward))
                } else {
                    self.phase != BooPhase::Select
                        || !self.selection_posted
                        || self.active.is_some()
                })
        {
            return Err("邦巢卡片结果缺少本次选中或获得证明");
        }
        if self.animation_step != n.animation_step
            && !(n.animation_step == 0 && n.visits.len() == self.visits.len() + 1
                || self.phase == BooPhase::Hire
                    && n.phase == self.phase
                    && self.active == n.active
                    && n.animation_step == self.animation_step + 1)
        {
            return Err("邦巢动画跳过步骤不能重放或跳级");
        }
        for (i, (o, v)) in self.transactions.iter().zip(&n.transactions).enumerate() {
            if Some(i as u16) != self.active && o != v
                || o.spend != v.spend
                || o.reward_seen && !v.reward_seen
                || o.acknowledged && !v.acknowledged
                || o.rejection_dismissed && !v.rejection_dismissed
                || o.balance_after.is_some() && o.balance_after != v.balance_after
                || o.counter_after.is_some() && o.counter_after != v.counter_after
                || o.rejected.is_some() && o.rejected != v.rejected
                || !o.reward_seen && v.reward_seen && !receipt(i, BooCommitKind::Spend)
                || o.counter_after.is_none()
                    && v.counter_after.is_some()
                    && !receipt(i, BooCommitKind::Spend)
                || o.rejected.is_none() && v.rejected.is_some() && !receipt(i, BooCommitKind::Spend)
                || !o.acknowledged && v.acknowledged && !receipt(i, BooCommitKind::ConfirmReward)
                || !o.rejection_dismissed
                    && v.rejection_dismissed
                    && !receipt(i, BooCommitKind::DismissRejection)
                || o.balance_after.is_none()
                    && v.balance_after.is_some()
                    && !(receipt(i, BooCommitKind::Spend) && v.counter_after.is_some()
                        || receipt(i, BooCommitKind::ConfirmReward))
            {
                return Err("不能改写邦巢事务或跳过实际提交回执");
            }
        }
        for (o, v) in self.commits.iter().zip(&n.commits) {
            if o.transaction != v.transaction
                || o.kind != v.kind
                || o.state != CommitState::Intent && o != v
            {
                return Err("不能改写邦巢提交回执");
            }
        }
        if let Some(c) = n.commits.get(self.commits.len())
            && (self.pending.is_some()
                || c.state != CommitState::Intent
                || n.pending != Some(self.commits.len() as u16)
                || n.active != Some(c.transaction)
                || !match c.kind {
                    BooCommitKind::Spend => matches!(
                        (self.phase, n.phase),
                        (BooPhase::Select, BooPhase::Hire) | (BooPhase::Inspect, BooPhase::Refresh)
                    ),
                    BooCommitKind::ConfirmReward => {
                        self.phase == BooPhase::Reward && n.phase == self.phase
                    }
                    BooCommitKind::DismissRejection => {
                        self.phase == BooPhase::Dismiss && n.phase == self.phase
                    }
                })
        {
            return Err("邦巢新意图不属于当前阶段");
        }
        if let Some(t) = n.transactions.get(self.transactions.len())
            && (self.active.is_some()
                || n.active != Some(self.transactions.len() as u16)
                || n.commits.len() != self.commits.len() + 1
                || t.reward_seen
                || t.acknowledged
                || t.balance_after.is_some()
                || t.counter_after.is_some()
                || t.rejected.is_some()
                || t.rejection_dismissed)
        {
            return Err("邦巢新事务必须先保存空白提交意图");
        }
        if self.target != n.target
            && !(self.target.is_none()
                && n.phase == BooPhase::Select
                && self.phase == BooPhase::Inspect
                || n.target.is_none()
                    && n.phase == BooPhase::Inspect
                    && (n.visits.len() == self.visits.len() + 1 || n.epoch == self.epoch + 1))
        {
            return Err("不能替换当前邦巢卡片");
        }
        if !self.selection_posted
            && n.selection_posted
            && (self.phase != BooPhase::Select || n.phase != self.phase || self.target != n.target)
        {
            return Err("邦巢卡片缺少本次选中动作");
        }
        if self.epoch != n.epoch
            && (n.epoch != self.epoch + 1
                || self.phase != BooPhase::Refresh
                || self
                    .active
                    .is_none_or(|i| !receipt(usize::from(i), BooCommitKind::Spend)))
        {
            return Err("邦巢刷新次数必须来自实际后态");
        }
        if self.outcome != n.outcome
            && !(n.phase == BooPhase::Return
                && match n.outcome {
                    BooOutcome::RefreshLimit => self.phase == BooPhase::Inspect && self.epoch == 50,
                    BooOutcome::Exhausted => {
                        self.phase == BooPhase::Inspect
                            && self.exhausted_scans == 1
                            && n.exhausted_scans == 2
                    }
                    BooOutcome::InsufficientFunds => matches!(
                        self.phase,
                        BooPhase::Inspect | BooPhase::Select | BooPhase::Dismiss
                    ),
                    BooOutcome::CapacityLimit => self.phase == BooPhase::Dismiss,
                    BooOutcome::Pending => false,
                })
        {
            return Err("邦巢完成原因缺少对应检查阶段");
        }
        Ok(())
    }
    fn edit(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let mut n = self.clone();
        f(&mut n)?;
        self.check_update(&n)?;
        *self = n;
        Ok(())
    }
    pub fn completed(&self) -> bool {
        self.phase == BooPhase::Done && self.returned_entry
    }
    pub fn prepare(&mut self, a: &BooAction) -> Result<(), &'static str> {
        self.edit(|s| {
            let kind = match a {
                BooAction::Spend(spend) => {
                    if !matches!(
                        (&spend, s.phase),
                        (BooSpend::Hire { .. }, BooPhase::Select)
                            | (BooSpend::Refresh { .. }, BooPhase::Inspect)
                    ) {
                        return Err("邦巢花费阶段不符");
                    }
                    if let Some(i) = s.active {
                        if s.transactions[usize::from(i)].spend != *spend {
                            return Err("不能更换邦巢未投递事务");
                        }
                    } else {
                        s.active = Some(s.transactions.len() as u16);
                        s.transactions.push(BooTransaction {
                            spend: spend.clone(),
                            reward_seen: false,
                            acknowledged: false,
                            balance_after: None,
                            counter_after: None,
                            rejected: None,
                            rejection_dismissed: false,
                        });
                    }
                    s.phase = if matches!(spend, BooSpend::Hire { .. }) {
                        BooPhase::Hire
                    } else {
                        BooPhase::Refresh
                    };
                    BooCommitKind::Spend
                }
                BooAction::ConfirmReward if s.phase == BooPhase::Reward => {
                    BooCommitKind::ConfirmReward
                }
                BooAction::DismissRejection if s.phase == BooPhase::Dismiss => {
                    BooCommitKind::DismissRejection
                }
                BooAction::ConfirmReward | BooAction::DismissRejection => {
                    return Err("邦巢确认不属于当前事务");
                }
                _ => return Ok(()),
            };
            if s.pending.is_some() || s.commits.len() >= 4096 {
                return Err("邦巢提交未完成或记录已满");
            }
            s.pending = Some(s.commits.len() as u16);
            s.commits.push(BooCommit {
                transaction: s.active.ok_or("邦巢缺少事务")?,
                kind,
                state: CommitState::Intent,
            });
            Ok(())
        })
    }
    pub fn posted_navigation(&mut self, a: &BooAction) -> Result<(), &'static str> {
        self.edit(|s| {
            match a {
                BooAction::Select(t)
                    if s.phase == BooPhase::Select && s.target.as_ref() == Some(t) =>
                {
                    s.selection_posted = true
                }
                BooAction::RevealSkip if s.phase == BooPhase::Hire && s.animation_step == 0 => {
                    s.animation_step = 1
                }
                BooAction::Skip if s.phase == BooPhase::Hire && s.animation_step == 1 => {
                    s.animation_step = 2
                }
                BooAction::Select(_) | BooAction::RevealSkip | BooAction::Skip => {
                    return Err("邦巢导航不属于当前选择或动画");
                }
                _ => {}
            }
            Ok(())
        })
    }
    pub fn not_posted(&mut self) -> Result<(), &'static str> {
        self.edit(|s| {
            if let Some(i) = s.pending.take() {
                let c = &mut s.commits[usize::from(i)];
                c.state = CommitState::NotPosted;
                if c.kind == BooCommitKind::Spend {
                    s.phase = if matches!(
                        s.transactions[usize::from(c.transaction)].spend,
                        BooSpend::Hire { .. }
                    ) {
                        BooPhase::Select
                    } else {
                        BooPhase::Inspect
                    }
                }
            }
            Ok(())
        })
    }
    fn observe(&mut self) {
        let i = self.pending.take().unwrap();
        self.commits[usize::from(i)].state = CommitState::Observed;
    }
    fn finish(&mut self, r: BooOutcome) {
        self.outcome = r;
        self.phase = BooPhase::Return;
    }
    fn visit(&mut self, result: BooVisitResult) {
        self.visits.push(BooVisit {
            target: self.target.take().unwrap(),
            result,
        });
        self.selection_posted = false;
        self.unknown_role_scans = 0;
        self.animation_step = 0;
        self.phase = BooPhase::Inspect;
    }
    pub fn next(&mut self, v: &BooView) -> Result<Option<BooAction>, &'static str> {
        let mut n = self.clone();
        let a = n.advance(v)?;
        self.check_update(&n)?;
        *self = n;
        Ok(a)
    }
    fn advance(&mut self, v: &BooView) -> Result<Option<BooAction>, &'static str> {
        use BooAction as A;
        use BooPhase as P;
        use BooView as V;
        if self.completed() || matches!(v, V::Unknown) {
            return Ok(None);
        }
        if let Some(i) = self.pending {
            let c = self.commits[usize::from(i)].clone();
            let tx = usize::from(c.transaction);
            let t = self.transactions[tx].clone();
            match c.kind {
                BooCommitKind::Spend => match (&t.spend, v) {
                    (
                        BooSpend::Hire { name, .. },
                        V::Reward {
                            name: n,
                            quantity: 1,
                        },
                    ) if n == name => {
                        self.observe();
                        self.transactions[tx].reward_seen = true;
                        self.phase = P::Reward;
                    }
                    (BooSpend::Hire { .. }, V::Animation { skip }) => {
                        return Ok(if self.animation_step == 0 {
                            Some(A::RevealSkip)
                        } else if *skip && self.animation_step == 1 {
                            Some(A::Skip)
                        } else {
                            None
                        });
                    }
                    (BooSpend::Refresh { refresh, .. }, V::Page(p)) => {
                        if let (Some(r), Some(b)) = (&p.refresh, p.balance)
                            && p.list_readable
                            && let Some(d) = step(refresh.counter, r.counter)
                        {
                            if self.direction.is_some_and(|v| v != d)
                                || b < t.spend.balance() - t.spend.cost()
                            {
                                return Err("邦巢刷新后计数方向或余额异常");
                            }
                            self.observe();
                            let t = &mut self.transactions[tx];
                            t.counter_after = Some(r.counter);
                            t.balance_after = Some(b);
                            self.active = None;
                            self.epoch += 1;
                            self.direction = Some(d);
                            self.exhausted_scans = 0;
                            self.phase = P::Inspect;
                        }
                    }
                    (_, V::Rejection(r))
                        if *r == BooOutcome::InsufficientFunds
                            || *r == BooOutcome::CapacityLimit
                                && matches!(t.spend, BooSpend::Hire { .. }) =>
                    {
                        self.observe();
                        self.transactions[tx].rejected = Some(*r);
                        self.phase = P::Dismiss;
                    }
                    _ => {}
                },
                BooCommitKind::ConfirmReward => {
                    if let V::Page(p) = v
                        && p.list_readable
                        && let Some(b) = p.balance
                    {
                        if b < t.spend.balance() - t.spend.cost() {
                            return Err("邦巢聘用后余额出现额外减少");
                        }
                        self.observe();
                        self.transactions[tx].acknowledged = true;
                        self.transactions[tx].balance_after = Some(b);
                        self.active = None;
                        self.visit(BooVisitResult::Purchased);
                    }
                }
                BooCommitKind::DismissRejection => {
                    if let V::Page(p) = v
                        && p.list_readable
                        && p.balance.is_some()
                    {
                        self.observe();
                        self.transactions[tx].rejection_dismissed = true;
                        self.active = None;
                        self.finish(t.rejected.unwrap());
                    }
                }
            }
            return Ok(None);
        }
        match self.phase {
            P::Enter => match v {
                V::Entry => Ok(Some(A::OpenShops)),
                V::Shops => Ok(Some(A::OpenShop)),
                V::Page(_) => {
                    self.phase = P::Inspect;
                    Ok(None)
                }
                _ => Err("邦巢起点不是入口或商店"),
            },
            P::Inspect => {
                let V::Page(p) = v else { return Ok(None) };
                if p.offers.len() > 6
                    || p.offers.iter().enumerate().any(|(i, o)| {
                        !(1..=6).contains(&o.slot) || p.offers[..i].iter().any(|v| v.slot == o.slot)
                    })
                {
                    return Err("邦巢卡片列号重复或越界");
                }
                if let Some(i) = self.active {
                    let t = &self.transactions[usize::from(i)];
                    return Ok(self
                        .spend_matches(&t.spend, p)
                        .then(|| A::Spend(t.spend.clone())));
                }
                let Some(balance) = p.balance else {
                    return Ok(None);
                };
                let candidate =
                    p.offers
                        .iter()
                        .filter(|o| {
                            !o.sold_out
                                && o.price
                                    .is_some_and(|n| [25000, 30000, 35000, 40000].contains(&n))
                                && !self.visits.iter().any(|v| {
                                    v.target.epoch == self.epoch && v.target.slot == o.slot
                                })
                        })
                        .min_by_key(|o| (u32::MAX - o.price.unwrap(), o.slot));
                if let Some(o) = candidate {
                    self.target = Some(BooTarget {
                        epoch: self.epoch,
                        slot: o.slot,
                        price: o.price.unwrap(),
                    });
                    self.phase = P::Select;
                    self.exhausted_scans = 0;
                    return Ok(None);
                }
                if !p.list_readable
                    || p.offers.len() != 6
                    || p.offers.iter().any(|o| !o.sold_out && o.price.is_none())
                {
                    self.exhausted_scans = 0;
                    return Ok(None);
                }
                if self.epoch == 50 {
                    self.finish(BooOutcome::RefreshLimit);
                    return Ok(None);
                }
                if p.exhausted {
                    self.exhausted_scans = (self.exhausted_scans + 1).min(2);
                    if self.exhausted_scans == 2 {
                        self.finish(BooOutcome::Exhausted);
                    }
                    return Ok(None);
                }
                self.exhausted_scans = 0;
                let Some(refresh) = p
                    .refresh
                    .as_ref()
                    .filter(|r| r.active && r.cost == Some(200) && valid_counter(r.counter))
                else {
                    return Ok(None);
                };
                if self
                    .transactions
                    .iter()
                    .rev()
                    .find_map(|t| t.counter_after)
                    .is_some_and(|c| c != refresh.counter)
                {
                    return Err("邦巢刷新计数在本次提交之外发生变化");
                }
                if balance < refresh.cost.unwrap() {
                    self.finish(BooOutcome::InsufficientFunds);
                    return Ok(None);
                }
                Ok(Some(A::Spend(BooSpend::Refresh {
                    epoch: self.epoch,
                    refresh: refresh.clone(),
                    balance,
                })))
            }
            P::Select => {
                let V::Page(p) = v else { return Ok(None) };
                let t = self.target.as_ref().unwrap().clone();
                if !self.selection_posted {
                    return Ok(p
                        .offers
                        .iter()
                        .any(|o| o.slot == t.slot && o.price == Some(t.price) && !o.sold_out)
                        .then_some(A::Select(t)));
                }
                let Some(sel) = p
                    .selected
                    .as_ref()
                    .filter(|s| s.slot == t.slot && s.price == t.price && !s.name.is_empty())
                else {
                    return Ok(None);
                };
                if let Some(i) = self.active {
                    let spec = &self.transactions[usize::from(i)].spend;
                    return Ok(self.spend_matches(spec, p).then(|| A::Spend(spec.clone())));
                }
                let Some(role) = BooRole::from_name(&sel.name) else {
                    self.unknown_role_scans = (self.unknown_role_scans + 1).min(3);
                    if self.unknown_role_scans == 3 {
                        self.visit(BooVisitResult::UnknownRole);
                    }
                    return Ok(None);
                };
                self.unknown_role_scans = 0;
                let Some(min) = role.minimum(&self.config) else {
                    self.visit(BooVisitResult::DisabledRole);
                    return Ok(None);
                };
                if t.price < min {
                    self.visit(BooVisitResult::BelowMinimum);
                    return Ok(None);
                }
                let Some(balance) = p.balance else {
                    return Ok(None);
                };
                if balance < t.price {
                    self.finish(BooOutcome::InsufficientFunds);
                    return Ok(None);
                }
                Ok(sel.hire.then(|| {
                    A::Spend(BooSpend::Hire {
                        target: t,
                        name: sel.name.clone(),
                        role,
                        balance,
                    })
                }))
            }
            P::Reward => {
                let t = &self.transactions[usize::from(self.active.unwrap())];
                Ok(matches!(v,V::Reward { name,quantity:1 } if matches!(&t.spend,BooSpend::Hire { name:n,.. } if n==name)).then_some(A::ConfirmReward))
            }
            P::Dismiss => {
                let t = &self.transactions[usize::from(self.active.unwrap())];
                Ok(matches!(v,V::Rejection(r) if t.rejected==Some(*r))
                    .then_some(A::DismissRejection))
            }
            P::Return => match v {
                V::Entry => {
                    self.returned_entry = true;
                    self.phase = P::Done;
                    Ok(None)
                }
                V::Page(_) | V::Shops => Ok(Some(A::Back)),
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }
    fn spend_matches(&self, s: &BooSpend, p: &BooPage) -> bool {
        if p.balance.is_none_or(|b| b < s.balance())
            || p.offers.len() > 6
            || p.offers.iter().enumerate().any(|(i, o)| {
                !(1..=6).contains(&o.slot) || p.offers[..i].iter().any(|v| v.slot == o.slot)
            })
        {
            return false;
        }
        match s {
            BooSpend::Hire { target, name, .. } => {
                self.target.as_ref() == Some(target)
                    && self.selection_posted
                    && p.selected.as_ref().is_some_and(|v| {
                        v.slot == target.slot
                            && v.name == *name
                            && v.price == target.price
                            && v.hire
                    })
                    && p.offers.iter().any(|o| {
                        o.slot == target.slot && o.price == Some(target.price) && !o.sold_out
                    })
            }
            BooSpend::Refresh { epoch, refresh, .. } => {
                *epoch == self.epoch
                    && p.list_readable
                    && p.offers.len() == 6
                    && p.offers.iter().all(|o| o.sold_out || o.price.is_some())
                    && p.offers.iter().all(|o| {
                        o.sold_out
                            || !o
                                .price
                                .is_some_and(|v| [25000, 30000, 35000, 40000].contains(&v))
                            || self
                                .visits
                                .iter()
                                .any(|v| v.target.epoch == self.epoch && v.target.slot == o.slot)
                    })
                    && p.refresh.as_ref() == Some(refresh)
                    && !p.exhausted
            }
        }
    }
    pub fn allows(&self, a: &BooAction, v: &BooView) -> bool {
        if self.validate().is_err() {
            return false;
        }
        match a {
            BooAction::Spend(s) => {
                self.pending.is_some_and(|i| {
                    let c = &self.commits[usize::from(i)];
                    c.kind == BooCommitKind::Spend
                        && self.transactions[usize::from(c.transaction)].spend == *s
                }) && matches!(v,BooView::Page(p) if self.spend_matches(s,p))
            }
            BooAction::ConfirmReward => {
                self.pending.is_some_and(|i| {
                    self.commits[usize::from(i)].kind == BooCommitKind::ConfirmReward
                }) && matches!(v,BooView::Reward{name,quantity:1} if matches!(&self.transactions[usize::from(self.active.unwrap())].spend,BooSpend::Hire{name:n,..} if n==name))
            }
            BooAction::DismissRejection => {
                self.pending.is_some_and(|i| {
                    self.commits[usize::from(i)].kind == BooCommitKind::DismissRejection
                }) && matches!(v,BooView::Rejection(r) if self.transactions[usize::from(self.active.unwrap())].rejected==Some(*r))
            }
            _ => self.clone().next(v).ok().flatten().as_ref() == Some(a),
        }
    }
}
