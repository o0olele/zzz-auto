//! 迷失之地同帧目标选择、镜头反馈及交互提交。提交回执不是换层或通关证明。
use crate::{
    lost_void::{LostVoidChallenge, REGIONS},
    lost_void_motion::{MotionAction, MotionDirection, MotionRecovery, MotionResult},
    lost_void_npc::{NpcMemory, NpcTarget},
    lost_void_scene::LostVoidScreen as S,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;
pub const NAV_LABELS: [&str; 16] = [
    "0000-感叹号",
    "0001-距离",
    "0002-战斗-鸣徽",
    "0003-战斗-武备",
    "0004-战斗-硬币",
    "0005-挑战-无伤",
    "0006-挑战-限时",
    "0007-偶遇事件",
    "0008-代价之间",
    "0009-休憩调息",
    "0010-邦布商店",
    "0011-挚交会谈",
    "0012-战斗-道中危机",
    "0013-战斗-终结之役",
    "0014-挑战-收割",
    "0015-迷雾",
];
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavDetection {
    pub label: String,
    pub rect: [i32; 4],
    pub score: f64,
}
impl NavDetection {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !NAV_LABELS.contains(&self.label.as_str())
            || !crate::intel_board::valid_rect(self.rect)
            || !self.score.is_finite()
            || self.score <= 0.6
            || self.score > 1.0
        {
            return Err("迷失之地导航检测结果无效");
        }
        Ok(())
    }
    pub fn name(&self) -> &str {
        &self.label[5..]
    }
    fn x(&self) -> f64 {
        f64::from(self.rect[0]) + f64::from(self.rect[2]) / 2.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavKind {
    Exclamation,
    Distance,
    Entry,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavTarget {
    pub kind: NavKind,
    pub names: Vec<String>,
    pub rect: [i32; 4],
}
impl NavTarget {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !crate::intel_board::valid_rect(self.rect)
            || self.names.is_empty()
            || self.names.len() > 256
            || match self.kind {
                NavKind::Exclamation => self.names != ["感叹号"],
                NavKind::Distance => self.names != ["距离"],
                NavKind::Entry => self
                    .names
                    .iter()
                    .any(|n| !NAV_LABELS[2..].iter().any(|l| &l[5..] == n)),
            }
        {
            return Err("迷失之地导航目标无效");
        }
        Ok(())
    }
    pub fn x(&self) -> f64 {
        f64::from(self.rect[0]) + f64::from(self.rect[2]) / 2.0
    }
    pub fn same_kind(&self, other: &Self) -> bool {
        self.kind == other.kind && self.names == other.names
    }
    pub fn region_hint(&self) -> Option<&str> {
        (self.kind == NavKind::Entry)
            .then(|| self.names.first().map(String::as_str))
            .flatten()
            .filter(|s| REGIONS.contains(s))
    }
}
fn group_rect(items: &[&NavDetection]) -> [i32; 4] {
    let left = items.iter().map(|d| d.rect[0]).min().unwrap();
    let top = items.iter().map(|d| d.rect[1]).min().unwrap();
    let right = items.iter().map(|d| d.rect[0] + d.rect[2]).max().unwrap();
    let bottom = items.iter().map(|d| d.rect[1] + d.rect[3]).max().unwrap();
    [left, top, right - left, bottom - top]
}
pub fn entry_groups(detections: &[NavDetection]) -> Result<Vec<NavTarget>, &'static str> {
    validate_detections(detections)?;
    let mut items: Vec<_> = detections
        .iter()
        .filter(|d| d.label != NAV_LABELS[0] && d.label != NAV_LABELS[1])
        .collect();
    items.sort_by_key(|d| (d.rect[0], d.rect[1], d.label.as_str()));
    let mut components: Vec<Vec<&NavDetection>> = vec![];
    for item in items {
        let mut merged = vec![item];
        let mut i = 0;
        while i < components.len() {
            let near = components[i].iter().any(|a| {
                merged.iter().any(|b| {
                    let ax = a.x();
                    let bx = b.x();
                    let ay = f64::from(a.rect[1]) + f64::from(a.rect[3]) / 2.0;
                    let by = f64::from(b.rect[1]) + f64::from(b.rect[3]) / 2.0;
                    (ax - bx).hypot(ay - by) < f64::from(a.rect[2].max(b.rect[2]) * 2)
                })
            });
            if near {
                merged.extend(components.remove(i));
                i = 0;
            } else {
                i += 1;
            }
        }
        components.push(merged);
    }
    Ok(components
        .into_iter()
        .map(|mut group| {
            group.sort_by_key(|d| (d.rect[0], d.rect[1], d.label.as_str()));
            NavTarget {
                kind: NavKind::Entry,
                names: group.iter().map(|d| d.name().into()).collect(),
                rect: group_rect(&group),
            }
        })
        .collect())
}
pub fn validate_detections(detections: &[NavDetection]) -> Result<(), &'static str> {
    if detections.len() > 256 {
        return Err("迷失之地导航检测数量超过上限");
    }
    let mut seen = BTreeSet::new();
    for d in detections {
        d.validate()?;
        if !seen.insert((&d.label, d.rect)) {
            return Err("迷失之地导航检测重复");
        }
    }
    Ok(())
}
pub fn select_target(
    detections: &[NavDetection],
    challenge: &LostVoidChallenge,
    memory: &NpcMemory,
    ignored: &[String],
    last: Option<&NavTarget>,
) -> Result<Option<NavTarget>, &'static str> {
    validate_detections(detections)?;
    challenge.validate()?;
    memory.validate()?;
    if ignored.len() > 16
        || ignored.iter().any(|s| {
            !REGIONS.contains(&s.as_str()) && !matches!(s.as_str(), "感叹号" | "距离" | "迷雾")
        })
    {
        return Err("迷失之地导航忽略类别无效");
    }
    let blocked = |name: &str| {
        ignored.iter().any(|s| s == name)
            || (name == "感叹号" && memory.skip_exclamations())
            || (name == "战斗-道中危机" && memory.ophelia_completed())
    };
    let mut entries = entry_groups(detections)?;
    if memory.ophelia_completed() {
        entries.retain(|g| !g.names.iter().any(|s| s == "战斗-道中危机"));
    }
    if let Some(last) = last.filter(|l| l.kind == NavKind::Entry)
        && let Some(found) = entries
            .iter()
            .filter(|g| g.same_kind(last) && !g.names.iter().any(|s| blocked(s)))
            .min_by(|a, b| {
                (a.x() - last.x())
                    .abs()
                    .total_cmp(&(b.x() - last.x()).abs())
            })
    {
        return Ok(Some(found.clone()));
    }
    for (label, kind) in [
        (NAV_LABELS[0], NavKind::Exclamation),
        (NAV_LABELS[1], NavKind::Distance),
    ] {
        if blocked(&label[5..]) {
            continue;
        }
        let candidate = if memory.region() == "入口" {
            detections
                .iter()
                .filter(|d| d.label == label)
                .max_by(|a, b| a.x().total_cmp(&b.x()))
        } else {
            detections
                .iter()
                .filter(|d| d.label == label)
                .min_by(|a, b| a.x().total_cmp(&b.x()))
        };
        if let Some(d) = candidate {
            return Ok(Some(NavTarget {
                kind,
                names: vec![d.name().into()],
                rect: d.rect,
            }));
        }
    }
    for mixed in [false, true] {
        let pool: Vec<_> = entries
            .iter()
            .filter(|g| (g.names.len() > 1) == mixed)
            .collect();
        for priority in &challenge.region_type_priority {
            if blocked(priority) {
                continue;
            }
            if let Some(g) = pool
                .iter()
                .filter(|g| g.names.contains(priority))
                .max_by_key(|g| g.rect[0])
            {
                return Ok(Some((*g).clone()));
            }
        }
        if let Some(g) = pool
            .iter()
            .filter(|g| !g.names.iter().any(|n| blocked(n)))
            .max_by_key(|g| g.rect[0])
        {
            return Ok(Some((*g).clone()));
        }
    }
    Ok(None)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavSteering {
    ratio: f64,
    last_x: Option<f64>,
    last_dx: i32,
    calibrations: u16,
    settle: u8,
}
impl Default for NavSteering {
    fn default() -> Self {
        Self {
            ratio: 0.2,
            last_x: None,
            last_dx: 0,
            calibrations: 1,
            settle: 0,
        }
    }
}
impl NavSteering {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.ratio.is_finite()
            || !(0.02..=1.0).contains(&self.ratio)
            || self
                .last_x
                .is_some_and(|x| !x.is_finite() || !(0.0..=1920.0).contains(&x))
            || self.last_dx.unsigned_abs() > 200
            || self.calibrations == 0
            || self.settle > 1
        {
            return Err("迷失之地镜头校准记录无效");
        }
        Ok(())
    }
    pub fn observe(&mut self, x: f64, moving: bool) -> Result<i32, &'static str> {
        self.validate()?;
        if !x.is_finite() || !(0.0..=1920.0).contains(&x) {
            return Err("导航目标水平位置无效");
        }
        let diff = x - 960.0;
        if diff.abs() <= 50.0 {
            return Ok(0);
        }
        if !moving && let Some(last) = self.last_x.take().filter(|_| self.last_dx != 0) {
            let old = last - 960.0;
            let moved = old - diff;
            if old * diff < 0.0 {
                self.ratio = (self.ratio * 0.5).max(0.02);
                self.settle = 1;
            } else if moved.abs() > 5.0 {
                let current = (f64::from(self.last_dx) / moved).abs().clamp(0.02, 1.0);
                self.ratio = if self.calibrations <= 2 {
                    current
                } else {
                    let n = f64::from(self.calibrations);
                    (self.ratio * (n - 1.0) + current) / n
                };
                self.calibrations = self.calibrations.saturating_add(1);
            }
        }
        if !moving && self.settle > 0 && diff.abs() < 120.0 {
            self.settle -= 1;
            return Ok(0);
        }
        let mut dx = if self.calibrations == 1 {
            if !moving {
                self.calibrations = 2;
            }
            5 * diff.signum() as i32
        } else {
            (diff * self.ratio) as i32
        };
        if dx != 0 && dx.abs() < 5 {
            dx = if !moving && diff.abs() < 120.0 {
                0
            } else {
                5 * dx.signum()
            };
        }
        Ok(dx.clamp(
            if moving { -15 } else { -200 },
            if moving { 15 } else { 200 },
        ))
    }
    pub fn posted(&mut self, x: f64, dx: i32) {
        self.last_x = Some(x);
        self.last_dx = dx;
    }
    pub fn clear_feedback(&mut self) {
        self.last_x = None;
        self.last_dx = 0;
        self.settle = 0;
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavFocus {
    pub target: NpcTarget,
    pub rect: [i32; 4],
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavView {
    pub screen: S,
    #[serde(default)]
    pub loading: Option<crate::lost_void_loading::LoadingObservation>,
    pub detections: Vec<NavDetection>,
    pub interact: bool,
    pub attack: bool,
    pub battle: bool,
    pub focus: Option<NavFocus>,
}
impl NavView {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(loading) = &self.loading {
            loading.validate()?;
            if self.screen != S::Unknown
                || !self.detections.is_empty()
                || self.interact
                || self.attack
                || self.battle
                || self.focus.is_some()
            {
                return Err("加载帧不能混用世界目标或输入依据");
            }
        }
        validate_detections(&self.detections)?;
        if let Some(f) = &self.focus {
            f.target.validate()?;
            if f.target == NpcTarget::Anonymous || !crate::intel_board::valid_rect(f.rect) {
                return Err("导航交互名牌无效");
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavRequest {
    pub run_id: Uuid,
    pub level_id: Uuid,
    pub interaction_id: Uuid,
    pub challenge: LostVoidChallenge,
    pub memory: NpcMemory,
    pub ignored: Vec<String>,
    pub timeout_ms: u32,
    pub turn_sensitivity_milli: u32,
}
impl NavRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.challenge.validate()?;
        self.memory.validate()?;
        if self.run_id.is_nil()
            || self.level_id.is_nil()
            || self.interaction_id.is_nil()
            || self.memory.run_id() != self.run_id
            || self.memory.level_id() != self.level_id
            || !(1000..=600_000).contains(&self.timeout_ms)
            || !(1..=100_000).contains(&self.turn_sensitivity_milli)
        {
            return Err("迷失之地导航身份或预算无效");
        }
        select_target(&[], &self.challenge, &self.memory, &self.ignored, None)?;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavWalkKind {
    Forward,
    Reposition,
    Recover,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NavAction {
    Motion {
        action: MotionAction,
    },
    Turn {
        dx: i32,
    },
    Walk {
        key: i32,
        ms: u32,
        mode: NavWalkKind,
    },
    Interact {
        target: NpcTarget,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NavOutcome {
    InteractionSubmitted { target: NpcTarget },
    ObservedEvent,
    EncounteredBattle,
    ObservedResult,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavReceipt {
    pub run_id: Uuid,
    pub level_id: Uuid,
    pub interaction_id: Uuid,
    pub outcome: NavOutcome,
    pub screen: S,
    pub battle: bool,
    pub next_region_hint: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    action: NavAction,
    before: NavView,
    target: Option<NavTarget>,
    steering: NavSteering,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavFlow {
    request: NavRequest,
    track: Option<NavTarget>,
    steering: NavSteering,
    candidate_steering: NavSteering,
    current: Option<NavView>,
    focus: Option<NpcTarget>,
    focus_reads: u8,
    pending: Option<Pending>,
    submitted: Option<NpcTarget>,
    #[serde(default)]
    departure: Option<crate::lost_void_loading::LoadingObservation>,
    ready: Option<(S, u8)>,
    receipt: Option<NavReceipt>,
    searches: u16,
    steps: u16,
    repositions: u16,
    side_next: bool,
    following: bool,
    #[serde(default)]
    motion: MotionRecovery,
}
impl NavFlow {
    pub fn new(request: NavRequest) -> Result<Self, &'static str> {
        request.validate()?;
        if !request.memory.ready_for_navigation() {
            return Err("上一交互或移动尚未交回，不能开始导航");
        }
        let mut probe = request.memory.clone();
        probe.begin_event(request.interaction_id, NpcTarget::Anonymous, false)?;
        Ok(Self {
            request,
            track: None,
            steering: Default::default(),
            candidate_steering: Default::default(),
            current: None,
            focus: None,
            focus_reads: 0,
            pending: None,
            submitted: None,
            departure: None,
            ready: None,
            receipt: None,
            searches: 0,
            steps: 0,
            repositions: 0,
            side_next: false,
            following: false,
            motion: MotionRecovery::default(),
        })
    }
    pub fn request(&self) -> &NavRequest {
        &self.request
    }
    pub fn memory(&self) -> &NpcMemory {
        &self.request.memory
    }
    pub fn departure(&self) -> Option<&crate::lost_void_loading::LoadingObservation> {
        self.departure.as_ref()
    }
    pub fn can_abandon_after_timeout(&self) -> bool {
        self.validate().is_ok()
            && !self.completed()
            && self.pending.is_none()
            && self.submitted.is_none()
            && self.departure.is_none()
            && self.request.memory.ready_for_navigation()
    }
    fn door_intent(&self) -> Option<NpcTarget> {
        self.submitted
            .as_ref()
            .filter(|t| matches!(t, NpcTarget::Entry(_)))
            .cloned()
            .or_else(|| {
                self.pending.as_ref().and_then(|p| match &p.action {
                    NavAction::Interact {
                        target: NpcTarget::Entry(region),
                    } => Some(NpcTarget::Entry(region.clone())),
                    _ => None,
                })
            })
    }
    pub fn motion(&self) -> &MotionRecovery {
        &self.motion
    }
    /// 父任务从已冻结的战斗设置提供普通攻击，不能在输入开始后替换。
    pub fn with_obstacle_attack(
        mut self,
        input: crate::InputProbeAction,
    ) -> Result<Self, &'static str> {
        if self.steps != 0 || self.pending.is_some() {
            return Err("导航开始后不能更换清障绑定");
        }
        self.motion = self.motion.with_attack(input)?;
        self.validate()?;
        Ok(self)
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn receipt(&self) -> Option<&NavReceipt> {
        self.receipt.as_ref()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if let Some(departure) = &self.departure {
            departure.validate()?;
            if self.door_intent().is_none() {
                return Err("导航加载历史缺少本次进门意图");
            }
        }
        self.steering.validate()?;
        self.candidate_steering.validate()?;
        self.motion.validate()?;
        if self.steps > 512
            || self.searches > 100
            || self.repositions > 128
            || self.focus_reads > 2
            || self.ready.is_some_and(|(_, n)| n > 2)
        {
            return Err("迷失之地导航步骤无效");
        }
        if let Some(v) = &self.current {
            v.validate()?;
        }
        if let Some(t) = &self.track {
            t.validate()?;
        }
        if let Some(p) = &self.pending {
            if self.steps == 0 {
                return Err("导航意图缺少对应动作次数");
            }
            p.before.validate()?;
            p.steering.validate()?;
            if let Some(t) = &p.target {
                t.validate()?;
            }
            if let NavAction::Interact { target } = &p.action {
                target.validate()?;
            }
            if matches!(&p.action,NavAction::Turn{dx} if *dx==0||dx.unsigned_abs()>200)
                || matches!(&p.action,NavAction::Walk{key,ms,..} if ![87,65,83,68].contains(key)||*ms==0||*ms>1500)
            {
                return Err("迷失之地导航保存意图无效");
            }
        }
        if let Some(r) = &self.receipt
            && (r.run_id != self.request.run_id
                || r.level_id != self.request.level_id
                || r.interaction_id != self.request.interaction_id
                || self.ready.is_none_or(|(s, n)| s != r.screen || n != 2))
        {
            return Err("迷失之地导航回执无效");
        }
        Ok(())
    }
    pub fn invalidate_observation(&mut self) {
        self.current = None;
        self.focus = None;
        self.focus_reads = 0;
        self.ready = None;
        self.receipt = None;
    }
    fn finish(&mut self, outcome: NavOutcome, screen: S) -> Result<(), &'static str> {
        let reads = if self.ready.is_some_and(|(s, _)| s == screen) {
            (self.ready.unwrap().1 + 1).min(2)
        } else {
            1
        };
        self.ready = Some((screen, reads));
        if reads < 2 {
            return Ok(());
        }
        if matches!(outcome, NavOutcome::ObservedEvent)
            && self.request.memory.ready_for_navigation()
        {
            self.request.memory.begin_event(
                self.request.interaction_id,
                NpcTarget::Anonymous,
                false,
            )?;
        }
        self.receipt = Some(NavReceipt {
            run_id: self.request.run_id,
            level_id: self.request.level_id,
            interaction_id: self.request.interaction_id,
            next_region_hint: match &outcome {
                NavOutcome::InteractionSubmitted {
                    target: NpcTarget::Entry(s),
                } => Some(s.clone()),
                _ => None,
            },
            outcome,
            screen,
            battle: self.current.as_ref().is_some_and(|v| v.battle),
        });
        Ok(())
    }
    fn can_interact(&self, target: &NpcTarget) -> bool {
        self.request.memory.can_interact(target)
            && match target {
                NpcTarget::Entry(s) => {
                    !(self.request.ignored.contains(s)
                        || s == "战斗-道中危机" && self.request.memory.ophelia_completed())
                }
                _ => true,
            }
    }
    pub fn next(&mut self, view: &NavView) -> Result<Option<NavAction>, &'static str> {
        self.validate()?;
        view.validate()?;
        if let Some(loading) = &view.loading {
            if self.door_intent().is_some() {
                self.departure = Some(loading.clone());
            }
            self.invalidate_observation();
            return Ok(None);
        }
        // F 结果未知时，只能以已出现的明确加载及后续实际页面消解，不重发 F。
        if self.departure.is_some()
            && self.submitted.is_none()
            && view.screen != S::Unknown
            && let Some(action) = self.pending.as_ref().map(|p| p.action.clone())
            && matches!(
                action,
                NavAction::Interact {
                    target: NpcTarget::Entry(_)
                }
            )
        {
            self.posted(&action)?;
        }
        self.current = Some(view.clone());
        if let Some(target) = self.submitted.clone() {
            if view.screen != S::Unknown {
                self.finish(NavOutcome::InteractionSubmitted { target }, view.screen)?;
            } else {
                self.ready = None;
            }
            return Ok(None);
        }
        if self.pending.is_some() {
            if self
                .pending
                .as_ref()
                .is_some_and(|p| matches!(p.action, NavAction::Interact { .. }))
            {
                if matches!(
                    view.screen,
                    S::Event
                        | S::CommonChoice
                        | S::Gear
                        | S::Store
                        | S::Lottery
                        | S::RouteChange
                        | S::GearConfirm
                ) {
                    self.finish(NavOutcome::ObservedEvent, view.screen)?;
                    if self.receipt.is_some() {
                        self.pending = None;
                    }
                } else {
                    self.ready = None;
                }
            }
            return Ok(None);
        }
        if view.screen == S::Unknown || view.screen == S::Modal {
            self.invalidate_observation();
            return Ok(None);
        }
        if view.battle && view.screen == S::World {
            self.finish(NavOutcome::EncounteredBattle, view.screen)?;
            return Ok(None);
        }
        if view.screen != S::World {
            match view.screen {
                S::Event
                | S::CommonChoice
                | S::Gear
                | S::Store
                | S::Lottery
                | S::RouteChange
                | S::GearConfirm => self.finish(NavOutcome::ObservedEvent, view.screen)?,
                S::BattleResult | S::BattleFail => {
                    self.finish(NavOutcome::ObservedResult, view.screen)?
                }
                _ => self.invalidate_observation(),
            }
            return Ok(None);
        }
        if !self.request.memory.ready_for_navigation() {
            return Ok(None);
        }
        if self.steps >= 512 {
            return Err("迷失之地导航动作达到上限");
        }
        self.ready = None;
        self.receipt = None;
        self.current = Some(view.clone());
        let focused = view.focus.as_ref().map(|f| f.target.clone());
        if focused == self.focus && focused.is_some() {
            self.focus_reads = (self.focus_reads + 1).min(2);
        } else {
            self.focus = focused;
            self.focus_reads = u8::from(self.focus.is_some());
        }
        if view.interact
            && let Some(f) = &view.focus
        {
            if self.can_interact(&f.target) && self.focus_reads < 2 {
                return Ok(None);
            }
            if self.focus_reads >= 2 && self.can_interact(&f.target) {
                return Ok(Some(NavAction::Interact {
                    target: f.target.clone(),
                }));
            }
        }
        let previous = self.track.clone();
        self.track = select_target(
            &view.detections,
            &self.request.challenge,
            &self.request.memory,
            &self.request.ignored,
            self.track.as_ref(),
        )?;
        if previous
            .as_ref()
            .zip(self.track.as_ref())
            .is_none_or(|(a, b)| !a.same_kind(b) || (a.x() - b.x()).abs() > 400.0)
        {
            self.steering = Default::default();
            self.following = false;
        }
        if !view.interact
            && let Some(action) = self.motion.action(view.attack)?
            && (self.motion.recovering() || self.track.is_some())
        {
            return Ok(Some(NavAction::Motion { action }));
        }
        if view.interact {
            let exclude_x = view
                .focus
                .as_ref()
                .map(|f| f64::from(f.rect[0]) + f64::from(f.rect[2]) / 2.0);
            let prefer_door = self.request.memory.skip_exclamations();
            let other = view
                .detections
                .iter()
                .filter(|d| {
                    if prefer_door {
                        d.label != NAV_LABELS[0] && d.label != NAV_LABELS[1]
                    } else {
                        d.label == NAV_LABELS[0]
                            && exclude_x.is_none_or(|x| (d.x() - x).abs() > 100.0)
                    }
                })
                .max_by_key(|d| d.rect[3]);
            if self.repositions >= 8 {
                return Ok(Some(NavAction::Walk {
                    key: 83,
                    ms: 1200,
                    mode: NavWalkKind::Recover,
                }));
            }
            if self.side_next
                && let Some(d) = other
            {
                let dx = d.x() - 960.0;
                if dx.abs() > 60.0 {
                    return Ok(Some(NavAction::Walk {
                        key: if dx < 0.0 { 65 } else { 68 },
                        ms: (100.0 + dx.abs() * 1000.0 / 800.0).min(300.0) as u32,
                        mode: NavWalkKind::Reposition,
                    }));
                }
            }
            return Ok(Some(NavAction::Walk {
                key: 87,
                ms: if prefer_door { 300 } else { 180 },
                mode: NavWalkKind::Reposition,
            }));
        }
        if let Some(target) = &self.track {
            let mut steer = self.steering.clone();
            let dx = steer.observe(target.x(), self.following)?;
            self.candidate_steering = steer;
            if dx != 0 {
                let cap = (10_000_000 / self.request.turn_sensitivity_milli).min(200) as i32;
                return Ok(Some(NavAction::Turn {
                    dx: dx.clamp(-cap, cap),
                }));
            }
            return Ok(Some(NavAction::Walk {
                key: 87,
                ms: 600,
                mode: NavWalkKind::Forward,
            }));
        }
        if self.searches >= 100 {
            return Err("迷失之地导航搜索达到上限");
        }
        Ok(Some(NavAction::Turn {
            dx: -((10_000_000 / self.request.turn_sensitivity_milli).min(200) as i32),
        }))
    }
    pub fn prepare(&mut self, action: NavAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some() || self.current.as_ref().is_none_or(|v| v.screen != S::World) {
            return Err("导航缺少本次世界画面");
        }
        let view = self.current.clone().unwrap();
        let mut preview = self.clone();
        if preview.next(&view)?.as_ref() != Some(&action) {
            return Err("导航动作与当前目标不符");
        }
        self.pending = Some(Pending {
            action,
            before: view,
            target: self.track.clone(),
            steering: self.candidate_steering.clone(),
        });
        self.steps += 1;
        Ok(())
    }
    pub fn allows(&self, action: &NavAction, view: &NavView) -> bool {
        let Some(p) = &self.pending else {
            return false;
        };
        if self.validate().is_err() || &p.action != action || view.screen != S::World || view.battle
        {
            return false;
        }
        let mut preview = self.clone();
        preview.pending = None;
        preview.steps = preview.steps.saturating_sub(1);
        let Ok(Some(fresh)) = preview.next(view) else {
            return false;
        };
        let target_same = match (&p.target, &preview.track) {
            (Some(a), Some(b)) => a.same_kind(b) && (a.x() - b.x()).abs() <= 100.0,
            (None, None) => true,
            _ => false,
        };
        match (action, fresh) {
            (NavAction::Motion { action: a }, NavAction::Motion { action: b }) => {
                a == &b && (self.motion.recovering() || target_same)
            }
            (NavAction::Interact { target: a }, NavAction::Interact { target: b }) => a == &b,
            (NavAction::Turn { dx: a }, NavAction::Turn { dx: b }) => {
                target_same && a.signum() == b.signum() && a.abs() <= b.abs().max(5) * 2
            }
            (
                NavAction::Walk {
                    key: a,
                    ms: am,
                    mode: ak,
                },
                NavAction::Walk {
                    key: b,
                    ms: bm,
                    mode: bk,
                },
            ) => target_same && *a == b && *ak == bk && am.abs_diff(bm) <= 25,
            _ => false,
        }
    }
    pub fn posted(&mut self, action: &NavAction) -> Result<(), &'static str> {
        self.validate()?;
        let p = self.pending.as_ref().ok_or("导航没有待投递意图")?;
        if &p.action != action {
            return Err("导航投递动作不一致");
        }
        match action {
            NavAction::Motion {
                action: MotionAction::ClearObstacle,
            } => self.motion.clear_submitted()?,
            NavAction::Motion {
                action: MotionAction::Probe { .. },
            } => return Err("背景探测必须提交实际采样结果"),
            NavAction::Interact { target } => {
                self.request.memory.begin_event(
                    self.request.interaction_id,
                    target.clone(),
                    false,
                )?;
                self.submitted = Some(target.clone());
            }
            NavAction::Turn { dx } => {
                self.motion.on_turn(*dx, p.target.is_none());
                if let Some(t) = &p.target {
                    self.steering = p.steering.clone();
                    self.steering.posted(t.x(), *dx);
                } else {
                    self.searches += 1;
                    self.steering = Default::default();
                }
            }
            NavAction::Walk { mode, ms, .. } => {
                if *mode == NavWalkKind::Reposition {
                    self.steering = Default::default();
                    self.following = false;
                    self.side_next = !self.side_next;
                    self.repositions += 1;
                } else if *mode == NavWalkKind::Recover {
                    self.repositions = 0;
                    self.side_next = false;
                    self.steering = Default::default();
                    self.following = false;
                } else {
                    self.motion.on_forward(*ms);
                    self.steering = p.steering.clone();
                    self.steering.clear_feedback();
                    self.following = true;
                }
            }
        }
        self.pending = None;
        self.current = None;
        self.ready = None;
        self.receipt = None;
        Ok(())
    }
    pub fn not_posted(&mut self, action: &NavAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.as_ref().is_none_or(|p| &p.action != action) {
            return Err("导航未投递动作不一致");
        }
        if matches!(
            action,
            NavAction::Interact {
                target: NpcTarget::Entry(_)
            }
        ) {
            self.departure = None;
        }
        self.pending = None;
        self.invalidate_observation();
        Ok(())
    }
    /// 只有本次固定镜头、实际持有方向期间的采样才能推进脱困；不记录位置或到达。
    pub fn motion_observed(
        &mut self,
        direction: MotionDirection,
        result: MotionResult,
    ) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.as_ref().is_none_or(|p| {
            p.action
                != (NavAction::Motion {
                    action: MotionAction::Probe { direction },
                })
        }) {
            return Err("背景运动没有对应的探测意图");
        }
        self.motion.observed(direction, result)?;
        self.pending = None;
        self.steering = Default::default();
        self.candidate_steering = Default::default();
        self.following = direction == MotionDirection::Forward && result == MotionResult::Moving;
        self.invalidate_observation();
        Ok(())
    }
    /// 已停止并释放的短段位移不计为完成；后续必须按新目标画面重新规划。
    pub fn interrupted_movement(&mut self, action: &NavAction) -> Result<(), &'static str> {
        if matches!(action, NavAction::Interact { .. }) {
            return Err("交互提交不能按部分位移清理");
        }
        self.not_posted(action)?;
        self.steering = Default::default();
        self.candidate_steering = Default::default();
        self.following = false;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.as_ref().is_some_and(|p| {
            matches!(
                p.action,
                NavAction::Motion {
                    action: MotionAction::ClearObstacle
                }
            )
        }) {
            self.motion.clear_submitted()?;
        }
        self.motion.resume();
        if self
            .pending
            .as_ref()
            .is_some_and(|p| !matches!(p.action, NavAction::Interact { .. }))
        {
            self.pending = None;
        }
        self.track = None;
        self.following = false;
        self.steering = Default::default();
        self.candidate_steering = Default::default();
        self.invalidate_observation();
        Ok(())
    }
}
