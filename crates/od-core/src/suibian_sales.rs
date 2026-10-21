//! 售卖铺：第二只邦布、缺货取消及四个货架的开售分别保留实际后态。
use crate::{
    suibian_adventure::CommitState, suibian_craft::goods_key, suibian_temple::SuibianTempleConfig,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SalesPhase {
    Enter,
    BooOpen,
    BooSelect,
    BooWait,
    BooBack,
    CancelScan,
    CancelShelf,
    CancelWait,
    CancelBack,
    StartScan,
    StartShelf,
    StartWait,
    StartBack,
    Return,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SalesBooOutcome {
    Pending,
    Assigned,
    AlreadySecond,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SalesBoo {
    Unknown,
    Available,
    Busy,
    Exhausted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SalesShelfState {
    Unknown,
    Empty,
    Shortage,
    Selling,
    Locked,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalesShelf {
    pub slot: u8,
    pub state: SalesShelfState,
    pub goods: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalesSelection {
    pub slot: u8,
    pub goods: String,
    pub stock: Option<u32>,
    pub shortage: bool,
    pub can_start: bool,
    pub can_cancel: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalesPicker {
    pub second: SalesBoo,
    pub selected: Option<u8>,
    pub name: Option<String>,
    pub energy: Option<(u32, u32)>,
    pub assigned: bool,
    pub can_assign: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SalesView {
    Unknown,
    Managed,
    Entry,
    Management,
    Main(Vec<SalesShelf>),
    Goods(SalesSelection),
    Picker(SalesPicker),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SalesTarget {
    pub slot: u8,
    pub goods: String,
    pub stock: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SalesSpec {
    AssignSecond { name: String, energy: u32, max: u32 },
    Cancel { target: SalesTarget },
    Start { target: SalesTarget },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SalesCommit {
    pub spec: SalesSpec,
    pub state: CommitState,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SalesAction {
    OpenManagement,
    OpenSales,
    OpenPicker,
    SelectSecond,
    OpenShelf(u8),
    Assign(SalesSpec),
    Cancel(SalesSpec),
    Start(SalesSpec),
    Back,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TempleSales {
    pub config: SuibianTempleConfig,
    pub phase: SalesPhase,
    pub boo: SalesBooOutcome,
    pub commits: Vec<SalesCommit>,
    pub pending: Option<u8>,
    pub cancelled: Vec<SalesTarget>,
    pub started: Vec<SalesTarget>,
    pub current: Option<u8>,
    pub target: Option<SalesTarget>,
    pub no_stock: Option<u8>,
    pub shelves_checked: bool,
    pub scans: u8,
    pub returned_entry: bool,
}
impl TempleSales {
    pub fn new(config: SuibianTempleConfig) -> Result<Self, &'static str> {
        config.validate()?;
        Ok(Self {
            config,
            phase: SalesPhase::Enter,
            boo: SalesBooOutcome::Pending,
            commits: vec![],
            pending: None,
            cancelled: vec![],
            started: vec![],
            current: None,
            target: None,
            no_stock: None,
            shelves_checked: false,
            scans: 0,
            returned_entry: false,
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate()?;
        let target_valid = |t: &SalesTarget| {
            (1..=4).contains(&t.slot)
                && goods_key(&t.goods).as_ref() == Some(&t.goods)
                && t.stock <= 1_000_000_000
        };
        if self.commits.len() > 64
            || self.scans > 2
            || self.cancelled.len() > 4
            || self.started.len() > 4
            || self.current.is_some_and(|s| !(1..=4).contains(&s))
            || self.no_stock.is_some_and(|s| !(1..=4).contains(&s))
            || self
                .target
                .as_ref()
                .is_some_and(|t| !target_valid(t) || Some(t.slot) != self.current)
            || self.returned_entry != (self.phase == SalesPhase::Done)
            || (self.phase == SalesPhase::Done
                && (self.boo == SalesBooOutcome::Pending
                    || (!self.shelves_checked && self.no_stock.is_none())
                    || self.pending.is_some()
                    || self.current.is_some()))
        {
            return Err("售卖检查点缺少目标、工作结果或入口返回证明");
        }
        for (i, c) in self.commits.iter().enumerate() {
            let valid = match &c.spec {
                SalesSpec::AssignSecond { name, energy, max } => {
                    !name.is_empty()
                        && name.chars().count() <= 64
                        && *energy > 0
                        && energy <= max
                        && *max <= 1_000_000
                }
                SalesSpec::Start { target } => target_valid(target) && target.stock > 0,
                SalesSpec::Cancel { target } => target_valid(target),
            };
            if !valid || (c.state == CommitState::Intent && self.pending != Some(i as u8)) {
                return Err("售卖提交目标或意图归属无效");
            }
        }
        if self.pending.is_some_and(|i| {
            usize::from(i) + 1 != self.commits.len()
                || self.commits[usize::from(i)].state != CommitState::Intent
        }) || matches!(
            self.phase,
            SalesPhase::BooWait | SalesPhase::CancelWait | SalesPhase::StartWait
        ) != self.pending.is_some()
        {
            return Err("售卖待确认提交缺少本次意图");
        }
        if let Some(i) = self.pending {
            let correct = match &self.commits[usize::from(i)].spec {
                SalesSpec::AssignSecond { .. } => {
                    self.phase == SalesPhase::BooWait && self.boo == SalesBooOutcome::Pending
                }
                SalesSpec::Cancel { target } => {
                    self.phase == SalesPhase::CancelWait && self.target.as_ref() == Some(target)
                }
                SalesSpec::Start { target } => {
                    self.phase == SalesPhase::StartWait && self.target.as_ref() == Some(target)
                }
            };
            if !correct {
                return Err("售卖意图与当前阶段不符");
            }
        }
        for (list, start) in [(&self.cancelled, false), (&self.started, true)] {
            for (i, t) in list.iter().enumerate() {
                if !target_valid(t)
                    || list[..i].iter().any(|v| v.slot == t.slot)
                    || !self.commits.iter().any(|c| {
                        c.state == CommitState::Observed
                            && match &c.spec {
                                SalesSpec::Start { target } => start && target == t,
                                SalesSpec::Cancel { target } => !start && target == t,
                                _ => false,
                            }
                    })
                {
                    return Err("售卖结果没有同货架同商品的实际后态");
                }
            }
        }
        if self.boo == SalesBooOutcome::Assigned
            && !self.commits.iter().any(|c| {
                c.state == CommitState::Observed && matches!(c.spec, SalesSpec::AssignSecond { .. })
            })
        {
            return Err("第二只邦布没有派驻后态");
        }
        Ok(())
    }
    pub fn check_update(&self, n: &Self) -> Result<(), &'static str> {
        self.validate()?;
        n.validate()?;
        if self.config != n.config
            || n.commits.len() < self.commits.len()
            || n.commits.len() > self.commits.len() + 1
            || !n.cancelled.starts_with(&self.cancelled)
            || n.cancelled.len() > self.cancelled.len() + 1
            || !n.started.starts_with(&self.started)
            || n.started.len() > self.started.len() + 1
            || (self.boo != SalesBooOutcome::Pending && self.boo != n.boo)
            || self.no_stock.is_some_and(|s| n.no_stock != Some(s))
            || self.shelves_checked && !n.shelves_checked
            || self.completed() && self != n
        {
            return Err("不能改写售卖冻结设置或已完成结果");
        }
        for (o, v) in self.commits.iter().zip(&n.commits) {
            if o.spec != v.spec || (o.state != CommitState::Intent && o != v) {
                return Err("不能改写售卖提交回执");
            }
        }
        if n.commits.len() > self.commits.len()
            && (self.pending.is_some()
                || n.pending != Some(self.commits.len() as u8)
                || n.commits
                    .last()
                    .is_none_or(|c| c.state != CommitState::Intent))
        {
            return Err("售卖须先保存新意图");
        }
        if let Some(commit) = n.commits.get(self.commits.len()) {
            let valid = match &commit.spec {
                SalesSpec::AssignSecond { .. } => {
                    self.phase == SalesPhase::BooSelect && n.phase == SalesPhase::BooWait
                }
                SalesSpec::Cancel { target } => {
                    self.phase == SalesPhase::CancelShelf
                        && n.phase == SalesPhase::CancelWait
                        && self.target.as_ref() == Some(target)
                }
                SalesSpec::Start { target } => {
                    self.phase == SalesPhase::StartShelf
                        && n.phase == SalesPhase::StartWait
                        && self.target.as_ref() == Some(target)
                }
            };
            if !valid {
                return Err("售卖新意图须属于当前已读取的阶段和目标");
            }
        }
        if self.boo != n.boo
            && matches!(
                n.boo,
                SalesBooOutcome::AlreadySecond | SalesBooOutcome::Unavailable
            )
            && (self.phase != SalesPhase::BooSelect
                || n.phase != SalesPhase::BooBack
                || self.pending.is_some())
        {
            return Err("售卖邦布无工作结果缺少当前卡片检查");
        }
        let observed = |spec: &SalesSpec| {
            self.pending.is_some_and(|i| {
                self.commits[usize::from(i)].spec == *spec
                    && n.commits[usize::from(i)].state == CommitState::Observed
            })
        };
        if n.cancelled.len() > self.cancelled.len()
            && !observed(&SalesSpec::Cancel {
                target: n.cancelled.last().unwrap().clone(),
            })
            || n.started.len() > self.started.len()
                && !observed(&SalesSpec::Start {
                    target: n.started.last().unwrap().clone(),
                })
            || self.boo != n.boo
                && n.boo == SalesBooOutcome::Assigned
                && !self.pending.is_some_and(|i| {
                    matches!(
                        self.commits[usize::from(i)].spec,
                        SalesSpec::AssignSecond { .. }
                    ) && n.commits[usize::from(i)].state == CommitState::Observed
                })
        {
            return Err("新增售卖结果缺少当前投递的后态");
        }
        if self.no_stock.is_none()
            && n.no_stock.is_some()
            && (self.phase != SalesPhase::StartShelf
                || n.no_stock != self.current
                || n.phase != SalesPhase::Return)
            || !self.shelves_checked
                && n.shelves_checked
                && (self.phase != SalesPhase::StartScan || n.phase != SalesPhase::Return)
        {
            return Err("售卖无工作结果缺少当前检查");
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
        self.phase == SalesPhase::Done && self.returned_entry
    }
    pub fn prepare(&mut self, a: &SalesAction) -> Result<(), &'static str> {
        self.edit(|s| {
            let (spec, from, to) = match a {
                SalesAction::Assign(v) => (v, SalesPhase::BooSelect, SalesPhase::BooWait),
                SalesAction::Cancel(v) => (v, SalesPhase::CancelShelf, SalesPhase::CancelWait),
                SalesAction::Start(v) => (v, SalesPhase::StartShelf, SalesPhase::StartWait),
                _ => return Ok(()),
            };
            if s.phase != from || s.pending.is_some() || s.commits.len() >= 64 {
                return Err("售卖提交阶段不符或次数已满");
            }
            let correct = match (a, spec) {
                (SalesAction::Assign(_), SalesSpec::AssignSecond { .. }) => true,
                (SalesAction::Cancel(_), SalesSpec::Cancel { target })
                | (SalesAction::Start(_), SalesSpec::Start { target }) => {
                    s.target.as_ref() == Some(target)
                }
                _ => false,
            };
            if !correct {
                return Err("售卖动作不属于本次目标");
            }
            s.pending = Some(s.commits.len() as u8);
            s.commits.push(SalesCommit {
                spec: spec.clone(),
                state: CommitState::Intent,
            });
            s.phase = to;
            Ok(())
        })
    }
    pub fn not_posted(&mut self) -> Result<(), &'static str> {
        self.edit(|s| {
            if let Some(i) = s.pending.take() {
                let c = &mut s.commits[usize::from(i)];
                c.state = CommitState::NotPosted;
                s.phase = match c.spec {
                    SalesSpec::AssignSecond { .. } => SalesPhase::BooSelect,
                    SalesSpec::Cancel { .. } => SalesPhase::CancelShelf,
                    SalesSpec::Start { .. } => SalesPhase::StartShelf,
                };
            }
            Ok(())
        })
    }
    fn observe(&mut self) {
        let i = self.pending.take().unwrap();
        self.commits[usize::from(i)].state = CommitState::Observed;
    }
    pub fn next(&mut self, v: &SalesView) -> Result<Option<SalesAction>, &'static str> {
        let mut n = self.clone();
        let a = n.advance(v)?;
        self.check_update(&n)?;
        *self = n;
        Ok(a)
    }
    fn advance(&mut self, v: &SalesView) -> Result<Option<SalesAction>, &'static str> {
        use SalesAction as A;
        use SalesPhase as P;
        use SalesView as V;
        if matches!(v, V::Managed) {
            return Err("售卖铺仍处于自动托管中");
        }
        if matches!(v, V::Unknown) || self.completed() {
            return Ok(None);
        }
        if let Some(i) = self.pending {
            match self.commits[usize::from(i)].spec.clone() {
                SalesSpec::AssignSecond { name, .. } => match v {
                    V::Main(_) => return Ok(Some(A::OpenPicker)),
                    V::Picker(p) if p.selected != Some(2) && p.second != SalesBoo::Unknown => {
                        return Ok(Some(A::SelectSecond));
                    }
                    V::Picker(p)
                        if p.selected == Some(2)
                            && p.assigned
                            && p.name.as_ref() == Some(&name) =>
                    {
                        self.observe();
                        self.boo = SalesBooOutcome::Assigned;
                        self.phase = P::BooBack;
                    }
                    _ => {}
                },
                SalesSpec::Cancel { target } => {
                    let main = matches!(v,V::Main(s) if s.iter().any(|s|s.slot==target.slot&&s.state==SalesShelfState::Empty));
                    let detail = matches!(v,V::Goods(s) if s.slot==target.slot&&s.goods==target.goods&&!s.can_cancel&&s.can_start);
                    if main || detail {
                        self.observe();
                        self.cancelled.push(target);
                        self.phase = if main {
                            self.clear_target();
                            P::CancelScan
                        } else {
                            P::CancelBack
                        };
                        self.scans = 0;
                    }
                }
                SalesSpec::Start { target } => {
                    let detail = matches!(v,V::Goods(s) if s.slot==target.slot&&s.goods==target.goods&&s.can_cancel&&!s.can_start);
                    let main = matches!(v,V::Main(s) if s.iter().any(|s|s.slot==target.slot&&s.state==SalesShelfState::Selling&&s.goods.as_ref()==Some(&target.goods)));
                    if detail || main {
                        self.observe();
                        self.started.push(target);
                        self.phase = if main {
                            self.clear_target();
                            P::StartScan
                        } else {
                            P::StartBack
                        };
                        self.scans = 0;
                    } else if matches!(v, V::Main(_)) {
                        return Ok(Some(A::OpenShelf(target.slot)));
                    }
                }
            }
            return Ok(None);
        }
        match self.phase {
            P::Enter => match v {
                V::Entry => Ok(Some(A::OpenManagement)),
                V::Management => Ok(Some(A::OpenSales)),
                V::Main(_) => {
                    self.phase = P::BooOpen;
                    Ok(None)
                }
                _ => Ok(None),
            },
            P::BooOpen => match v {
                V::Main(_) => {
                    self.phase = P::BooSelect;
                    Ok(Some(A::OpenPicker))
                }
                V::Picker(_) => {
                    self.phase = P::BooSelect;
                    Ok(None)
                }
                _ => Ok(None),
            },
            P::BooSelect => match v {
                V::Main(_) => Ok(Some(A::OpenPicker)),
                V::Picker(p) if p.second == SalesBoo::Unknown => Ok(None),
                V::Picker(p) if p.selected != Some(2) => Ok(Some(A::SelectSecond)),
                V::Picker(p) if p.assigned => {
                    self.boo = SalesBooOutcome::AlreadySecond;
                    self.phase = P::BooBack;
                    Ok(None)
                }
                V::Picker(p) if matches!(p.second, SalesBoo::Busy | SalesBoo::Exhausted) => {
                    self.boo = SalesBooOutcome::Unavailable;
                    self.phase = P::BooBack;
                    Ok(None)
                }
                V::Picker(p) if p.can_assign => {
                    let (Some(name), Some((energy, max))) = (&p.name, p.energy) else {
                        return Ok(None);
                    };
                    Ok(Some(A::Assign(SalesSpec::AssignSecond {
                        name: name.clone(),
                        energy,
                        max,
                    })))
                }
                _ => Ok(None),
            },
            P::BooBack => match v {
                V::Picker(_) => Ok(Some(A::Back)),
                V::Main(_) => {
                    self.phase = P::CancelScan;
                    self.scans = 0;
                    Ok(None)
                }
                _ => Ok(None),
            },
            P::CancelScan | P::StartScan => {
                let V::Main(shelves) = v else { return Ok(None) };
                if !shelves_valid(shelves) {
                    return Ok(None);
                }
                let cancel = self.phase == P::CancelScan;
                let next = shelves.iter().find(|s| {
                    if cancel {
                        s.state == SalesShelfState::Shortage
                            && !self.cancelled.iter().any(|c| c.slot == s.slot)
                    } else {
                        s.state == SalesShelfState::Empty
                            && !self.started.iter().any(|c| c.slot == s.slot)
                    }
                });
                if let Some(s) = next {
                    self.current = Some(s.slot);
                    self.target = None;
                    self.scans = 0;
                    self.phase = if cancel {
                        P::CancelShelf
                    } else {
                        P::StartShelf
                    };
                    return Ok(Some(A::OpenShelf(s.slot)));
                }
                if shelves.iter().any(|s| s.state == SalesShelfState::Unknown) {
                    self.scans = 0;
                    return Ok(None);
                }
                if cancel && shelves.iter().any(|s| s.state == SalesShelfState::Shortage) {
                    return Err("已取消的缺货货架仍显示库存不足");
                }
                self.scans = (self.scans + 1).min(2);
                if self.scans < 2 {
                    return Ok(None);
                }
                self.scans = 0;
                if cancel {
                    self.phase = P::StartScan
                } else {
                    self.shelves_checked = true;
                    self.phase = P::Return
                }
                Ok(None)
            }
            P::CancelShelf | P::StartShelf => {
                let slot = self.current.ok_or("缺少售卖货架")?;
                if let V::Main(s) = v {
                    return Ok(s
                        .iter()
                        .find(|s| s.slot == slot)
                        .filter(|s| {
                            if self.phase == P::CancelShelf {
                                s.state == SalesShelfState::Shortage
                            } else {
                                s.state == SalesShelfState::Empty
                            }
                        })
                        .map(|_| A::OpenShelf(slot)));
                }
                let V::Goods(s) = v else { return Ok(None) };
                if s.slot != slot {
                    return Err("当前商品不是本次货架");
                }
                if goods_key(&s.goods).as_ref() != Some(&s.goods) {
                    return Ok(None);
                }
                let Some(stock) = s.stock else {
                    return Ok(None);
                };
                let target = SalesTarget {
                    slot,
                    goods: s.goods.clone(),
                    stock,
                };
                if self.phase == P::StartShelf && (s.shortage || stock == 0) {
                    self.no_stock = Some(slot);
                    self.phase = P::Return;
                    self.clear_target();
                    return Ok(None);
                }
                if self.target.as_ref() != Some(&target) {
                    self.target = Some(target);
                    return Ok(None);
                }
                if self.phase == P::CancelShelf && s.shortage && s.can_cancel && !s.can_start {
                    Ok(Some(A::Cancel(SalesSpec::Cancel { target })))
                } else if self.phase == P::StartShelf
                    && !s.shortage
                    && stock > 0
                    && s.can_start
                    && !s.can_cancel
                {
                    Ok(Some(A::Start(SalesSpec::Start { target })))
                } else {
                    Ok(None)
                }
            }
            P::CancelBack | P::StartBack => match v {
                V::Goods(_) => Ok(Some(A::Back)),
                V::Main(_) => {
                    let cancel = self.phase == P::CancelBack;
                    self.clear_target();
                    self.phase = if cancel { P::CancelScan } else { P::StartScan };
                    Ok(None)
                }
                _ => Ok(None),
            },
            P::Return => match v {
                V::Entry => {
                    self.phase = P::Done;
                    self.returned_entry = true;
                    Ok(None)
                }
                V::Main(_) | V::Goods(_) | V::Picker(_) | V::Management => Ok(Some(A::Back)),
                _ => Ok(None),
            },
            P::BooWait | P::CancelWait | P::StartWait => Err("售卖等待阶段缺少意图"),
            P::Done => Ok(None),
        }
    }
    fn clear_target(&mut self) {
        self.current = None;
        self.target = None;
    }
    pub fn allows(&self, a: &SalesAction, v: &SalesView) -> bool {
        if self.validate().is_err() {
            return false;
        }
        match a {
            SalesAction::Assign(spec) => {
                matches!((spec,v),(SalesSpec::AssignSecond{name,energy,max},SalesView::Picker(p)) if self.pending.is_some_and(|i|self.commits[usize::from(i)].spec==*spec)&&p.selected==Some(2)&&p.second==SalesBoo::Available&&!p.assigned&&p.can_assign&&p.name.as_ref()==Some(name)&&p.energy==Some((*energy,*max)))
            }
            SalesAction::Cancel(spec) => {
                matches!((spec,v),(SalesSpec::Cancel{target},SalesView::Goods(s)) if self.pending.is_some_and(|i|self.commits[usize::from(i)].spec==*spec)&&s.slot==target.slot&&s.goods==target.goods&&s.stock==Some(target.stock)&&s.shortage&&s.can_cancel&&!s.can_start)
            }
            SalesAction::Start(spec) => {
                matches!((spec,v),(SalesSpec::Start{target},SalesView::Goods(s)) if self.pending.is_some_and(|i|self.commits[usize::from(i)].spec==*spec)&&s.slot==target.slot&&s.goods==target.goods&&s.stock==Some(target.stock)&&!s.shortage&&s.can_start&&!s.can_cancel)
            }
            _ => {
                let mut copy = self.clone();
                copy.next(v).is_ok_and(|v| v.as_ref() == Some(a))
            }
        }
    }
}
fn shelves_valid(s: &[SalesShelf]) -> bool {
    s.len() == 4
        && s.iter().enumerate().all(|(i, s)| {
            s.slot == i as u8 + 1
                && s.goods
                    .as_ref()
                    .is_none_or(|n| goods_key(n).as_ref() == Some(n))
        })
}
