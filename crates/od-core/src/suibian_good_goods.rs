//! 好物铺只购买本次冻结配置允许的能源插件，数量调整与实际兑换后态分开处理。
use crate::{suibian_adventure::CommitState, suibian_temple::SuibianTempleConfig};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const ENERGY_PLUGIN: &str = "邦布能源插件";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GoodPhase {
    Enter,
    Inspect,
    Open,
    Quantity,
    Purchase,
    Reward,
    Verify,
    Dismiss,
    Return,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GoodOutcome {
    Pending,
    Purchased,
    SoldOut,
    NotFound,
    InsufficientFunds,
    CapacityLimit,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GoodTarget {
    pub slot: u8,
    pub unit_price: u32,
    pub balance: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GoodPurchase {
    pub target: GoodTarget,
    pub quantity: u32,
    pub limit: u32,
    pub cost: u32,
    pub reward_seen: bool,
    pub reward_confirmed: bool,
    pub balance_after: Option<u32>,
    pub rejected: Option<GoodOutcome>,
    pub rejection_dismissed: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GoodCommitKind {
    Purchase,
    ConfirmReward,
    DismissRejection,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GoodCommit {
    pub kind: GoodCommitKind,
    pub state: CommitState,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoodOffer {
    pub slot: u8,
    pub unit_price: Option<u32>,
    pub sold_out: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoodPage {
    pub business: bool,
    pub balance: Option<u32>,
    pub offers: Vec<GoodOffer>,
    pub list_readable: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoodExchange {
    pub name: String,
    pub quantity: u32,
    pub limit: u32,
    pub balance: u32,
    pub cost: u32,
    pub confirm: bool,
}
impl GoodExchange {
    pub fn maximum(&self, price: u32) -> Option<u32> {
        (price > 0 && self.limit > 0 && self.limit <= 1_000_000)
            .then(|| self.limit.min(self.balance / price))
            .filter(|v| *v > 0)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoodView {
    Unknown,
    Entry,
    Shops,
    Page(GoodPage),
    Exchange(GoodExchange),
    Reward { name: String, quantity: u32 },
    Rejection(GoodOutcome),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoodAction {
    OpenShops,
    OpenShop,
    BusinessTab,
    OpenGood(GoodTarget),
    MaxQuantity(GoodExchange),
    Purchase(GoodPurchase),
    ConfirmReward,
    DismissRejection,
    Back,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TempleGoodGoods {
    pub config: SuibianTempleConfig,
    pub phase: GoodPhase,
    pub outcome: GoodOutcome,
    pub target: Option<GoodTarget>,
    pub opening_posted: bool,
    pub purchase: Option<GoodPurchase>,
    pub commits: Vec<GoodCommit>,
    pub pending: Option<u8>,
    pub scans: u8,
    pub returned_entry: bool,
}
impl TempleGoodGoods {
    pub fn new(config: SuibianTempleConfig) -> Result<Self, &'static str> {
        config.validate()?;
        if !config.good_goods_purchase_enabled {
            return Err("本次配置未开启好物铺购买");
        }
        Ok(Self {
            config,
            phase: GoodPhase::Enter,
            outcome: GoodOutcome::Pending,
            target: None,
            opening_posted: false,
            purchase: None,
            commits: vec![],
            pending: None,
            scans: 0,
            returned_entry: false,
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate()?;
        if !self.config.good_goods_purchase_enabled
            || self.commits.len() > 32
            || self.scans > 2
            || self.target.as_ref().is_some_and(|t| {
                !(1..=9).contains(&t.slot) || t.unit_price < 100 || t.unit_price > 1_000_000_000
            })
            || self.opening_posted && self.target.is_none()
            || self.returned_entry != (self.phase == GoodPhase::Done)
            || self.phase == GoodPhase::Done
                && (self.outcome == GoodOutcome::Pending || self.pending.is_some())
        {
            return Err("好物铺检查点缺少冻结开关、商品或返回证明");
        }
        if self.pending.is_some_and(|i| {
            usize::from(i) + 1 != self.commits.len()
                || self.commits[usize::from(i)].state != CommitState::Intent
        }) || self
            .commits
            .iter()
            .enumerate()
            .any(|(i, c)| c.state == CommitState::Intent && self.pending != Some(i as u8))
        {
            return Err("好物铺待确认提交归属无效");
        }
        if self
            .pending
            .is_some_and(|i| match self.commits[usize::from(i)].kind {
                GoodCommitKind::Purchase => self.phase != GoodPhase::Purchase,
                GoodCommitKind::ConfirmReward => self.phase != GoodPhase::Reward,
                GoodCommitKind::DismissRejection => self.phase != GoodPhase::Dismiss,
            })
            || (self.phase == GoodPhase::Purchase && self.pending.is_none())
        {
            return Err("好物铺提交与当前阶段不符");
        }
        if matches!(self.phase, GoodPhase::Return | GoodPhase::Done)
            != (self.outcome != GoodOutcome::Pending)
            || matches!(
                self.phase,
                GoodPhase::Open
                    | GoodPhase::Quantity
                    | GoodPhase::Purchase
                    | GoodPhase::Reward
                    | GoodPhase::Verify
                    | GoodPhase::Dismiss
            ) && self.target.is_none()
            || matches!(
                self.phase,
                GoodPhase::Quantity
                    | GoodPhase::Purchase
                    | GoodPhase::Reward
                    | GoodPhase::Verify
                    | GoodPhase::Dismiss
            ) && !self.opening_posted
            || self.phase == GoodPhase::Reward
                && self
                    .purchase
                    .as_ref()
                    .is_none_or(|p| !p.reward_seen || p.reward_confirmed)
            || self.phase == GoodPhase::Verify
                && self.purchase.as_ref().is_none_or(|p| !p.reward_confirmed)
            || self.phase == GoodPhase::Dismiss
                && self
                    .purchase
                    .as_ref()
                    .is_none_or(|p| p.rejected.is_none() || p.rejection_dismissed)
        {
            return Err("好物铺阶段缺少对应菜单、兑换或获得事实");
        }
        if let Some(p) = &self.purchase {
            let expected = p.limit.min(p.target.balance / p.target.unit_price.max(1));
            if self.target.as_ref() != Some(&p.target)
                || !self.opening_posted
                || p.quantity == 0
                || p.limit == 0
                || p.limit > 1_000_000
                || p.quantity != expected
                || p.quantity.checked_mul(p.target.unit_price) != Some(p.cost)
                || p.cost > p.target.balance
                || p.reward_confirmed && !p.reward_seen
                || p.rejected.is_some_and(|r| {
                    !matches!(
                        r,
                        GoodOutcome::InsufficientFunds | GoodOutcome::CapacityLimit
                    )
                })
                || p.rejection_dismissed && p.rejected.is_none()
                || p.reward_seen && p.rejected.is_some()
                || p.balance_after
                    .is_some_and(|v| v < p.target.balance - p.cost)
                || p.reward_seen && !self.observed(GoodCommitKind::Purchase)
                || self.observed(GoodCommitKind::Purchase) && !p.reward_seen && p.rejected.is_none()
                || p.reward_confirmed && !self.observed(GoodCommitKind::ConfirmReward)
                || p.rejection_dismissed && !self.observed(GoodCommitKind::DismissRejection)
            {
                return Err("好物铺兑换缺少最大数量、金额或本次后态");
            }
        } else if self.pending.is_some() || !self.commits.is_empty() {
            return Err("好物铺提交缺少本次兑换");
        }
        if self.outcome == GoodOutcome::Purchased
            && self.purchase.as_ref().is_none_or(|p| {
                !p.reward_confirmed || p.balance_after.is_none() || p.rejected.is_some()
            })
            || self.purchase.as_ref().is_some_and(|p| p.rejected.is_some())
                && matches!(self.phase, GoodPhase::Return | GoodPhase::Done)
                && self
                    .purchase
                    .as_ref()
                    .is_none_or(|p| !p.rejection_dismissed || p.rejected != Some(self.outcome))
        {
            return Err("好物铺完成结果缺少实际获得、余额或拒绝收尾");
        }
        Ok(())
    }
    fn observed(&self, kind: GoodCommitKind) -> bool {
        self.commits
            .iter()
            .any(|c| c.kind == kind && c.state == CommitState::Observed)
    }
    pub fn check_update(&self, n: &Self) -> Result<(), &'static str> {
        self.validate()?;
        n.validate()?;
        if self.config != n.config
            || self.opening_posted && !n.opening_posted
            || self.outcome != GoodOutcome::Pending && self.outcome != n.outcome
            || self.completed() && self != n
            || n.commits.len() < self.commits.len()
            || n.commits.len() > self.commits.len() + 1
        {
            return Err("不能改写好物铺冻结设置、商品或已完成结果");
        }
        if let Some(old) = &self.target {
            let Some(target) = &n.target else {
                return Err("不能删除好物铺商品");
            };
            if old.slot != target.slot
                || old.unit_price != target.unit_price
                || old.balance != target.balance
                    && !(self.purchase.is_none()
                        && n.purchase.is_none()
                        && self.opening_posted
                        && matches!(self.phase, GoodPhase::Open | GoodPhase::Quantity)
                        && n.phase == GoodPhase::Quantity
                        && target.balance >= old.balance)
            {
                return Err("只能在兑换准备期间更新同一商品的增加余额");
            }
        }
        for (o, v) in self.commits.iter().zip(&n.commits) {
            if o.kind != v.kind || o.state != CommitState::Intent && o != v {
                return Err("不能改写好物铺提交回执");
            }
        }
        if let Some(c) = n.commits.get(self.commits.len())
            && (self.pending.is_some()
                || c.state != CommitState::Intent
                || n.pending != Some(self.commits.len() as u8)
                || !matches!(
                    (self.phase, n.phase, c.kind),
                    (
                        GoodPhase::Quantity,
                        GoodPhase::Purchase,
                        GoodCommitKind::Purchase
                    ) | (
                        GoodPhase::Reward,
                        GoodPhase::Reward,
                        GoodCommitKind::ConfirmReward
                    ) | (
                        GoodPhase::Dismiss,
                        GoodPhase::Dismiss,
                        GoodCommitKind::DismissRejection
                    )
                ))
        {
            return Err("好物铺新意图不属于当前阶段");
        }
        let receipt = |kind| {
            self.pending.is_some_and(|i| {
                self.commits[usize::from(i)].kind == kind
                    && n.commits[usize::from(i)].state == CommitState::Observed
            })
        };
        if self.target.is_none()
            && n.target.is_some()
            && (self.phase != GoodPhase::Inspect || n.phase != GoodPhase::Open)
        {
            return Err("好物铺目标须来自本次可读经营页");
        }
        if self.outcome != n.outcome {
            let valid = match n.outcome {
                GoodOutcome::Purchased => {
                    self.phase == GoodPhase::Verify && n.phase == GoodPhase::Return
                }
                GoodOutcome::SoldOut | GoodOutcome::NotFound => {
                    self.phase == GoodPhase::Inspect
                        && n.phase == GoodPhase::Return
                        && self.scans == 1
                        && n.scans == 2
                        && n.purchase.is_none()
                }
                GoodOutcome::InsufficientFunds if self.phase == GoodPhase::Inspect => {
                    n.phase == GoodPhase::Return && n.purchase.is_none()
                }
                GoodOutcome::InsufficientFunds | GoodOutcome::CapacityLimit => {
                    self.phase == GoodPhase::Dismiss
                        && n.phase == GoodPhase::Return
                        && receipt(GoodCommitKind::DismissRejection)
                }
                GoodOutcome::Pending => false,
            };
            if !valid {
                return Err("好物铺完成或无工作结果缺少当前检查阶段");
            }
        }
        match (&self.purchase, &n.purchase) {
            (Some(o), Some(v)) => {
                let mut same = v.clone();
                same.reward_seen = o.reward_seen;
                same.reward_confirmed = o.reward_confirmed;
                same.balance_after = o.balance_after;
                same.rejected = o.rejected;
                same.rejection_dismissed = o.rejection_dismissed;
                if &same != o
                    || o.reward_seen && !v.reward_seen
                    || o.reward_confirmed && !v.reward_confirmed
                    || o.balance_after.is_some() && o.balance_after != v.balance_after
                    || o.rejected.is_some() && o.rejected != v.rejected
                    || o.rejection_dismissed && !v.rejection_dismissed
                    || !o.reward_seen && v.reward_seen && !receipt(GoodCommitKind::Purchase)
                    || !o.reward_confirmed
                        && v.reward_confirmed
                        && !receipt(GoodCommitKind::ConfirmReward)
                    || o.rejected.is_none()
                        && v.rejected.is_some()
                        && !receipt(GoodCommitKind::Purchase)
                    || !o.rejection_dismissed
                        && v.rejection_dismissed
                        && !receipt(GoodCommitKind::DismissRejection)
                    || o.balance_after.is_none()
                        && v.balance_after.is_some()
                        && self.phase != GoodPhase::Verify
                {
                    return Err("好物铺兑换事实不能改写或跳过提交回执");
                }
            }
            (Some(_), None) => return Err("不能删除好物铺兑换"),
            (None, Some(p))
                if self.phase != GoodPhase::Quantity
                    || n.phase != GoodPhase::Purchase
                    || p.reward_seen
                    || p.reward_confirmed
                    || p.balance_after.is_some()
                    || p.rejected.is_some()
                    || p.rejection_dismissed =>
            {
                return Err("好物铺兑换必须从本次最大数量开始");
            }
            _ => {}
        }
        if !self.opening_posted
            && n.opening_posted
            && (self.phase != GoodPhase::Open || n.phase != GoodPhase::Open)
        {
            return Err("商品菜单缺少本次打开动作");
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
        self.phase == GoodPhase::Done && self.returned_entry
    }
    pub fn prepare(&mut self, a: &GoodAction) -> Result<(), &'static str> {
        self.edit(|s| {
            let kind = match a {
                GoodAction::Purchase(p)
                    if s.phase == GoodPhase::Quantity
                        && s.purchase.as_ref().is_none_or(|v| v == p) =>
                {
                    s.purchase = Some(p.clone());
                    s.phase = GoodPhase::Purchase;
                    GoodCommitKind::Purchase
                }
                GoodAction::ConfirmReward
                    if s.phase == GoodPhase::Reward
                        && s.purchase
                            .as_ref()
                            .is_some_and(|p| p.reward_seen && !p.reward_confirmed) =>
                {
                    GoodCommitKind::ConfirmReward
                }
                GoodAction::DismissRejection
                    if s.phase == GoodPhase::Dismiss
                        && s.purchase
                            .as_ref()
                            .is_some_and(|p| p.rejected.is_some() && !p.rejection_dismissed) =>
                {
                    GoodCommitKind::DismissRejection
                }
                GoodAction::Purchase(_)
                | GoodAction::ConfirmReward
                | GoodAction::DismissRejection => return Err("好物铺提交阶段不符"),
                _ => return Ok(()),
            };
            if s.pending.is_some() || s.commits.len() >= 32 {
                return Err("好物铺提交尚未确认或次数已满");
            }
            s.pending = Some(s.commits.len() as u8);
            s.commits.push(GoodCommit {
                kind,
                state: CommitState::Intent,
            });
            Ok(())
        })
    }
    pub fn posted_navigation(&mut self, a: &GoodAction) -> Result<(), &'static str> {
        self.edit(|s| {
            if let GoodAction::OpenGood(t) = a {
                if s.phase != GoodPhase::Open || s.target.as_ref() != Some(t) {
                    return Err("打开的好物不是当前商品");
                }
                s.opening_posted = true;
            }
            Ok(())
        })
    }
    pub fn not_posted(&mut self) -> Result<(), &'static str> {
        self.edit(|s| {
            if let Some(i) = s.pending.take() {
                let c = &mut s.commits[usize::from(i)];
                c.state = CommitState::NotPosted;
                if c.kind == GoodCommitKind::Purchase {
                    s.phase = GoodPhase::Quantity;
                }
            }
            Ok(())
        })
    }
    fn observe(&mut self) {
        let i = self.pending.take().unwrap();
        self.commits[usize::from(i)].state = CommitState::Observed;
    }
    pub fn next(&mut self, v: &GoodView) -> Result<Option<GoodAction>, &'static str> {
        let mut n = self.clone();
        let a = n.advance(v)?;
        self.check_update(&n)?;
        *self = n;
        Ok(a)
    }
    fn advance(&mut self, v: &GoodView) -> Result<Option<GoodAction>, &'static str> {
        use GoodAction as A;
        use GoodPhase as P;
        use GoodView as V;
        if matches!(v, V::Unknown) || self.completed() {
            return Ok(None);
        }
        if let Some(i) = self.pending {
            match self.commits[usize::from(i)].kind {
                GoodCommitKind::Purchase => match v {
                    V::Reward { name, quantity }
                        if name == ENERGY_PLUGIN
                            && *quantity == self.purchase.as_ref().unwrap().quantity =>
                    {
                        self.observe();
                        self.purchase.as_mut().unwrap().reward_seen = true;
                        self.phase = P::Reward;
                    }
                    V::Rejection(reason)
                        if matches!(
                            reason,
                            GoodOutcome::InsufficientFunds | GoodOutcome::CapacityLimit
                        ) =>
                    {
                        self.observe();
                        self.purchase.as_mut().unwrap().rejected = Some(*reason);
                        self.phase = P::Dismiss;
                    }
                    _ => {}
                },
                GoodCommitKind::ConfirmReward if matches!(v,V::Page(p) if p.business && p.list_readable) =>
                {
                    self.observe();
                    self.purchase.as_mut().unwrap().reward_confirmed = true;
                    self.phase = P::Verify;
                }
                GoodCommitKind::DismissRejection if matches!(v,V::Page(p) if p.business&&p.list_readable&&p.balance.is_some()) =>
                {
                    self.observe();
                    let p = self.purchase.as_mut().unwrap();
                    p.rejection_dismissed = true;
                    self.outcome = p.rejected.unwrap();
                    self.phase = P::Return;
                }
                _ => {}
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
                _ => Err("好物铺起点不是入口或商店"),
            },
            P::Inspect => {
                let V::Page(p) = v else { return Ok(None) };
                if p.offers.len() > 9
                    || p.offers.iter().enumerate().any(|(i, o)| {
                        !(1..=9).contains(&o.slot) || p.offers[..i].iter().any(|v| v.slot == o.slot)
                    })
                {
                    return Err("好物铺商品位置缺少有效去重列号");
                }
                if !p.business {
                    return Ok(Some(A::BusinessTab));
                }
                if !p.list_readable {
                    self.scans = 0;
                    return Ok(None);
                }
                let Some(balance) = p.balance else {
                    return Ok(None);
                };
                if p.offers.is_empty() {
                    self.scans = (self.scans + 1).min(2);
                    if self.scans == 2 {
                        self.outcome = GoodOutcome::NotFound;
                        self.phase = P::Return;
                    }
                    return Ok(None);
                }
                let o = p.offers.iter().min_by_key(|o| o.slot).unwrap();
                if o.sold_out {
                    self.scans = (self.scans + 1).min(2);
                    if self.scans == 2 {
                        self.outcome = GoodOutcome::SoldOut;
                        self.phase = P::Return;
                    }
                    return Ok(None);
                }
                let Some(price) = o.unit_price.filter(|p| *p >= 100) else {
                    return Ok(None);
                };
                if balance < price {
                    self.outcome = GoodOutcome::InsufficientFunds;
                    self.phase = P::Return;
                    return Ok(None);
                }
                self.target = Some(GoodTarget {
                    slot: o.slot,
                    unit_price: price,
                    balance,
                });
                self.phase = P::Open;
                Ok(None)
            }
            P::Open => match v {
                V::Page(p) if p.business && !self.opening_posted => {
                    let t = self.target.as_ref().unwrap();
                    Ok(p.offers
                        .iter()
                        .any(|o| {
                            o.slot == t.slot && o.unit_price == Some(t.unit_price) && !o.sold_out
                        })
                        .then(|| A::OpenGood(t.clone()))
                        .filter(|_| p.balance.is_some_and(|b| b >= t.balance)))
                }
                V::Exchange(e) if self.opening_posted => {
                    self.check_exchange(e)?;
                    self.target.as_mut().unwrap().balance = e.balance;
                    self.phase = P::Quantity;
                    Ok(None)
                }
                V::Exchange(_) => Err("兑换菜单没有本次打开记录"),
                _ => Ok(None),
            },
            P::Quantity => {
                let V::Exchange(e) = v else { return Ok(None) };
                self.check_exchange(e)?;
                if let Some(saved) = &self.purchase {
                    if e.quantity != saved.quantity
                        || e.limit != saved.limit
                        || e.cost != saved.cost
                    {
                        return Err("未投递的兑换数量已改变");
                    }
                    return Ok(e.confirm.then(|| A::Purchase(saved.clone())));
                }
                self.target.as_mut().unwrap().balance = e.balance;
                let target = self.target.as_ref().unwrap();
                let max = e.maximum(target.unit_price).ok_or("兑换数量或余额不足")?;
                if e.quantity != max {
                    return Ok(Some(A::MaxQuantity(e.clone())));
                }
                if !e.confirm {
                    return Ok(None);
                }
                let p = GoodPurchase {
                    target: target.clone(),
                    quantity: e.quantity,
                    limit: e.limit,
                    cost: e.cost,
                    reward_seen: false,
                    reward_confirmed: false,
                    balance_after: None,
                    rejected: None,
                    rejection_dismissed: false,
                };
                if self.purchase.as_ref().is_some_and(|saved| saved != &p) {
                    return Err("未投递的兑换数量已改变");
                }
                Ok(Some(A::Purchase(p)))
            }
            P::Reward => match v {
                V::Reward { name, quantity }
                    if name == ENERGY_PLUGIN
                        && *quantity == self.purchase.as_ref().unwrap().quantity =>
                {
                    Ok(Some(A::ConfirmReward))
                }
                _ => Ok(None),
            },
            P::Verify => {
                let V::Page(p) = v else { return Ok(None) };
                let bought = self.purchase.as_mut().unwrap();
                let Some(balance) = p.balance else {
                    return Ok(None);
                };
                if !p.business || !p.list_readable {
                    return Ok(None);
                }
                if balance >= bought.target.balance - bought.cost {
                    bought.balance_after = Some(balance);
                    self.outcome = GoodOutcome::Purchased;
                    self.phase = P::Return;
                    Ok(None)
                } else {
                    Err("兑换后余额不符合本次数量和总价")
                }
            }
            P::Dismiss => match v {
                V::Rejection(r) if self.purchase.as_ref().unwrap().rejected == Some(*r) => {
                    Ok(Some(A::DismissRejection))
                }
                _ => Ok(None),
            },
            P::Return => match v {
                V::Entry => {
                    self.returned_entry = true;
                    self.phase = P::Done;
                    Ok(None)
                }
                V::Page(_) | V::Shops => Ok(Some(A::Back)),
                _ => Ok(None),
            },
            P::Purchase => Err("好物铺等待兑换缺少投递意图"),
            P::Done => Ok(None),
        }
    }
    fn check_exchange(&self, e: &GoodExchange) -> Result<(), &'static str> {
        let t = self.target.as_ref().ok_or("缺少兑换目标")?;
        if e.name != ENERGY_PLUGIN
            || e.balance < t.balance
            || e.quantity == 0
            || e.limit == 0
            || e.quantity > e.limit
            || e.quantity.checked_mul(t.unit_price) != Some(e.cost)
            || e.cost > e.balance
        {
            return Err("兑换商品、数量或总价与本次冻结目标不符");
        }
        Ok(())
    }
    pub fn allows(&self, a: &GoodAction, v: &GoodView) -> bool {
        if self.validate().is_err() {
            return false;
        }
        if let GoodAction::Purchase(p) = a {
            return self.purchase.as_ref() == Some(p)
                && self.pending.is_some_and(|i| {
                    self.commits[usize::from(i)].kind == GoodCommitKind::Purchase
                })
                && matches!(v, GoodView::Exchange(e) if self.check_exchange(e).is_ok() && e.quantity == p.quantity && e.limit == p.limit && e.cost == p.cost && e.confirm);
        }
        if let GoodAction::MaxQuantity(plan) = a {
            return self.phase == GoodPhase::Quantity
                && self.purchase.is_none()
                && matches!(v, GoodView::Exchange(e) if self.check_exchange(e).is_ok() && e.name == plan.name && e.quantity == plan.quantity && e.limit == plan.limit && e.cost == plan.cost && e.balance >= plan.balance);
        }
        let mut copy = self.clone();
        if let Some(i) = copy.pending.take() {
            let kind = copy.commits[usize::from(i)].kind;
            copy.commits.pop();
            if kind == GoodCommitKind::Purchase {
                copy.phase = GoodPhase::Quantity;
            }
        }
        copy.next(v).is_ok_and(|v| v.as_ref() == Some(a))
    }
}
