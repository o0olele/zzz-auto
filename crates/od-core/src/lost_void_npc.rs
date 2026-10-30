//! 层内 NPC 完成记录与交互后移动；不提供挑战或日周完成证明。
use crate::{
    lost_void::{ArtifactCandidate, REGIONS},
    lost_void_event::LostVoidEventReceipt,
    lost_void_scene::LostVoidScreen as S,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NpcName {
    Marin,
    Ophelia,
    Ray,
    Researcher,
    Ayuan,
    Guaizai,
}
impl NpcName {
    pub fn name(self) -> &'static str {
        match self {
            Self::Marin => "玛琳",
            Self::Ophelia => "奥菲莉亚",
            Self::Ray => "蕾",
            Self::Researcher => "神出鬼没的研究员",
            Self::Ayuan => "阿援",
            Self::Guaizai => "乖仔",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum NpcTarget {
    Npc(NpcName),
    Agent(String),
    Entry(String),
    Anonymous,
}
impl NpcTarget {
    pub fn validate(&self) -> Result<(), &'static str> {
        if match self {
            Self::Agent(id) => !crate::random_play::agents().iter().any(|a| &a.id == id),
            Self::Entry(region) => !REGIONS.contains(&region.as_str()),
            _ => false,
        } {
            return Err("迷失之地交互对象不在固定目录");
        }
        Ok(())
    }
}
pub fn parse_npc_target(text: &str) -> Option<NpcTarget> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let text = text
        .strip_prefix('>')
        .and_then(|t| t.strip_suffix('<'))
        .or_else(|| text.strip_prefix('<').and_then(|t| t.strip_suffix('>')))
        .unwrap_or(&text);
    for n in [
        NpcName::Marin,
        NpcName::Ophelia,
        NpcName::Ray,
        NpcName::Researcher,
        NpcName::Ayuan,
        NpcName::Guaizai,
    ] {
        if text == n.name() {
            return Some(NpcTarget::Npc(n));
        }
    }
    for a in crate::random_play::agents() {
        if text == a.name {
            return Some(NpcTarget::Agent(a.id));
        }
    }
    let region = text.replace('·', "-");
    if REGIONS.contains(&region.as_str()) {
        return Some(NpcTarget::Entry(region));
    }
    ["终结之役·牲鬼", "终结之役·杰佩托"]
        .contains(&text)
        .then(|| NpcTarget::Entry("战斗-终结之役".into()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Interaction {
    id: Uuid,
    target: NpcTarget,
    after_battle: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcMoveRequest {
    pub run_id: Uuid,
    pub level_id: Uuid,
    pub interaction_id: Uuid,
    pub region: String,
    pub target: NpcTarget,
    pub after_battle: bool,
    pub gear_handled: bool,
    pub gear_repeats: u8,
    pub skip_exclamations: bool,
    pub ophelia_moved_before: bool,
    pub timeout_ms: u32,
}
impl NpcMoveRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.target.validate()?;
        if self.run_id.is_nil()
            || self.level_id.is_nil()
            || self.interaction_id.is_nil()
            || !REGIONS.contains(&self.region.as_str())
            || !(1000..=120_000).contains(&self.timeout_ms)
            || self.gear_repeats > 128
            || (self.gear_repeats >= 3 && !self.skip_exclamations)
        {
            return Err("迷失之地 NPC 移动身份、区域或参数无效");
        }
        Ok(())
    }
    pub fn needs_icons(&self) -> bool {
        !self.after_battle
            && self.region == "入口"
            && !self.ophelia_moved_before
            && self.target == NpcTarget::Npc(NpcName::Ophelia)
    }
    fn plan(&self, remaining_icons: Option<bool>) -> Option<Vec<NpcMoveAction>> {
        let hold = |key, ms| NpcMoveAction { key, ms };
        if self.after_battle || matches!(self.target, NpcTarget::Entry(_)) {
            return Some(vec![]);
        }
        Some(if self.region == "入口" {
            match self.target {
                NpcTarget::Npc(NpcName::Ophelia) if !self.ophelia_moved_before => {
                    if remaining_icons? {
                        vec![hold(87, 300), hold(65, 100)]
                    } else {
                        vec![hold(83, 2000)]
                    }
                }
                NpcTarget::Npc(NpcName::Researcher) => vec![hold(68, 500)],
                _ if self.gear_handled => {
                    if self.skip_exclamations {
                        vec![hold(
                            87,
                            (400 + u32::from(self.gear_repeats) * 200).min(1500),
                        )]
                    } else {
                        vec![hold(83, 800)]
                    }
                }
                NpcTarget::Npc(_) => vec![hold(83, 2000)],
                _ => vec![],
            }
        } else if self.region == "挚交会谈" {
            match self.target {
                NpcTarget::Agent(_) | NpcTarget::Npc(NpcName::Marin | NpcName::Ayuan) => {
                    vec![hold(83, 1000), hold(68, 1500)]
                }
                NpcTarget::Npc(NpcName::Ophelia) => vec![hold(83, 1000), hold(65, 1000)],
                _ => vec![hold(83, 1000)],
            }
        } else {
            vec![hold(83, 1000)]
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcMemory {
    run_id: Uuid,
    level_id: Uuid,
    region: String,
    visited: BTreeSet<NpcTarget>,
    used_ids: BTreeSet<Uuid>,
    completed_first: Vec<ArtifactCandidate>,
    gear_repeats: u8,
    ophelia_moved: bool,
    #[serde(default)]
    timeout_skip_exclamations: bool,
    pending: Option<Interaction>,
    moving: Option<NpcMoveRequest>,
}
impl NpcMemory {
    pub fn new(run_id: Uuid, level_id: Uuid, region: String) -> Result<Self, &'static str> {
        let s = Self {
            run_id,
            level_id,
            region,
            visited: BTreeSet::new(),
            used_ids: BTreeSet::new(),
            completed_first: vec![],
            gear_repeats: 0,
            ophelia_moved: false,
            timeout_skip_exclamations: false,
            pending: None,
            moving: None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.run_id.is_nil()
            || self.level_id.is_nil()
            || !REGIONS.contains(&self.region.as_str())
            || self.visited.len() > 128
            || self.used_ids.len() > 128
            || self.gear_repeats > 128
            || self.used_ids.contains(&Uuid::nil())
            || (self.pending.is_some() && self.moving.is_some())
            || self.completed_first.len() > 2
            || (self.region != "入口" && !self.completed_first.is_empty())
            || (self.timeout_skip_exclamations
                && (self.region != "入口" || self.completed_first.is_empty()))
            || self.completed_first.iter().enumerate().any(|(i, a)| {
                a.slot != 0
                    || !crate::lost_void_gear::valid_gear(a)
                    || self.completed_first[..i]
                        .iter()
                        .any(|b| a.category == b.category && a.name == b.name && a.level == b.level)
            })
        {
            return Err("迷失之地楼层交互记录无效");
        }
        for t in &self.visited {
            t.validate()?;
        }
        if let Some(p) = &self.pending {
            p.target.validate()?;
            if p.id.is_nil() || self.used_ids.contains(&p.id) {
                return Err("迷失之地交互身份已经使用");
            }
        }
        if let Some(r) = &self.moving {
            r.validate()?;
            if r.run_id != self.run_id
                || r.level_id != self.level_id
                || r.region != self.region
                || !self.used_ids.contains(&r.interaction_id)
            {
                return Err("迷失之地返回移动与当前楼层不符");
            }
        }
        Ok(())
    }
    pub fn can_interact(&self, target: &NpcTarget) -> bool {
        self.validate().is_ok()
            && target.validate().is_ok()
            && *target != NpcTarget::Anonymous
            && self.pending.is_none()
            && self.moving.is_none()
            && !self.visited.contains(target)
            && (!self.skip_exclamations() || matches!(target, NpcTarget::Entry(_)))
    }
    pub fn skip_exclamations(&self) -> bool {
        self.region == "入口"
            && (self.timeout_skip_exclamations
                || self.gear_repeats >= 3
                || (self.completed_first.len() == 2
                    && self.visited.contains(&NpcTarget::Npc(NpcName::Researcher))))
    }
    pub fn abandon_entry_exclamations_after_timeout(&mut self) -> Result<(), &'static str> {
        if !self.ready_for_navigation()
            || self.region != "入口"
            || self.completed_first.is_empty()
            || self.skip_exclamations()
        {
            return Err("入口尚不能在超时后放弃剩余感叹号");
        }
        self.timeout_skip_exclamations = true;
        self.validate()
    }
    pub fn run_id(&self) -> Uuid {
        self.run_id
    }
    pub fn level_id(&self) -> Uuid {
        self.level_id
    }
    pub fn region(&self) -> &str {
        &self.region
    }
    pub fn ready_for_navigation(&self) -> bool {
        self.validate().is_ok() && self.pending.is_none() && self.moving.is_none()
    }
    pub fn ophelia_completed(&self) -> bool {
        self.visited.contains(&NpcTarget::Npc(NpcName::Ophelia))
    }
    pub fn ignored_regions(&self) -> Vec<String> {
        let mut ignored: Vec<_> = self
            .visited
            .iter()
            .filter_map(|t| match t {
                NpcTarget::Entry(region) => Some(region.clone()),
                _ => None,
            })
            .collect();
        if self.visited.contains(&NpcTarget::Npc(NpcName::Marin))
            && !ignored.iter().any(|r| r == "偶遇事件")
        {
            ignored.push("偶遇事件".into());
        }
        ignored
    }
    pub fn pending_event(&self) -> Option<(Uuid, &NpcTarget)> {
        self.pending.as_ref().map(|p| (p.id, &p.target))
    }
    pub fn completed_first(&self) -> &[ArtifactCandidate] {
        &self.completed_first
    }
    /// 自动弹出的战后 / 武备交互可保留匿名目标，不能据此授权按 F。
    pub fn begin_event(
        &mut self,
        id: Uuid,
        target: NpcTarget,
        after_battle: bool,
    ) -> Result<(), &'static str> {
        self.validate()?;
        target.validate()?;
        if id.is_nil()
            || self.used_ids.contains(&id)
            || self.pending.is_some()
            || self.moving.is_some()
            || self.used_ids.len() >= 128
        {
            return Err("迷失之地交互尚未交回或身份已用过");
        }
        self.pending = Some(Interaction {
            id,
            target,
            after_battle,
        });
        Ok(())
    }
    pub fn finish_event(
        &mut self,
        receipt: &LostVoidEventReceipt,
        timeout_ms: u32,
    ) -> Result<NpcMoveRequest, &'static str> {
        self.validate()?;
        let p = self.pending.as_ref().ok_or("迷失之地没有当前交互目标")?;
        if receipt.run_id != self.run_id
            || receipt.interaction_id != p.id
            || receipt.returned_to != S::World
            || (receipt.actions == 0 && receipt.children == 0)
            || receipt.gear_repeats > 128
            || !receipt.completed_first.starts_with(&self.completed_first)
        {
            return Err("迷失之地事件尚无匹配的返回大世界回执");
        }
        let mut next = self.clone();
        let repeats = next
            .gear_repeats
            .saturating_add(receipt.gear_repeats)
            .min(128);
        // 返回当前世界的非空事件不是换层证明；途中被 NPC 抢到交互时保留进门机会。
        let actual_target = if matches!(p.target, NpcTarget::Entry(_)) {
            NpcTarget::Anonymous
        } else {
            p.target.clone()
        };
        let request = NpcMoveRequest {
            run_id: self.run_id,
            level_id: self.level_id,
            interaction_id: p.id,
            region: self.region.clone(),
            target: actual_target.clone(),
            after_battle: p.after_battle,
            gear_handled: receipt.completed_first != self.completed_first
                || receipt.gear_repeats > 0,
            gear_repeats: repeats,
            skip_exclamations: self.region == "入口"
                && (repeats >= 3
                    || (receipt.completed_first.len() == 2
                        && (self.visited.contains(&NpcTarget::Npc(NpcName::Researcher))
                            || p.target == NpcTarget::Npc(NpcName::Researcher)))),
            ophelia_moved_before: self.ophelia_moved,
            timeout_ms,
        };
        request.validate()?;
        next.visited.insert(actual_target);
        next.used_ids.insert(p.id);
        next.completed_first = receipt.completed_first.clone();
        next.gear_repeats = repeats;
        next.pending = None;
        next.moving = Some(request.clone());
        next.validate()?;
        *self = next;
        Ok(request)
    }
    pub fn accept_move(&mut self, receipt: &NpcMoveReceipt) -> Result<(), &'static str> {
        self.validate()?;
        if self.moving.as_ref() != Some(&receipt.request) {
            return Err("迷失之地移动回执不属于当前交互");
        }
        if receipt.request.needs_icons() {
            self.ophelia_moved = true;
        }
        self.moving = None;
        Ok(())
    }
    /// 移动中自动弹出的奖励不结束原走位，也不能冒充另一个 NPC 的访问。
    pub fn absorb_inline_event(
        &mut self,
        receipt: &LostVoidEventReceipt,
    ) -> Result<(), &'static str> {
        self.validate()?;
        let moving = self
            .moving
            .as_ref()
            .ok_or("当前没有被事件打断的 NPC 移动")?;
        if receipt.run_id != self.run_id
            || receipt.interaction_id.is_nil()
            || receipt.interaction_id == moving.interaction_id
            || self.used_ids.contains(&receipt.interaction_id)
            || receipt.returned_to != S::World
            || receipt.actions == 0 && receipt.children == 0
            || !receipt.completed_first.starts_with(&self.completed_first)
            || receipt.gear_repeats > 128
        {
            return Err("移动中的附加事件身份或后态不符");
        }
        let mut next = self.clone();
        next.used_ids.insert(receipt.interaction_id);
        next.completed_first = receipt.completed_first.clone();
        next.gear_repeats = next
            .gear_repeats
            .saturating_add(receipt.gear_repeats)
            .min(128);
        next.validate()?;
        *self = next;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcMoveAction {
    pub key: i32,
    pub ms: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcMoveView {
    pub screen: S,
    pub other_exclamation: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcMoveReceipt {
    pub request: NpcMoveRequest,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcMoveFlow {
    request: NpcMoveRequest,
    icons: Option<bool>,
    plan: Option<Vec<NpcMoveAction>>,
    index: usize,
    held_ms: u32,
    pending: Option<NpcMoveAction>,
    stable: Option<NpcMoveView>,
    reads: u8,
    receipt: Option<NpcMoveReceipt>,
}
impl NpcMoveFlow {
    pub fn new(request: NpcMoveRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            icons: None,
            plan: None,
            index: 0,
            held_ms: 0,
            pending: None,
            stable: None,
            reads: 0,
            receipt: None,
        })
    }
    pub fn request(&self) -> &NpcMoveRequest {
        &self.request
    }
    pub fn needs_icons(&self) -> bool {
        self.request.needs_icons() && self.index == 0 && self.held_ms == 0
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn receipt(&self) -> Option<&NpcMoveReceipt> {
        self.receipt.as_ref()
    }
    pub fn invalidate_observation(&mut self) {
        self.stable = None;
        self.reads = 0;
        self.receipt = None;
    }
    fn action(&self) -> Option<NpcMoveAction> {
        self.plan.as_ref()?.get(self.index).map(|a| NpcMoveAction {
            key: a.key,
            ms: a.ms - self.held_ms,
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.reads > 2
            || (self.reads > 0 && self.stable.is_none())
            || self.stable.as_ref().is_some_and(|v| v.screen != S::World)
            || self
                .plan
                .as_ref()
                .is_some_and(|p| Some(p) != self.request.plan(self.icons).as_ref())
            || self.plan.as_ref().is_some_and(|p| {
                self.index > p.len() || (self.index == p.len() && self.held_ms != 0)
            })
            || self.plan.is_none() && (self.index != 0 || self.held_ms != 0)
            || self
                .plan
                .as_ref()
                .and_then(|p| p.get(self.index))
                .is_some_and(|a| self.held_ms >= a.ms)
            || self.pending.is_some_and(|a| Some(a) != self.action())
            || self.receipt.as_ref().is_some_and(|r| {
                r.request != self.request
                    || self.pending.is_some()
                    || self.action().is_some()
                    || self.plan.is_none()
                    || self.reads != 2
            })
        {
            return Err("迷失之地 NPC 移动阶段或意图无效");
        }
        Ok(())
    }
    pub fn next(&mut self, view: &NpcMoveView) -> Result<Option<NpcMoveAction>, &'static str> {
        self.validate()?;
        if view.screen != S::World || (self.needs_icons() && view.other_exclamation.is_none()) {
            self.stable = None;
            self.reads = 0;
            self.receipt = None;
            return Ok(None);
        }
        if self.pending.is_some() {
            return Ok(None);
        }
        // 第一段尚未移动且已明确未投递时，可以按新图标分布重新选择方向。
        if self.needs_icons() && self.plan.is_some() && self.icons != view.other_exclamation {
            self.plan = None;
            self.icons = None;
            self.invalidate_observation();
        }
        if self.stable.as_ref() == Some(view) {
            self.reads = (self.reads + 1).min(2);
        } else {
            self.stable = Some(view.clone());
            self.reads = 1;
        }
        if self.reads < 2 {
            return Ok(None);
        }
        if self.plan.is_none() {
            self.icons = view.other_exclamation;
            self.plan = self.request.plan(self.icons);
            self.reads = 0;
            self.stable = None;
            return Ok(None);
        }
        let action = self.action();
        if action.is_none() {
            self.receipt = Some(NpcMoveReceipt {
                request: self.request.clone(),
            });
        }
        Ok(action)
    }
    pub fn prepare(&mut self, a: NpcMoveAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some()
            || Some(a) != self.action()
            || self.reads != 2
            || self.stable.as_ref().is_none_or(|v| v.screen != S::World)
        {
            return Err("NPC 移动缺少本次新帧依据");
        }
        self.pending = Some(a);
        self.receipt = None;
        Ok(())
    }
    pub fn allows(&self, a: NpcMoveAction, view: &NpcMoveView) -> bool {
        self.validate().is_ok()
            && self.pending == Some(a)
            && self.action() == Some(a)
            && view.screen == S::World
            && (!self.needs_icons() || view.other_exclamation == self.icons)
    }
    pub fn held(&mut self, a: NpcMoveAction, ms: u32) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending != Some(a) || ms > a.ms {
            return Err("NPC 移动持有时长与当前意图不符");
        }
        self.held_ms += ms;
        self.pending = None;
        if ms == a.ms {
            self.index += 1;
            self.held_ms = 0;
        }
        self.stable = None;
        self.reads = 0;
        self.receipt = None;
        Ok(())
    }
    pub fn not_posted(&mut self, a: NpcMoveAction) -> Result<(), &'static str> {
        self.held(a, 0)
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.receipt = None;
        self.stable = None;
        self.reads = 0;
        Ok(())
    }
}
