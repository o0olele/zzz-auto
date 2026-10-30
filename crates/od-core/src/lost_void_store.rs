//! 邦布商店交易：冻结价格和消费前状态，支付及货架后态共同确认完成。
use crate::{
    lost_void::{ArtifactCandidate, ArtifactSelection, LostVoidChallenge, choose_artifacts},
    lost_void_choice::{LostVoidChoiceReceipt, LostVoidChoiceRequest},
    lost_void_scene::{LostVoidObservation, LostVoidScreen as S},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreCurrency {
    Gold,
    Blood,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreOffer {
    pub artifact: ArtifactCandidate,
    pub price: Option<u32>,
    pub available: bool,
    pub special: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorePage {
    pub currency: Option<StoreCurrency>,
    pub balance: Option<u32>,
    pub health: Vec<u8>,
    pub offers: Vec<StoreOffer>,
    pub refresh_price: Option<u32>,
    pub refresh_enabled: bool,
    pub back: bool,
}
fn same_item(a: &ArtifactCandidate, b: &ArtifactCandidate) -> bool {
    a.category == b.category && a.name == b.name && a.level == b.level
}
impl StorePage {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.offers.len() > 8
            || self.health.len() > 3
            || self.health.iter().any(|h| *h > 100)
            || self.balance.is_some_and(|b| b > 99_999_999)
            || self.refresh_price.is_some_and(|p| p > 1_000_000)
            || self.offers.iter().enumerate().any(|(i, o)| {
                o.artifact.slot != i as u8
                    || o.price.is_some_and(|p| p > 1_000_000)
                    || self.offers[..i]
                        .iter()
                        .any(|a| same_item(&a.artifact, &o.artifact))
            })
            || (self.currency == Some(StoreCurrency::Blood)
                && (self.refresh_price.is_some_and(|p| p > 100)
                    || self.offers.iter().any(|o| o.price.is_some_and(|p| p > 100))))
        {
            return Err("迷失之地商店价格、商品或血量证据无效");
        }
        choose_artifacts(
            &LostVoidChallenge::default(),
            &self
                .offers
                .iter()
                .map(|o| o.artifact.clone())
                .collect::<Vec<_>>(),
            0,
            &ArtifactSelection::default(),
        )?;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorePurpose {
    Buy,
    Refresh,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreDialog {
    pub purpose: StorePurpose,
    pub costs: Vec<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidStoreView {
    pub scene: LostVoidObservation,
    pub page: Option<StorePage>,
    pub dialog: Option<StoreDialog>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidStoreRequest {
    pub run_id: Uuid,
    pub interaction_id: Uuid,
    pub challenge: LostVoidChallenge,
    pub options: ArtifactSelection,
    pub members: u8,
    pub timeout_ms: u32,
}
impl LostVoidStoreRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        LostVoidChoiceRequest {
            run_id: self.run_id,
            interaction_id: self.interaction_id,
            challenge: self.challenge.clone(),
            options: self.options.clone(),
            return_to: vec![S::Store],
            timeout_ms: self.timeout_ms,
        }
        .validate()?;
        if !(1..=3).contains(&self.members) || !self.options.ignored_slots.is_empty() {
            return Err("迷失之地商店人数无效或复用了其他货架的忽略槽位");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreAction {
    Buy(u8),
    Refresh,
    Confirm,
    Scroll,
    Back,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreStep {
    Input(StoreAction),
    Choice,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidStoreReceipt {
    pub run_id: Uuid,
    pub interaction_id: Uuid,
    pub returned_to: S,
    pub purchases: u16,
    pub refreshes: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transaction {
    purpose: StorePurpose,
    offer: Option<StoreOffer>,
    cost: u32,
    before: StorePage,
    confirm_issued: bool,
    child: Option<Uuid>,
    child_returned: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidStoreFlow {
    request: LostVoidStoreRequest,
    current: Option<StorePage>,
    ready: Option<StoreAction>,
    pending: Option<(StoreAction, bool)>,
    transaction: Option<Transaction>,
    scrolled: bool,
    purchases: u16,
    refreshes: u16,
    returned: bool,
    receipt: Option<LostVoidStoreReceipt>,
}
impl LostVoidStoreFlow {
    pub fn new(request: LostVoidStoreRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            current: None,
            ready: None,
            pending: None,
            transaction: None,
            scrolled: false,
            purchases: 0,
            refreshes: 0,
            returned: false,
            receipt: None,
        })
    }
    pub fn request(&self) -> &LostVoidStoreRequest {
        &self.request
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn receipt(&self) -> Option<&LostVoidStoreReceipt> {
        self.receipt.as_ref()
    }
    fn permitted(&self, p: &StorePage) -> Option<bool> {
        match p.currency? {
            StoreCurrency::Gold => Some(self.request.challenge.store_gold),
            StoreCurrency::Blood => {
                if !self.request.challenge.store_blood {
                    return Some(false);
                }
                if p.health.len() != self.request.members as usize {
                    return None;
                }
                Some(
                    p.health
                        .iter()
                        .all(|h| *h >= self.request.challenge.store_blood_min),
                )
            }
        }
    }
    fn funds(&self, p: &StorePage, cost: u32) -> Option<bool> {
        match p.currency? {
            StoreCurrency::Gold => Some(p.balance? >= cost),
            StoreCurrency::Blood => (p.health.len() == self.request.members as usize)
                .then(|| p.health.iter().all(|h| u32::from(*h) >= cost)),
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if let Some(p) = &self.current {
            p.validate()?;
        }
        if let Some(t) = &self.transaction {
            t.before.validate()?;
            if t.cost > 1_000_000
                || t.before.currency.is_none()
                || t.child.is_some_and(|id| id.is_nil())
                || (t.child_returned && t.child.is_none())
                || match t.purpose {
                    StorePurpose::Buy => t
                        .offer
                        .as_ref()
                        .is_none_or(|o| o.price != Some(t.cost) || !t.before.offers.contains(o)),
                    StorePurpose::Refresh => {
                        t.offer.is_some() || t.before.refresh_price != Some(t.cost)
                    }
                }
            {
                return Err("迷失之地商店交易检查点不一致");
            }
        }
        if self.purchases > 4096
            || self.refreshes > 4096
            || (self.ready.is_some() && self.pending.is_some())
            || self.pending.is_some_and(|(a, _)| {
                matches!(
                    a,
                    StoreAction::Buy(_) | StoreAction::Refresh | StoreAction::Confirm
                ) && self.transaction.is_none()
            })
            || self.receipt.as_ref().is_some_and(|r| {
                !self.returned
                    || r.run_id != self.request.run_id
                    || r.interaction_id != self.request.interaction_id
                    || ![S::World, S::Event].contains(&r.returned_to)
                    || r.purchases != self.purchases
                    || r.refreshes != self.refreshes
                    || self.transaction.is_some()
            })
        {
            return Err("迷失之地商店检查点不一致");
        }
        Ok(())
    }
    fn payment(&self, t: &Transaction, p: &StorePage) -> bool {
        if p.currency != t.before.currency {
            return false;
        }
        let own = self.pending.is_some_and(|(a, _)| {
            matches!(
                a,
                StoreAction::Buy(_) | StoreAction::Refresh | StoreAction::Confirm
            )
        });
        let normal_old: Vec<_> = t
            .before
            .offers
            .iter()
            .filter(|o| !o.special)
            .map(|o| (&o.artifact.category, &o.artifact.name))
            .collect();
        let normal_new: Vec<_> = p
            .offers
            .iter()
            .filter(|o| !o.special)
            .map(|o| (&o.artifact.category, &o.artifact.name))
            .collect();
        let acknowledged = own
            && (t.purpose == StorePurpose::Buy
                || (!normal_old.is_empty() && !normal_new.is_empty() && normal_old != normal_new));
        match p.currency {
            Some(StoreCurrency::Gold) => {
                t.before.balance.zip(p.balance).is_some_and(|(old, new)| {
                    old.checked_sub(t.cost) == Some(new)
                        || (acknowledged && (old != new || t.child_returned))
                })
            }
            Some(StoreCurrency::Blood) => {
                p.health.len() == self.request.members as usize
                    && p.health.len() == t.before.health.len()
                    && ((acknowledged
                        && (p.health != t.before.health || t.child_returned || t.cost == 0))
                        || (p
                            .health
                            .iter()
                            .zip(&t.before.health)
                            .all(|(new, old)| new <= old)
                            && (t.cost == 0 || p.health != t.before.health)))
            }
            None => false,
        }
    }
    fn stock_changed(t: &Transaction, p: &StorePage) -> bool {
        match &t.offer {
            Some(offer) => {
                if let Some(now) = p
                    .offers
                    .iter()
                    .find(|o| same_item(&o.artifact, &offer.artifact))
                {
                    return !now.available;
                }
                t.before
                    .offers
                    .iter()
                    .filter(|o| !same_item(&o.artifact, &offer.artifact))
                    .all(|o| {
                        p.offers.iter().any(|now| {
                            same_item(&o.artifact, &now.artifact)
                                && o.price == now.price
                                && o.available == now.available
                        })
                    })
            }
            None => {
                let old: Vec<_> = t
                    .before
                    .offers
                    .iter()
                    .filter(|o| !o.special)
                    .map(|o| (&o.artifact.category, &o.artifact.name))
                    .collect();
                let new: Vec<_> = p
                    .offers
                    .iter()
                    .filter(|o| !o.special)
                    .map(|o| (&o.artifact.category, &o.artifact.name))
                    .collect();
                t.cost > 0 || (!old.is_empty() && !new.is_empty() && old != new)
            }
        }
    }
    fn choose(&self, p: &StorePage, force: bool) -> Result<Option<StoreAction>, &'static str> {
        let release = force
            || u32::from(self.refreshes)
                >= u32::from(self.request.challenge.buy_only_priority_2.saturating_sub(1));
        let mut options = self.request.options.clone();
        options.priority_2 &= release;
        options.fallback &= release;
        options.ignored_slots = p
            .offers
            .iter()
            .filter(|o| !o.available)
            .map(|o| o.artifact.slot)
            .collect();
        let ranked = choose_artifacts(
            &self.request.challenge,
            &p.offers
                .iter()
                .map(|o| o.artifact.clone())
                .collect::<Vec<_>>(),
            p.offers.len() as u8,
            &options,
        )?;
        for a in ranked {
            let offer = &p.offers[a.slot as usize];
            let Some(cost) = offer.price else {
                return Ok(None);
            };
            match self.funds(p, cost) {
                Some(true) => return Ok(Some(StoreAction::Buy(a.slot))),
                Some(false) => {}
                None => return Ok(None),
            }
        }
        Ok(Some(StoreAction::Back))
    }
    pub fn next(&mut self, v: &LostVoidStoreView) -> Result<Option<StoreStep>, &'static str> {
        self.validate()?;
        self.ready = None;
        if self.completed() {
            return Ok(None);
        }
        if v.scene.screen == S::Unknown {
            return Ok(None);
        }
        if [S::World, S::Event].contains(&v.scene.screen) {
            if self.pending.is_some_and(|(a, _)| a == StoreAction::Back) || self.returned {
                self.returned = true;
                self.pending = None;
                self.receipt = Some(LostVoidStoreReceipt {
                    run_id: self.request.run_id,
                    interaction_id: self.request.interaction_id,
                    returned_to: v.scene.screen,
                    purchases: self.purchases,
                    refreshes: self.refreshes,
                });
            }
            return Ok(None);
        }
        if v.scene.screen == S::CommonChoice {
            let t = self
                .transaction
                .as_mut()
                .ok_or("迷失之地商店获得物缺少本次交易意图")?;
            if t.child_returned {
                return Err("迷失之地商店重复出现已交接获得物");
            }
            if t.child.is_none() {
                t.child = Some(Uuid::new_v4());
            }
            return Ok(Some(StoreStep::Choice));
        }
        if v.scene.screen == S::Modal {
            let t = self
                .transaction
                .as_ref()
                .ok_or("迷失之地不能采用已有商店确认框")?;
            let Some(d) = &v.dialog else {
                return Err("迷失之地商店确认框无法识别");
            };
            if d.purpose != t.purpose || d.costs.iter().filter(|p| **p == t.cost).count() != 1 {
                return Err("迷失之地商店确认目的或价格发生变化");
            }
            if t.confirm_issued {
                return Ok(None);
            }
            self.pending = None;
            self.ready = Some(StoreAction::Confirm);
            return Ok(Some(StoreStep::Input(StoreAction::Confirm)));
        }
        if v.scene.screen != S::Store {
            return Err("迷失之地商店遇到非预期页面");
        }
        let Some(p) = &v.page else { return Ok(None) };
        p.validate()?;
        if self.pending.is_some_and(|(a, _)| a == StoreAction::Back) {
            return Ok(None);
        }
        if self.pending.is_some_and(|(a, _)| a == StoreAction::Scroll) {
            self.scrolled = true;
            self.pending = None;
        }
        if let Some(t) = &self.transaction {
            if t.child.is_some() && !t.child_returned {
                return Ok(None);
            }
            if !self.payment(t, p) || !Self::stock_changed(t, p) {
                return Ok(None);
            }
            match t.purpose {
                StorePurpose::Buy => {
                    self.purchases = self
                        .purchases
                        .checked_add(1)
                        .ok_or("迷失之地商店购买次数溢出")?
                }
                StorePurpose::Refresh => {
                    self.refreshes = self
                        .refreshes
                        .checked_add(1)
                        .ok_or("迷失之地商店刷新次数溢出")?;
                    self.scrolled = false;
                }
            }
            self.transaction = None;
            self.pending = None;
        }
        self.current = Some(p.clone());
        let Some(permitted) = self.permitted(p) else {
            return Ok(None);
        };
        let action = if !permitted {
            StoreAction::Back
        } else {
            let candidate = match self.choose(p, false)? {
                Some(candidate) => candidate,
                None => {
                    let known_funds = match p.currency {
                        Some(StoreCurrency::Gold) => p.balance.is_some(),
                        Some(StoreCurrency::Blood) => {
                            p.health.len() == self.request.members as usize
                        }
                        None => false,
                    };
                    if !self.scrolled && known_funds {
                        self.ready = Some(StoreAction::Scroll);
                        return Ok(Some(StoreStep::Input(StoreAction::Scroll)));
                    }
                    return Ok(None);
                }
            };
            if matches!(candidate, StoreAction::Buy(_)) {
                candidate
            } else if !self.scrolled {
                StoreAction::Scroll
            } else if p.refresh_enabled {
                let Some(cost) = p.refresh_price else {
                    return Ok(None);
                };
                match self.funds(p, cost) {
                    Some(true) => StoreAction::Refresh,
                    Some(false) => {
                        let Some(fallback) = self.choose(p, true)? else {
                            return Ok(None);
                        };
                        fallback
                    }
                    None => return Ok(None),
                }
            } else {
                let Some(fallback) = self.choose(p, true)? else {
                    return Ok(None);
                };
                fallback
            }
        };
        self.ready = Some(action);
        Ok(Some(StoreStep::Input(action)))
    }
    pub fn prepare(&mut self, a: StoreAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.ready != Some(a) || self.pending.is_some() {
            return Err("迷失之地没有待保存的商店动作");
        }
        if matches!(a, StoreAction::Buy(_) | StoreAction::Refresh) {
            let p = self.current.as_ref().ok_or("迷失之地商店页面缺失")?;
            let offer = if let StoreAction::Buy(slot) = a {
                Some(
                    p.offers
                        .get(slot as usize)
                        .ok_or("迷失之地商店槽位无效")?
                        .clone(),
                )
            } else {
                None
            };
            let cost = offer
                .as_ref()
                .map(|o| o.price)
                .unwrap_or(p.refresh_price)
                .ok_or("迷失之地商店价格缺失")?;
            if self.permitted(p) != Some(true) || self.funds(p, cost) != Some(true) {
                return Err("迷失之地商店没有本次消费依据");
            }
            self.transaction = Some(Transaction {
                purpose: if offer.is_some() {
                    StorePurpose::Buy
                } else {
                    StorePurpose::Refresh
                },
                offer,
                cost,
                before: p.clone(),
                confirm_issued: false,
                child: None,
                child_returned: false,
            });
        }
        if a == StoreAction::Confirm {
            self.transaction
                .as_mut()
                .ok_or("迷失之地商店确认缺少交易")?
                .confirm_issued = true;
        }
        self.ready = None;
        self.pending = Some((a, false));
        self.validate()
    }
    pub fn allows(&self, a: StoreAction, v: &LostVoidStoreView) -> bool {
        if self.validate().is_err() || self.pending != Some((a, false)) {
            return false;
        }
        if a == StoreAction::Confirm {
            return v.scene.screen == S::Modal
                && v.dialog
                    .as_ref()
                    .zip(self.transaction.as_ref())
                    .is_some_and(|(d, t)| {
                        d.purpose == t.purpose
                            && d.costs.iter().filter(|p| **p == t.cost).count() == 1
                    });
        }
        v.scene.screen == S::Store
            && v.page.as_ref() == self.current.as_ref()
            && self.current.as_ref().is_some_and(|p| match a {
                StoreAction::Back => p.back,
                StoreAction::Scroll => true,
                StoreAction::Buy(slot) => {
                    p.offers.get(slot as usize).is_some_and(|o| {
                        o.available
                            && o.price
                                .is_some_and(|cost| self.funds(p, cost) == Some(true))
                    }) && self.permitted(p) == Some(true)
                }
                StoreAction::Refresh => {
                    p.refresh_enabled
                        && p.refresh_price
                            .is_some_and(|cost| self.funds(p, cost) == Some(true))
                        && self.permitted(p) == Some(true)
                }
                StoreAction::Confirm => false,
            })
    }
    pub fn posted(&mut self, a: StoreAction) -> Result<(), &'static str> {
        if self.pending != Some((a, false)) {
            return Err("迷失之地商店投递回执不匹配");
        }
        self.pending = Some((a, true));
        self.validate()
    }
    pub fn partial_scroll(&mut self, a: StoreAction) -> Result<(), &'static str> {
        if a != StoreAction::Scroll {
            return Err("只有商店查看动作可以报告半次滚动");
        }
        self.not_posted(a)
    }
    pub fn not_posted(&mut self, a: StoreAction) -> Result<(), &'static str> {
        if self.pending != Some((a, false)) {
            return Err("迷失之地已投递商店动作不能撤销");
        }
        self.pending = None;
        match a {
            StoreAction::Buy(_) | StoreAction::Refresh => self.transaction = None,
            StoreAction::Confirm => {
                if let Some(t) = &mut self.transaction {
                    t.confirm_issued = false;
                }
            }
            _ => {}
        }
        self.validate()
    }
    pub fn choice_request(&self) -> Result<LostVoidChoiceRequest, &'static str> {
        self.validate()?;
        Ok(LostVoidChoiceRequest {
            run_id: self.request.run_id,
            interaction_id: self
                .transaction
                .as_ref()
                .and_then(|t| t.child)
                .ok_or("迷失之地商店选择子流程身份缺失")?,
            challenge: self.request.challenge.clone(),
            options: self.request.options.clone(),
            return_to: vec![S::Store],
            timeout_ms: self.request.timeout_ms,
        })
    }
    pub fn accept_choice(&mut self, r: &LostVoidChoiceReceipt) -> Result<(), &'static str> {
        self.validate()?;
        let t = self.transaction.as_mut().ok_or("迷失之地商店交易缺失")?;
        if t.child != Some(r.interaction_id)
            || t.child_returned
            || r.run_id != self.request.run_id
            || r.returned_to != S::Store
            || !(1..=16).contains(&r.confirmed_pages)
        {
            return Err("迷失之地商店选择回执不属于本次交易");
        }
        t.child_returned = true;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.receipt = None;
        self.ready = None;
        Ok(())
    }
}

/// 余额和价格只接受完整整数、明确乘号或“免费”，不抽取混杂描述中的数字。
pub fn parse_store_number(text: &str) -> Option<u32> {
    let t: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .replace('％', "%");
    if t == "免费" {
        return Some(0);
    }
    let stripped = t.strip_prefix(['×', 'x', 'X']).unwrap_or(&t);
    let t = stripped.strip_suffix('%').unwrap_or(stripped);
    if t.is_empty() || t.len() > 8 || !t.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    t.parse().ok()
}
