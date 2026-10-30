//! 悬赏面板的领取检查及返回；不提供挑战通关或奖励实际到账的推测。
use crate::lost_void_scene::BountyProgress;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BountyRequest {
    pub run_id: Uuid,
    pub claim_id: Uuid,
    pub source_id: Uuid,
    pub timeout_ms: u32,
}
impl BountyRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.run_id.is_nil()
            || self.claim_id.is_nil()
            || self.source_id.is_nil()
            || self.claim_id == self.run_id
            || self.claim_id == self.source_id
            || !(1000..=300_000).contains(&self.timeout_ms)
        {
            return Err("迷失之地悬赏身份或预算无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BountyPage {
    Unknown,
    EntryStandard,
    EntryPeriodic,
    Panel,
    Reward,
    Handbook,
    NormalWorld,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimState {
    Unknown,
    Available,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BountyView {
    pub page: BountyPage,
    pub panel_ready: bool,
    /// 不可用须由完整可见面板中的明确禁用状态提供，不能来自按钮漏读。
    pub claim: ClaimState,
    pub open_button: bool,
    pub close_button: bool,
    pub bounty: Option<BountyProgress>,
}
impl BountyView {
    pub fn validate(&self) -> Result<(), &'static str> {
        let entry = matches!(
            self.page,
            BountyPage::EntryStandard | BountyPage::EntryPeriodic
        );
        let invalid_panel = if self.panel_ready {
            self.page != BountyPage::Panel
        } else {
            self.claim != ClaimState::Unknown || self.close_button
        };
        if invalid_panel
            || self.open_button && !entry
            || self.bounty.is_some() && !(entry || self.panel_ready)
            || self
                .bounty
                .is_some_and(|b| b.total != 8000 || b.current > b.total)
        {
            return Err("迷失之地悬赏页面与同帧证据不一致");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BountyAction {
    Open,
    ClaimAll,
    DismissReward,
    ClosePanel,
    BackEntry,
    BackHandbook,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Dispatch {
    Intent,
    Posted,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BountyReceipt {
    pub run_id: Uuid,
    pub claim_id: Uuid,
    pub source_id: Uuid,
    pub attempted: bool,
    pub reward_seen: bool,
    pub bounty: Option<BountyProgress>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BountyFlow {
    request: BountyRequest,
    dispatch: BTreeMap<BountyAction, Dispatch>,
    opened: bool,
    checked: bool,
    blocked_available: bool,
    reward_seen: bool,
    bounty: Option<BountyProgress>,
    current: Option<BountyView>,
    stable: u8,
    stable_since: Option<u64>,
    active_ms: u64,
    offered: Option<BountyAction>,
    receipt: Option<BountyReceipt>,
}
impl BountyFlow {
    pub fn new(request: BountyRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            dispatch: BTreeMap::new(),
            opened: false,
            checked: false,
            blocked_available: false,
            reward_seen: false,
            bounty: None,
            current: None,
            stable: 0,
            stable_since: None,
            active_ms: 0,
            offered: None,
            receipt: None,
        })
    }
    pub fn request(&self) -> &BountyRequest {
        &self.request
    }
    pub fn receipt(&self) -> Option<&BountyReceipt> {
        self.receipt.as_ref()
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn checked(&self) -> bool {
        self.checked && !self.blocked_available
    }
    pub fn bounty_progress(&self) -> Option<BountyProgress> {
        self.bounty
    }
    pub fn action_intents(&self) -> Vec<BountyAction> {
        self.dispatch.keys().copied().collect()
    }
    fn action(&self, v: &BountyView) -> Option<BountyAction> {
        use BountyAction as A;
        if self.completed() {
            return None;
        }
        let a = match v.page {
            BountyPage::EntryStandard | BountyPage::EntryPeriodic
                if self.checked() && self.opened =>
            {
                A::BackEntry
            }
            BountyPage::EntryStandard | BountyPage::EntryPeriodic
                if !self.opened && v.open_button =>
            {
                A::Open
            }
            BountyPage::Panel
                if v.panel_ready && v.claim == ClaimState::Available && !self.checked() =>
            {
                A::ClaimAll
            }
            BountyPage::Panel
                if v.panel_ready
                    && v.claim == ClaimState::Unavailable
                    && self.checked()
                    && v.close_button =>
            {
                A::ClosePanel
            }
            BountyPage::Reward if self.reward_seen && self.dispatch.contains_key(&A::ClaimAll) => {
                A::DismissReward
            }
            BountyPage::Handbook if self.checked() && self.opened => A::BackHandbook,
            _ => return None,
        };
        (!self.dispatch.contains_key(&a)).then_some(a)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        use BountyAction as A;
        self.request.validate()?;
        if let Some(v) = &self.current {
            v.validate()?;
        }
        if self.stable > 2
            || self.active_ms > u64::from(self.request.timeout_ms)
            || self.current.is_some() != (self.stable > 0)
            || self.stable_since.is_some() != self.current.is_some()
            || self.stable_since.is_some_and(|t| t > self.active_ms)
            || self
                .bounty
                .is_some_and(|b| b.total != 8000 || b.current > b.total)
            || self.checked && !self.opened
            || self.blocked_available && !self.checked
            || self.reward_seen && !self.dispatch.contains_key(&A::ClaimAll)
            || self.dispatch.contains_key(&A::ClaimAll) && !self.opened
            || self.dispatch.contains_key(&A::DismissReward) && !self.reward_seen
            || [A::ClosePanel, A::BackEntry, A::BackHandbook]
                .iter()
                .any(|a| self.dispatch.contains_key(a) && !self.checked)
            || self.offered.is_some_and(|a| {
                self.stable != 2 || self.current.as_ref().and_then(|v| self.action(v)) != Some(a)
            })
        {
            return Err("迷失之地悬赏检查点与领取历史不一致");
        }
        if let Some(r) = &self.receipt
            && (r.run_id != self.request.run_id
                || r.claim_id != self.request.claim_id
                || r.source_id != self.request.source_id
                || !self.checked()
                || !self.opened
                || r.reward_seen != self.reward_seen
                || r.bounty != self.bounty
                || r.attempted != self.dispatch.contains_key(&A::ClaimAll)
                || self.stable != 2
                || self
                    .current
                    .as_ref()
                    .is_none_or(|v| v.page != BountyPage::NormalWorld))
        {
            return Err("悬赏尚未确认普通大世界返回");
        }
        Ok(())
    }
    fn clear_fresh(&mut self) {
        self.current = None;
        self.stable = 0;
        self.stable_since = None;
        self.offered = None;
        self.receipt = None;
    }
    pub fn invalidate_observation(&mut self) {
        self.clear_fresh();
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.clear_fresh();
        self.validate()
    }
    pub fn next(
        &mut self,
        v: &BountyView,
        active_ms: u64,
    ) -> Result<Option<BountyAction>, &'static str> {
        self.validate()?;
        v.validate()?;
        if active_ms < self.active_ms || active_ms > u64::from(self.request.timeout_ms) {
            return Err("悬赏活动时间回退或超过预算");
        }
        if self.completed() {
            return Ok(None);
        }
        let mut next = self.clone();
        next.active_ms = active_ms;
        next.offered = None;
        if v.page == BountyPage::Unknown {
            next.clear_fresh();
            *self = next;
            return Ok(None);
        }
        next.stable = if next.current.as_ref() == Some(v) {
            (next.stable + 1).min(2)
        } else {
            next.stable_since = Some(active_ms);
            1
        };
        next.current = Some(v.clone());
        if next.stable == 2 {
            if let Some(b) = v.bounty {
                if next.bounty.is_some_and(|old| old.current > b.current) {
                    return Err("本次悬赏检查的进度发生回退");
                }
                next.bounty = Some(b);
            }
            if v.panel_ready {
                next.opened = true;
                if v.claim == ClaimState::Unavailable
                    && active_ms.saturating_sub(next.stable_since.unwrap()) >= 1000
                {
                    next.checked = true;
                    next.blocked_available = false;
                } else if v.claim == ClaimState::Available && next.checked {
                    next.blocked_available = true;
                }
            }
            if v.page == BountyPage::Reward && next.dispatch.contains_key(&BountyAction::ClaimAll) {
                next.reward_seen = true;
            }
            if v.page == BountyPage::NormalWorld && next.checked() && next.opened {
                next.receipt = Some(BountyReceipt {
                    run_id: next.request.run_id,
                    claim_id: next.request.claim_id,
                    source_id: next.request.source_id,
                    attempted: next.dispatch.contains_key(&BountyAction::ClaimAll),
                    reward_seen: next.reward_seen,
                    bounty: next.bounty,
                });
            } else {
                next.offered = next.action(v);
            }
        }
        next.validate()?;
        let offered = next.offered;
        *self = next;
        Ok(offered)
    }
    pub fn prepare(&mut self, a: BountyAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.offered != Some(a) || self.dispatch.contains_key(&a) {
            return Err("悬赏没有对应的新帧意图");
        }
        self.offered = None;
        self.dispatch.insert(a, Dispatch::Intent);
        self.validate()
    }
    pub fn allows(&self, a: BountyAction, v: &BountyView) -> bool {
        if self.validate().is_err()
            || v.validate().is_err()
            || self.completed()
            || self.dispatch.get(&a) != Some(&Dispatch::Intent)
        {
            return false;
        }
        match a {
            BountyAction::Open => {
                matches!(
                    v.page,
                    BountyPage::EntryStandard | BountyPage::EntryPeriodic
                ) && v.open_button
                    && !self.opened
            }
            BountyAction::ClaimAll => {
                v.page == BountyPage::Panel
                    && v.panel_ready
                    && v.claim == ClaimState::Available
                    && !self.checked()
            }
            BountyAction::DismissReward => v.page == BountyPage::Reward && self.reward_seen,
            BountyAction::ClosePanel => {
                v.page == BountyPage::Panel
                    && v.panel_ready
                    && v.claim == ClaimState::Unavailable
                    && v.close_button
                    && self.checked()
            }
            BountyAction::BackEntry => {
                matches!(
                    v.page,
                    BountyPage::EntryStandard | BountyPage::EntryPeriodic
                ) && self.checked()
            }
            BountyAction::BackHandbook => v.page == BountyPage::Handbook && self.checked(),
        }
    }
    pub fn posted(&mut self, a: BountyAction) -> Result<(), &'static str> {
        self.validate()?;
        let d = self.dispatch.get_mut(&a).ok_or("悬赏缺少待投递意图")?;
        if *d == Dispatch::Unknown {
            return Err("未知悬赏输入不能改为已投递");
        }
        *d = Dispatch::Posted;
        self.validate()
    }
    pub fn unknown(&mut self, a: BountyAction) -> Result<(), &'static str> {
        self.validate()?;
        let d = self.dispatch.get_mut(&a).ok_or("悬赏缺少待投递意图")?;
        if *d == Dispatch::Posted {
            return Err("已知悬赏输入不能退回未知");
        }
        *d = Dispatch::Unknown;
        self.validate()
    }
    pub fn not_posted(&mut self, a: BountyAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.dispatch.get(&a) != Some(&Dispatch::Intent) {
            return Err("已投递或未知悬赏动作不能撤销");
        }
        let mut next = self.clone();
        next.dispatch.remove(&a);
        next.clear_fresh();
        next.validate()?;
        *self = next;
        Ok(())
    }
}
