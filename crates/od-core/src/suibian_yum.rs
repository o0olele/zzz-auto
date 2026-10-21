//! 饮茶仙订单、提交及补料编排。制造与游历使用同一次冻结配置和独立完成证明。
use crate::{
    suibian_adventure::{CommitState, DispatchOutcome, TempleAdventure},
    suibian_craft::{CraftOutcome, TempleCraft, goods_key},
    suibian_temple::{AdventureMission, SuibianTempleConfig},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum YumPhase {
    Enter,
    Submit,
    Transaction,
    Orders,
    SelectOrder,
    Materials,
    OpenMaterial,
    GoCraft,
    Craft,
    GoAdventure,
    Adventure,
    BackMaterial,
    BackOrders,
    Return,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct YumCount {
    pub value: u32,
    pub max: u32,
}
impl YumCount {
    fn valid(self) -> bool {
        self.max > 0 && self.value <= self.max && self.max <= 1_000_000
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum YumTransactionKind {
    Submit,
    Refresh,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct YumSpec {
    pub kind: YumTransactionKind,
    pub order: String,
    pub cap: YumCount,
    pub counter: YumCount,
    pub reward: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum YumTransactionState {
    Awaiting,
    NotPosted,
    Completed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct YumTransaction {
    pub spec: YumSpec,
    pub state: YumTransactionState,
    pub prompt_seen: bool,
    pub prompt_confirmed: bool,
    pub reward_seen: bool,
    pub reward_confirmed: bool,
    pub cap_after: Option<YumCount>,
    pub counter_after: Option<YumCount>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum YumCommitKind {
    Start,
    ConfirmPrompt,
    ConfirmReward,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct YumCommit {
    pub transaction: u16,
    pub kind: YumCommitKind,
    pub state: CommitState,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct YumOrder {
    pub name: String,
    pub rows: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct YumMaterial {
    pub order: u16,
    pub row: u8,
    pub name: String,
    pub mission: Option<String>,
    pub craft_unavailable: bool,
    pub craft: Option<Box<TempleCraft>>,
    pub adventure: Option<Box<TempleAdventure>>,
    pub craft_ms: u64,
    pub adventure_ms: u64,
    pub done: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YumNeed {
    pub row: u8,
    pub have: u32,
    pub need: u32,
    pub red: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YumPage {
    pub regular: bool,
    pub cap: Option<YumCount>,
    pub counter: Option<YumCount>,
    pub submit: Option<bool>,
    pub refresh: Option<bool>,
    pub reward: Option<u32>,
    pub orders: Vec<String>,
    pub list_readable: bool,
    pub selected: Option<String>,
    pub needs: Vec<YumNeed>,
    pub materials_readable: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YumMenu {
    pub name: String,
    pub craft: bool,
    pub mission: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YumView {
    Unknown,
    Entry,
    Managed,
    Shops,
    Page(YumPage),
    Prompt(YumTransactionKind),
    Reward,
    Material(YumMenu),
    Craft(String),
    CraftUnavailable,
    Adventure(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YumAction {
    OpenShops,
    OpenYum,
    Regular,
    Submit(YumSpec),
    Refresh(YumSpec),
    ConfirmPrompt,
    ConfirmReward,
    SelectOrder(String),
    Scroll,
    OpenMaterial(u8),
    OpenCraft,
    OpenAdventure,
    Back,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TempleYum {
    pub config: SuibianTempleConfig,
    pub submit_only: bool,
    pub phase: YumPhase,
    pub transactions: Vec<YumTransaction>,
    pub commits: Vec<YumCommit>,
    pub active: Option<u16>,
    pub pending: Option<u16>,
    pub orders: Vec<YumOrder>,
    pub current_order: Option<u16>,
    pub current_row: Option<u8>,
    pub materials: Vec<YumMaterial>,
    pub current_material: Option<u16>,
    pub last_cap: Option<YumCount>,
    pub refresh_direction: Option<i8>,
    pub idle_scans: u8,
    pub scrolls: u8,
    pub after_scroll: bool,
    pub unchanged: u8,
    pub last_orders: Vec<String>,
    pub scanned_end: bool,
    pub at_limit: bool,
    pub skip_adventure: bool,
    pub returned_entry: bool,
}
pub fn order_key(text: &str) -> Option<String> {
    let word: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let word = word.trim_start_matches(|c: char| !('一'..='鿿').contains(&c));
    (word.chars().count() >= 2
        && word.chars().count() <= 64
        && !word.contains("随便观货品")
        && !word.contains("精粹货品")
        && !word.contains("精淬货品")
        && !word.chars().any(char::is_control))
    .then(|| word.to_owned())
}
fn valid_name(name: &str) -> bool {
    goods_key(name).as_deref() == Some(name)
}
fn valid_order(name: &str) -> bool {
    order_key(name).as_deref() == Some(name)
}
impl TempleYum {
    pub fn new(config: SuibianTempleConfig, submit_only: bool) -> Result<Self, &'static str> {
        let f = Self {
            config,
            submit_only,
            phase: YumPhase::Enter,
            transactions: vec![],
            commits: vec![],
            active: None,
            pending: None,
            orders: vec![],
            current_order: None,
            current_row: None,
            materials: vec![],
            current_material: None,
            last_cap: None,
            refresh_direction: None,
            idle_scans: 0,
            scrolls: 0,
            after_scroll: false,
            unchanged: 0,
            last_orders: vec![],
            scanned_end: false,
            at_limit: false,
            skip_adventure: false,
            returned_entry: false,
        };
        f.validate()?;
        Ok(f)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate()?;
        if self.transactions.len() > 256
            || self.commits.len() > 1024
            || self.orders.len() > 128
            || self.materials.len() > 384
            || self.orders.iter().enumerate().any(|(i, o)| {
                !valid_order(&o.name)
                    || self.orders[..i].iter().any(|p| p.name == o.name)
                    || o.rows.len() > 3
                    || o.rows
                        .iter()
                        .enumerate()
                        .any(|(j, n)| !(1..=3).contains(n) || o.rows[..j].contains(n))
            })
            || self
                .current_order
                .is_some_and(|n| usize::from(n) >= self.orders.len())
            || self.current_row.is_some_and(|r| !(1..=3).contains(&r))
            || self
                .current_material
                .is_some_and(|i| usize::from(i) >= self.materials.len())
            || self.last_cap.is_some_and(|c| !c.valid())
            || self.refresh_direction.is_some_and(|n| n != 1 && n != -1)
            || self.idle_scans > 2
            || self.unchanged > 2
            || self.scrolls > 64
            || self.last_orders.len() > 8
            || self.last_orders.iter().any(|n| !valid_order(n))
            || self.returned_entry != (self.phase == YumPhase::Done)
            || ((self.phase == YumPhase::Transaction) != self.active.is_some())
            || (self.phase == YumPhase::Done
                && (!self.at_limit && !self.scanned_end
                    || self.pending.is_some()
                    || self.materials.iter().any(|m| !m.done)))
        {
            return Err("饮茶仙检查点缺少冻结顺序、目标或返回证明");
        }
        for (i, t) in self.transactions.iter().enumerate() {
            if !valid_order(&t.spec.order)
                || !t.spec.cap.valid()
                || !t.spec.counter.valid()
                || t.spec.counter.max > 30
                || t.spec.cap.value >= t.spec.cap.max
                || t.spec.reward == 0
                || t.spec.reward > 1_000_000
                || (t.spec.kind == YumTransactionKind::Refresh
                    && !self.config.yum_cha_sin_period_refresh)
                || (t.state == YumTransactionState::Awaiting && self.active != Some(i as u16))
                || (t.prompt_confirmed && !t.prompt_seen)
                || (t.reward_confirmed && !t.reward_seen)
                || (t.reward_seen && t.spec.kind != YumTransactionKind::Submit)
                || (t.state == YumTransactionState::Completed
                    && (t.cap_after.is_none()
                        || t.counter_after.is_none()
                        || t.prompt_seen && !t.prompt_confirmed
                        || t.reward_seen && !t.reward_confirmed))
            {
                return Err("饮茶仙提交缺少归属、确认或实际计数后态");
            }
            if t.state == YumTransactionState::Completed {
                let cap = t.cap_after.unwrap();
                let counter = t.counter_after.unwrap();
                if !Self::matches_after(&t.spec, cap, counter)
                    || !self.commits.iter().any(|c| {
                        usize::from(c.transaction) == i
                            && c.kind == YumCommitKind::Start
                            && c.state == CommitState::Observed
                    })
                    || (t.prompt_confirmed
                        && !self.commits.iter().any(|c| {
                            usize::from(c.transaction) == i
                                && c.kind == YumCommitKind::ConfirmPrompt
                                && c.state == CommitState::Observed
                        }))
                    || (t.reward_confirmed
                        && !self.commits.iter().any(|c| {
                            usize::from(c.transaction) == i
                                && c.kind == YumCommitKind::ConfirmReward
                                && c.state == CommitState::Observed
                        }))
                {
                    return Err("饮茶仙完成结果没有本次投递及单次计数变化");
                }
            }
        }
        if self.active.is_some_and(|i| {
            usize::from(i) + 1 != self.transactions.len()
                || self.transactions[usize::from(i)].state != YumTransactionState::Awaiting
        }) || self.commits.iter().enumerate().any(|(i, c)| {
            usize::from(c.transaction) >= self.transactions.len()
                || (c.state == CommitState::Intent && self.pending != Some(i as u16))
        }) || self.pending.is_some_and(|i| {
            usize::from(i) + 1 != self.commits.len()
                || self.commits[usize::from(i)].state != CommitState::Intent
                || self.active != Some(self.commits[usize::from(i)].transaction)
        }) {
            return Err("饮茶仙待确认提交不属于当前事务");
        }
        for (i, m) in self.materials.iter().enumerate() {
            if usize::from(m.order) >= self.orders.len()
                || !self.orders[usize::from(m.order)].rows.contains(&m.row)
                || !valid_name(&m.name)
                || self.materials[..i].iter().any(|o| o.name == m.name)
                || m.mission
                    .as_ref()
                    .is_some_and(|s| AdventureMission::parse(s).is_none())
                || (m.craft.is_some() && m.craft_unavailable)
                || m.adventure.is_some() && m.mission.is_none()
            {
                return Err("饮茶仙补料没有本次订单、行号或去重材料");
            }
            if let Some(c) = &m.craft {
                c.validate()?;
                if c.config != self.config
                    || c.material.as_ref() != Some(&m.name)
                    || c.mode != crate::suibian_craft::CraftMode::Material
                    || (m.done && !c.completed())
                {
                    return Err("补料制造不属于同一冻结设置或尚未完成");
                }
            }
            if let Some(a) = &m.adventure {
                a.validate()?;
                if a.config != self.config
                    || a.material_mission != m.mission
                    || a.mode != crate::suibian_adventure::AdventureMode::Material
                    || (m.done && !a.completed())
                    || Self::craft_started(m)
                {
                    return Err("补料游历与制造结果冲突或尚未完成");
                }
            }
        }
        if matches!(
            self.phase,
            YumPhase::GoCraft
                | YumPhase::Craft
                | YumPhase::GoAdventure
                | YumPhase::Adventure
                | YumPhase::BackMaterial
        ) && self.current_material.is_none()
        {
            return Err("补料阶段没有当前材料");
        }
        if self.phase == YumPhase::Craft && self.material().is_none_or(|m| m.craft.is_none())
            || self.phase == YumPhase::Adventure
                && self.material().is_none_or(|m| m.adventure.is_none())
        {
            return Err("饮茶仙缺少当前补料子流程");
        }
        Ok(())
    }
    fn matches_after(s: &YumSpec, cap: YumCount, counter: YumCount) -> bool {
        if !cap.valid() || !counter.valid() || cap.max != s.cap.max || counter.max != s.counter.max
        {
            return false;
        }
        match s.kind {
            YumTransactionKind::Submit => {
                cap.value == s.cap.value.saturating_add(s.reward).min(s.cap.max)
                    && counter == s.counter
            }
            YumTransactionKind::Refresh => {
                cap == s.cap && counter.value.abs_diff(s.counter.value) == 1
            }
        }
    }
    fn material(&self) -> Option<&YumMaterial> {
        self.current_material
            .and_then(|i| self.materials.get(usize::from(i)))
    }
    fn craft_started(m: &YumMaterial) -> bool {
        m.craft.as_ref().is_some_and(|c| {
            c.completed() && c.results.iter().any(|r| r.outcome == CraftOutcome::Started)
        })
    }
    pub fn completed(&self) -> bool {
        self.phase == YumPhase::Done && self.validate().is_ok()
    }
    pub fn check_update(&self, n: &Self) -> Result<(), &'static str> {
        self.validate()?;
        n.validate()?;
        if !self.skip_adventure
            && n.skip_adventure
            && !(self.phase == YumPhase::Adventure
                && self
                    .material()
                    .and_then(|m| m.adventure.as_ref())
                    .is_some_and(|a| {
                        a.completed()
                            && a.dispatches.iter().any(|r| {
                                matches!(
                                    r.outcome,
                                    DispatchOutcome::NoTeams | DispatchOutcome::InsufficientEnergy
                                )
                            })
                    }))
        {
            return Err("跳过后续补料游历缺少明确容量或电量结果");
        }
        if self.config != n.config
            || self.submit_only != n.submit_only
            || n.transactions.len() < self.transactions.len()
            || n.transactions.len() > self.transactions.len() + 1
            || n.commits.len() < self.commits.len()
            || n.commits.len() > self.commits.len() + 1
            || n.orders.len() < self.orders.len()
            || n.orders.len() > self.orders.len() + 1
            || n.materials.len() < self.materials.len()
            || n.materials.len() > self.materials.len() + 1
            || self.skip_adventure && !n.skip_adventure
            || self.at_limit && !n.at_limit
            || self.scanned_end && !n.scanned_end
            || self.scrolls > n.scrolls
            || self
                .refresh_direction
                .is_some_and(|d| n.refresh_direction != Some(d))
            || self.last_cap.is_some_and(|c| {
                n.last_cap
                    .is_none_or(|v| v.max != c.max || v.value < c.value)
            })
            || (self.completed() && self != n)
        {
            return Err("不能改写饮茶仙冻结参数或已完成证明");
        }
        for (old, new) in self.commits.iter().zip(&n.commits) {
            let mut same = old.clone();
            same.state = new.state;
            if same != *new || (old.state != CommitState::Intent && old != new) {
                return Err("饮茶仙提交回执不能改写");
            }
        }
        if n.commits.len() > self.commits.len()
            && (self.pending.is_some()
                || n.commits
                    .last()
                    .is_none_or(|c| c.state != CommitState::Intent))
        {
            return Err("饮茶仙须先保存新的提交意图");
        }
        for (i, (old, new)) in self.transactions.iter().zip(&n.transactions).enumerate() {
            let observed = |kinds: &[YumCommitKind]| {
                self.pending.is_some_and(|p| {
                    let c = &self.commits[usize::from(p)];
                    usize::from(c.transaction) == i
                        && kinds.contains(&c.kind)
                        && n.commits[usize::from(p)].state == CommitState::Observed
                })
            };
            if (!old.prompt_seen && new.prompt_seen && !observed(&[YumCommitKind::Start]))
                || (!old.prompt_confirmed
                    && new.prompt_confirmed
                    && !observed(&[YumCommitKind::ConfirmPrompt]))
                || (!old.reward_seen
                    && new.reward_seen
                    && !observed(&[YumCommitKind::Start, YumCommitKind::ConfirmPrompt]))
                || (!old.reward_confirmed
                    && new.reward_confirmed
                    && !observed(&[YumCommitKind::ConfirmReward]))
            {
                return Err("饮茶仙弹窗事实没有本次提交回执");
            }
            if old.spec != new.spec
                || (old.state != YumTransactionState::Awaiting && old != new)
                || old.prompt_seen && !new.prompt_seen
                || old.prompt_confirmed && !new.prompt_confirmed
                || old.reward_seen && !new.reward_seen
                || old.reward_confirmed && !new.reward_confirmed
            {
                return Err("饮茶仙事务事实不能撤回");
            }
            if old.state != new.state
                && !self.pending.is_some_and(|p| {
                    let c = &self.commits[usize::from(p)];
                    usize::from(c.transaction) == i
                        && (new.state != YumTransactionState::NotPosted
                            || c.kind == YumCommitKind::Start)
                        && n.commits[usize::from(p)].state
                            == if new.state == YumTransactionState::NotPosted {
                                CommitState::NotPosted
                            } else {
                                CommitState::Observed
                            }
                })
            {
                return Err("饮茶仙事务结果没有对应回执");
            }
        }
        if n.transactions.len() > self.transactions.len()
            && (self.active.is_some()
                || n.transactions.last().is_none_or(|t| {
                    t.state != YumTransactionState::Awaiting || t.prompt_seen || t.reward_seen
                }))
        {
            return Err("新饮茶仙事务不能携带成功证明");
        }
        for (old, new) in self.orders.iter().zip(&n.orders) {
            if old.name != new.name
                || !new.rows.starts_with(&old.rows)
                || new.rows.len() > old.rows.len() + 1
            {
                return Err("不能改写已检查的订单材料行");
            }
        }
        if n.orders.len() > self.orders.len() && n.orders.last().is_none_or(|o| !o.rows.is_empty())
        {
            return Err("新订单不能携带已处理材料行");
        }
        if n.materials.len() > self.materials.len()
            && n.materials.last().is_none_or(|m| {
                self.phase != YumPhase::OpenMaterial
                    || Some(m.order) != self.current_order
                    || Some(m.row) != self.current_row
                    || m.craft.is_some()
                    || m.adventure.is_some()
                    || m.craft_ms != 0
                    || m.adventure_ms != 0
                    || m.done
            })
        {
            return Err("新补料必须来自本次选中的订单行");
        }
        for (old, new) in self.materials.iter().zip(&n.materials) {
            if old.order != new.order
                || old.row != new.row
                || old.name != new.name
                || old.mission != new.mission
                || old.craft_unavailable && !new.craft_unavailable
                || old.done && !new.done
                || old.craft_ms > new.craft_ms
                || old.adventure_ms > new.adventure_ms
                || (old.done && old != new)
            {
                return Err("不能改写饮茶仙材料或活动预算");
            }
            match (&old.craft, &new.craft) {
                (Some(o), Some(v)) => o.check_update(v)?,
                (Some(_), None) => return Err("不能删除制造子流程"),
                (None, Some(v))
                    if self.phase != YumPhase::GoCraft
                        || n.phase != YumPhase::Craft
                        || **v != TempleCraft::material(self.config.clone(), old.name.clone())? =>
                {
                    return Err("补料制造须从冻结设置开始");
                }
                _ => {}
            }
            match (&old.adventure, &new.adventure) {
                (Some(o), Some(v)) => o.check_update(v)?,
                (Some(_), None) => return Err("不能删除游历子流程"),
                (None, Some(v))
                    if self.phase != YumPhase::GoAdventure
                        || n.phase != YumPhase::Adventure
                        || **v
                            != TempleAdventure::material(
                                self.config.clone(),
                                old.mission.clone().ok_or("无补料副本")?,
                            )? =>
                {
                    return Err("补料游历须从冻结设置开始");
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn edit(
        &mut self,
        change: impl FnOnce(&mut Self) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let mut n = self.clone();
        change(&mut n)?;
        self.check_update(&n)?;
        *self = n;
        Ok(())
    }
    pub fn update_craft(&mut self, flow: TempleCraft, ms: u64) -> Result<(), &'static str> {
        self.edit(|s| {
            if s.phase != YumPhase::Craft {
                return Err("不是补料制造阶段");
            }
            let m = &mut s.materials[usize::from(s.current_material.ok_or("无当前材料")?)];
            m.craft = Some(Box::new(flow));
            m.craft_ms = ms;
            Ok(())
        })
    }
    pub fn update_adventure(&mut self, flow: TempleAdventure, ms: u64) -> Result<(), &'static str> {
        self.edit(|s| {
            if s.phase != YumPhase::Adventure {
                return Err("不是补料游历阶段");
            }
            let m = &mut s.materials[usize::from(s.current_material.ok_or("无当前材料")?)];
            m.adventure = Some(Box::new(flow));
            m.adventure_ms = ms;
            Ok(())
        })
    }
    pub fn finish_child(&mut self) -> Result<(), &'static str> {
        self.edit(|s| {
            let m = s.material().ok_or("没有补料子流程")?;
            if s.phase == YumPhase::Craft {
                if m.craft
                    .as_ref()
                    .is_none_or(|c| !c.completed() || !c.returned_material)
                {
                    return Err("补料制造尚未安全返回");
                }
                s.phase = if Self::craft_started(m) {
                    YumPhase::BackOrders
                } else {
                    YumPhase::GoAdventure
                };
            } else if s.phase == YumPhase::Adventure {
                let a = m
                    .adventure
                    .as_ref()
                    .filter(|a| a.completed())
                    .ok_or("补料游历尚未完成")?;
                if a.dispatches.iter().any(|r| {
                    matches!(
                        r.outcome,
                        DispatchOutcome::NoTeams | DispatchOutcome::InsufficientEnergy
                    )
                }) {
                    s.skip_adventure = true;
                }
                s.phase = YumPhase::BackMaterial;
            } else {
                return Err("当前不是补料子流程");
            }
            Ok(())
        })
    }
    pub fn prepare(&mut self, action: &YumAction) -> Result<(), &'static str> {
        self.edit(|s| {
            if s.pending.is_some() || s.commits.len() >= 1024 {
                return Err("饮茶仙提交尚未确认或次数已满");
            }
            let kind = match action {
                YumAction::Submit(spec) | YumAction::Refresh(spec) => {
                    let expected = if matches!(action, YumAction::Submit(_)) {
                        YumTransactionKind::Submit
                    } else {
                        YumTransactionKind::Refresh
                    };
                    if s.phase != YumPhase::Submit
                        || s.active.is_some()
                        || spec.kind != expected
                        || s.transactions.len() >= 256
                    {
                        return Err("饮茶仙事务阶段不符");
                    }
                    s.active = Some(s.transactions.len() as u16);
                    s.transactions.push(YumTransaction {
                        spec: spec.clone(),
                        state: YumTransactionState::Awaiting,
                        prompt_seen: false,
                        prompt_confirmed: false,
                        reward_seen: false,
                        reward_confirmed: false,
                        cap_after: None,
                        counter_after: None,
                    });
                    s.phase = YumPhase::Transaction;
                    YumCommitKind::Start
                }
                YumAction::ConfirmPrompt => {
                    let t = s
                        .active
                        .and_then(|i| s.transactions.get(usize::from(i)))
                        .ok_or("没有当前确认事务")?;
                    if !t.prompt_seen || t.prompt_confirmed {
                        return Err("确认不属于当前饮茶仙事务");
                    }
                    YumCommitKind::ConfirmPrompt
                }
                YumAction::ConfirmReward => {
                    let t = s
                        .active
                        .and_then(|i| s.transactions.get(usize::from(i)))
                        .ok_or("没有当前奖励")?;
                    if !t.reward_seen || t.reward_confirmed {
                        return Err("奖励确认未获授权");
                    }
                    YumCommitKind::ConfirmReward
                }
                _ => return Ok(()),
            };
            s.pending = Some(s.commits.len() as u16);
            s.commits.push(YumCommit {
                transaction: s.active.ok_or("无当前事务")?,
                kind,
                state: CommitState::Intent,
            });
            Ok(())
        })
    }
    pub fn not_posted(&mut self) -> Result<(), &'static str> {
        self.edit(|s| {
            if let Some(p) = s.pending.take() {
                let c = &mut s.commits[usize::from(p)];
                c.state = CommitState::NotPosted;
                if c.kind == YumCommitKind::Start {
                    s.transactions[usize::from(c.transaction)].state =
                        YumTransactionState::NotPosted;
                    s.active = None;
                    s.phase = YumPhase::Submit;
                }
            }
            Ok(())
        })
    }
    pub fn allows(&self, action: &YumAction, view: &YumView) -> bool {
        let mut p = self.clone();
        if let Some(i) = p.pending.take() {
            let c = p.commits[usize::from(i)].clone();
            let expected = match action {
                YumAction::Submit(_) | YumAction::Refresh(_) => Some(YumCommitKind::Start),
                YumAction::ConfirmPrompt => Some(YumCommitKind::ConfirmPrompt),
                YumAction::ConfirmReward => Some(YumCommitKind::ConfirmReward),
                _ => None,
            };
            if expected != Some(c.kind) {
                return false;
            }
            p.commits.pop();
            if c.kind == YumCommitKind::Start {
                p.transactions.pop();
                p.active = None;
                p.phase = YumPhase::Submit;
            }
        } else if matches!(
            action,
            YumAction::Submit(_)
                | YumAction::Refresh(_)
                | YumAction::ConfirmPrompt
                | YumAction::ConfirmReward
        ) {
            return false;
        }
        p.next(view).is_ok_and(|a| a.as_ref() == Some(action))
    }
    pub fn posted_navigation(&mut self, action: &YumAction) -> Result<(), &'static str> {
        self.edit(|s| {
            if *action == YumAction::Scroll {
                if s.scrolls >= 64 {
                    return Err("订单下拉次数超限");
                }
                s.scrolls += 1;
                s.after_scroll = true;
                s.unchanged = 0;
            }
            Ok(())
        })
    }
    fn mark_observed(&mut self) {
        let p = self.pending.take().expect("当前提交存在");
        self.commits[usize::from(p)].state = CommitState::Observed;
    }
    fn transaction(&mut self, v: &YumView) -> Result<Option<YumAction>, &'static str> {
        let index = usize::from(self.active.ok_or("无饮茶仙事务")?);
        let t = self.transactions[index].clone();
        if let Some(p) = self.pending {
            let kind = self.commits[usize::from(p)].kind;
            match v {
                YumView::Prompt(k) if kind == YumCommitKind::Start && *k == t.spec.kind => {
                    self.mark_observed();
                    self.transactions[index].prompt_seen = true;
                    return Ok(None);
                }
                YumView::Reward
                    if t.spec.kind == YumTransactionKind::Submit
                        && matches!(kind, YumCommitKind::Start | YumCommitKind::ConfirmPrompt) =>
                {
                    self.mark_observed();
                    let tx = &mut self.transactions[index];
                    if kind == YumCommitKind::ConfirmPrompt {
                        tx.prompt_confirmed = true;
                    }
                    tx.reward_seen = true;
                    return Ok(None);
                }
                YumView::Page(page) if page.regular => {
                    let (Some(cap), Some(counter)) = (page.cap, page.counter) else {
                        return Ok(None);
                    };
                    if !Self::matches_after(&t.spec, cap, counter) {
                        return Ok(None);
                    }
                    if t.spec.kind == YumTransactionKind::Refresh {
                        let direction = if counter.value > t.spec.counter.value {
                            1
                        } else {
                            -1
                        };
                        if self.refresh_direction.is_some_and(|d| d != direction) {
                            return Err("刷新计数方向发生变化");
                        }
                        self.refresh_direction = Some(direction);
                    }
                    self.mark_observed();
                    let tx = &mut self.transactions[index];
                    if kind == YumCommitKind::ConfirmPrompt {
                        tx.prompt_confirmed = true;
                    }
                    if kind == YumCommitKind::ConfirmReward {
                        tx.reward_confirmed = true;
                    }
                    tx.state = YumTransactionState::Completed;
                    tx.cap_after = Some(cap);
                    tx.counter_after = Some(counter);
                    self.last_cap = Some(cap);
                    self.active = None;
                    self.phase = YumPhase::Submit;
                    self.idle_scans = 0;
                    return Ok(None);
                }
                _ => return Ok(None),
            }
        }
        if t.prompt_seen && !t.prompt_confirmed {
            return Ok(matches!(v,YumView::Prompt(k)if *k==t.spec.kind)
                .then_some(YumAction::ConfirmPrompt));
        }
        if t.reward_seen && !t.reward_confirmed {
            return Ok(matches!(v, YumView::Reward).then_some(YumAction::ConfirmReward));
        }
        Ok(None)
    }
    pub fn next(&mut self, view: &YumView) -> Result<Option<YumAction>, &'static str> {
        self.validate()?;
        let before = self.clone();
        let result = self.advance(view);
        if result.is_err() {
            *self = before;
            return result;
        }
        if let Err(e) = before.check_update(self) {
            *self = before;
            return Err(e);
        }
        result
    }
    fn advance(&mut self, v: &YumView) -> Result<Option<YumAction>, &'static str> {
        use YumAction as A;
        use YumPhase as P;
        use YumView as V;
        if matches!(v, V::Unknown) || self.completed() {
            return Ok(None);
        }
        if self.active.is_some() {
            return self.transaction(v);
        }
        if let V::Page(p) = v
            && p.regular
            && let Some(c) = p.cap
        {
            if !c.valid()
                || self
                    .last_cap
                    .is_some_and(|o| o.max != c.max || o.value > c.value)
            {
                return Err("饮茶仙周期计数倒退或无效");
            }
            self.last_cap = Some(c);
        }
        for _ in 0..6 {
            match self.phase {
                P::Enter => match v {
                    V::Entry => return Ok(Some(A::OpenShops)),
                    V::Shops => return Ok(Some(A::OpenYum)),
                    V::Managed => return Err("托管中不能手动饮茶仙"),
                    V::Page(p) => {
                        if !p.regular {
                            return Ok(Some(A::Regular));
                        }
                        self.phase = P::Submit;
                    }
                    _ => return Err("饮茶仙起点不是入口或订单页"),
                },
                P::Submit => {
                    if let V::Page(p) = v {
                        if !p.regular {
                            return Ok(Some(A::Regular));
                        }
                        let (Some(cap), Some(counter)) = (p.cap, p.counter) else {
                            return Ok(None);
                        };
                        if cap.value == cap.max {
                            self.at_limit = true;
                            self.phase = P::Return;
                            continue;
                        }
                        let name = p.selected.as_ref().filter(|n| valid_order(n));
                        if p.submit == Some(true) {
                            if p.materials_readable
                                && !p.needs.is_empty()
                                && p.needs
                                    .iter()
                                    .all(|r| r.need > 0 && r.have >= r.need && !r.red)
                                && let (Some(name), Some(reward)) =
                                    (name, p.reward.filter(|n| *n > 0))
                            {
                                return Ok(Some(A::Submit(YumSpec {
                                    kind: YumTransactionKind::Submit,
                                    order: name.clone(),
                                    cap,
                                    counter,
                                    reward,
                                })));
                            }
                            return Ok(None);
                        }
                        if p.submit.is_none() {
                            return Ok(None);
                        }
                        if self.config.yum_cha_sin_period_refresh
                            && p.refresh == Some(true)
                            && self
                                .transactions
                                .iter()
                                .filter(|t| {
                                    t.spec.kind == YumTransactionKind::Refresh
                                        && t.state == YumTransactionState::Completed
                                })
                                .count()
                                < 30
                        {
                            if let Some(name) = name {
                                return Ok(Some(A::Refresh(YumSpec {
                                    kind: YumTransactionKind::Refresh,
                                    order: name.clone(),
                                    cap,
                                    counter,
                                    reward: 1,
                                })));
                            }
                            return Ok(None);
                        }
                        if self.config.yum_cha_sin_period_refresh && p.refresh.is_none() {
                            return Ok(None);
                        }
                        self.idle_scans = (self.idle_scans + 1).min(2);
                        if self.idle_scans < 2 {
                            return Ok(None);
                        }
                        if self.submit_only {
                            self.scanned_end = true;
                            self.phase = P::Return;
                        } else {
                            self.phase = P::Orders;
                        }
                    } else {
                        return Ok(None);
                    }
                }
                P::Orders => {
                    if let V::Page(p) = v {
                        if !p.regular {
                            return Ok(Some(A::Regular));
                        }
                        if !p.list_readable {
                            return Ok(None);
                        }
                        if p.orders.len() > 8 || p.orders.iter().any(|n| !valid_order(n)) {
                            return Err("定期采办订单列表无效");
                        }
                        if let Some(name) = p
                            .orders
                            .iter()
                            .find(|n| !self.orders.iter().any(|o| &o.name == *n))
                        {
                            if self.orders.len() >= 128 {
                                return Err("饮茶仙订单数量超限");
                            }
                            self.current_order = Some(self.orders.len() as u16);
                            self.orders.push(YumOrder {
                                name: name.clone(),
                                rows: vec![],
                            });
                            self.phase = P::SelectOrder;
                            if p.selected.as_ref() == Some(name) {
                                return Ok(None);
                            }
                            return Ok(Some(A::SelectOrder(name.clone())));
                        }
                        if self.after_scroll && p.orders == self.last_orders {
                            self.unchanged = (self.unchanged + 1).min(2);
                            if self.unchanged < 2 {
                                return Ok(None);
                            }
                            self.scanned_end = true;
                            self.phase = P::Return;
                            continue;
                        }
                        self.last_orders = p.orders.clone();
                        self.after_scroll = false;
                        if self.scrolls >= 64 {
                            return Err("订单查找达到上限，未证明列表结束");
                        }
                        return Ok(Some(A::Scroll));
                    } else {
                        return Ok(None);
                    }
                }
                P::SelectOrder | P::Materials => {
                    if let V::Page(p) = v {
                        let i = usize::from(self.current_order.ok_or("缺少当前订单")?);
                        let name = &self.orders[i].name;
                        if !p.regular {
                            return Ok(Some(A::Regular));
                        }
                        if p.selected.as_ref() != Some(name) {
                            return Ok(p
                                .orders
                                .contains(name)
                                .then(|| A::SelectOrder(name.clone())));
                        }
                        self.phase = P::Materials;
                        if !p.materials_readable {
                            return Ok(None);
                        }
                        if p.needs.iter().any(|r| {
                            !(1..=3).contains(&r.row) || r.need == 0 || (r.red != (r.have < r.need))
                        }) {
                            return Err("缺料数量与红字行冲突");
                        }
                        if let Some(need) = p
                            .needs
                            .iter()
                            .find(|r| r.red && !self.orders[i].rows.contains(&r.row))
                        {
                            self.current_row = Some(need.row);
                            self.phase = P::OpenMaterial;
                            return Ok(Some(A::OpenMaterial(need.row)));
                        }
                        self.current_order = None;
                        self.current_row = None;
                        self.phase = P::Orders;
                    } else {
                        return Ok(None);
                    }
                }
                P::OpenMaterial => match v {
                    V::Page(p) => {
                        let i = usize::from(self.current_order.ok_or("无当前订单")?);
                        let row = self.current_row.ok_or("无当前材料行")?;
                        return Ok((p.selected.as_ref() == Some(&self.orders[i].name)
                            && p.needs
                                .iter()
                                .any(|r| r.row == row && r.red && r.have < r.need))
                        .then_some(A::OpenMaterial(row)));
                    }
                    V::Material(menu) => {
                        if !valid_name(&menu.name)
                            || menu
                                .mission
                                .as_ref()
                                .is_some_and(|m| AdventureMission::parse(m).is_none())
                        {
                            return Err("补料菜单名称或副本无效");
                        }
                        let order = self.current_order.ok_or("无当前订单")?;
                        let row = self.current_row.ok_or("无当前材料行")?;
                        self.orders[usize::from(order)].rows.push(row);
                        if let Some(i) = self.materials.iter().position(|m| m.name == menu.name) {
                            self.current_material = Some(i as u16);
                            self.phase = P::BackOrders;
                            return Ok(None);
                        }
                        self.current_material = Some(self.materials.len() as u16);
                        self.materials.push(YumMaterial {
                            order,
                            row,
                            name: menu.name.clone(),
                            mission: menu.mission.clone(),
                            craft_unavailable: !menu.craft,
                            craft: None,
                            adventure: None,
                            craft_ms: 0,
                            adventure_ms: 0,
                            done: false,
                        });
                        self.phase = if menu.craft {
                            P::GoCraft
                        } else {
                            P::GoAdventure
                        };
                        return Ok(None);
                    }
                    _ => return Ok(None),
                },
                P::GoCraft => match v {
                    V::Material(menu) if self.material().is_some_and(|m| m.name == menu.name) => {
                        return Ok(menu.craft.then_some(A::OpenCraft));
                    }
                    V::Craft(name) => {
                        let i = usize::from(self.current_material.ok_or("无当前材料")?);
                        if self.materials[i].name != *name {
                            return Err("制造不是所选缺料商品");
                        }
                        self.materials[i].craft = Some(Box::new(TempleCraft::material(
                            self.config.clone(),
                            name.clone(),
                        )?));
                        self.phase = P::Craft;
                        return Ok(None);
                    }
                    V::CraftUnavailable => {
                        let i = usize::from(self.current_material.unwrap());
                        self.materials[i].craft_unavailable = true;
                        self.phase = P::BackMaterial;
                        return Ok(None);
                    }
                    _ => return Ok(None),
                },
                P::GoAdventure => {
                    let m = self.material().ok_or("无补料材料")?;
                    if self.skip_adventure || m.mission.is_none() || Self::craft_started(m) {
                        self.phase = P::BackOrders;
                        continue;
                    }
                    match v {
                        V::Material(menu) if menu.name == m.name && menu.mission == m.mission => {
                            return Ok(Some(A::OpenAdventure));
                        }
                        V::Adventure(mission) => {
                            if m.mission.as_ref() != Some(mission) {
                                return Err("游历不是所选材料副本");
                            }
                            let i = usize::from(self.current_material.unwrap());
                            self.materials[i].adventure = Some(Box::new(
                                TempleAdventure::material(self.config.clone(), mission.clone())?,
                            ));
                            self.phase = P::Adventure;
                            return Ok(None);
                        }
                        _ => return Ok(None),
                    }
                }
                P::BackMaterial => match v {
                    V::Material(menu) if self.material().is_some_and(|m| m.name == menu.name) => {
                        self.phase = if self
                            .material()
                            .is_some_and(|m| m.craft_unavailable && m.adventure.is_none())
                        {
                            P::GoAdventure
                        } else {
                            P::BackOrders
                        };
                    }
                    V::Adventure(_) | V::CraftUnavailable | V::Craft(_) => {
                        return Ok(Some(A::Back));
                    }
                    _ => return Ok(None),
                },
                P::BackOrders => match v {
                    V::Material(menu) if self.material().is_some_and(|m| m.name == menu.name) => {
                        return Ok(Some(A::Back));
                    }
                    V::Material(_) => return Err("补料返回了其他材料菜单"),
                    V::Page(_) => {
                        if let Some(i) = self.current_material.take() {
                            self.materials[usize::from(i)].done = true;
                        }
                        self.current_row = None;
                        self.phase = P::Materials;
                        return Ok(None);
                    }
                    _ => return Ok(None),
                },
                P::Return => match v {
                    V::Entry => {
                        self.returned_entry = true;
                        self.phase = P::Done;
                        return Ok(None);
                    }
                    V::Page(_) | V::Shops => return Ok(Some(A::Back)),
                    _ => return Ok(None),
                },
                P::Craft | P::Adventure => return Ok(None),
                P::Transaction => return Err("事务缺少活动索引"),
                P::Done => return Ok(None),
            }
        }
        Err("饮茶仙单帧推进超过上限")
    }
}
