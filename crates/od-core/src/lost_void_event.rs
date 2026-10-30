//! 对白与事件交接。以新帧内容变化确认输入，不以黑屏或消失的菜单推定下一层。
use crate::{
    lost_void::{ArtifactCandidate, ArtifactSelection, LostVoidChallenge},
    lost_void_choice::{LostVoidChoiceReceipt, LostVoidChoiceRequest},
    lost_void_gear::{LostVoidGearReceipt, LostVoidGearRequest},
    lost_void_interact::{InteractionKind, LostVoidInteractReceipt, LostVoidInteractRequest},
    lost_void_scene::{LostVoidObservation, LostVoidScreen as S},
    lost_void_store::{LostVoidStoreReceipt, LostVoidStoreRequest},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SPECIAL_DIALOGUES: [&str; 4] = [
    "似乎购买了充值卡就会得到齿轮硬币奖励，但是在离开之后身上的齿轮硬币都",
    "（声音消失了，伸手从裂隙那头好像摸到了什么）",
    "这位似曾相识的研究员为我们准备了一些「礼物」。",
    "但当正要选择的时候，她却拦住了我们。",
];
pub fn dialogue_text(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .replace('（', "(")
        .replace('）', ")")
        .replace(['“', '【'], "「")
        .replace(['”', '】'], "」")
}
pub fn known_narration(text: &str) -> bool {
    let text = dialogue_text(text);
    SPECIAL_DIALOGUES.iter().enumerate().any(|(i, s)| {
        let known = dialogue_text(s);
        if i == 0 {
            text.starts_with(&known)
        } else {
            text == known
        }
    })
}
pub fn gear_complete_notice(text: &str) -> bool {
    let t = dialogue_text(text);
    ![
        "消耗",
        "购买",
        "刷新",
        "放弃",
        "退出",
        "重开",
        "撤退",
        "返回大厅",
    ]
    .iter()
    .any(|w| t.contains(w))
        && [
            "所有战术棱镜均已升级",
            "所有战术棱镜已满级",
            "全部战术棱镜已满级",
            "战术棱镜均已满级",
        ]
        .iter()
        .any(|w| t.contains(w))
}
pub fn challenge_confirmation(text: &str) -> bool {
    let t = dialogue_text(text).replace(['·', '：', ':', '－'], "-");
    !["放弃", "退出", "重开", "结束", "撤退", "返回"]
        .iter()
        .any(|w| t.contains(w))
        && [
            "挑战-限时",
            "挑战-无伤",
            "挑战-收割",
            "开始挑战",
            "开启挑战",
        ]
        .iter()
        .any(|w| t.contains(w))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPageKind {
    Dialogue,
    ChallengeConfirm,
    GearNotice,
    BlackText,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventOption {
    pub slot: u8,
    pub text: Option<String>,
    pub icon: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventPage {
    pub kind: EventPageKind,
    pub speaker: Option<String>,
    pub text: String,
    pub options: Vec<EventOption>,
}
fn valid_text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}
impl EventPage {
    fn option_slots(&self) -> Vec<u8> {
        let has_text = self.options.iter().any(|o| o.text.is_some());
        self.options
            .iter()
            .filter(|o| !has_text || o.text.is_some())
            .map(|o| o.slot)
            .collect()
    }
    fn permits(&self, a: EventAction) -> bool {
        match (self.kind, a) {
            (EventPageKind::Dialogue, EventAction::Advance) => self.options.is_empty(),
            (EventPageKind::Dialogue, EventAction::Choose(slot)) => {
                self.option_slots().contains(&slot)
            }
            (
                EventPageKind::ChallengeConfirm
                | EventPageKind::GearNotice
                | EventPageKind::BlackText,
                EventAction::Confirm,
            ) => true,
            _ => false,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_text(&self.text, 2048)
            || self.speaker.as_ref().is_some_and(|s| !valid_text(s, 48))
            || self.options.len() > 8
            || self.options.iter().enumerate().any(|(i, o)| {
                o.slot != i as u8
                    || (o.text.is_none() && !o.icon)
                    || o.text.as_ref().is_some_and(|s| !valid_text(s, 512))
            })
            || match self.kind {
                EventPageKind::Dialogue => {
                    self.speaker.is_none()
                        && !known_narration(&self.text)
                        && !self.options.iter().any(|o| o.icon)
                }
                EventPageKind::ChallengeConfirm => {
                    self.speaker.is_some()
                        || !self.options.is_empty()
                        || !challenge_confirmation(&self.text)
                }
                EventPageKind::BlackText => self.speaker.is_some() || !self.options.is_empty(),
                EventPageKind::GearNotice => {
                    self.speaker.is_some()
                        || !self.options.is_empty()
                        || !gear_complete_notice(&self.text)
                }
            }
        {
            return Err("迷失之地对白、选项或专用确认依据无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidEventView {
    pub scene: LostVoidObservation,
    pub page: Option<EventPage>,
    #[serde(default)]
    pub loading: Option<crate::lost_void_loading::LoadingObservation>,
}
impl LostVoidEventView {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(loading) = &self.loading {
            loading.validate()?;
            if self.scene.screen != S::Unknown || self.page.is_some() {
                return Err("事件加载观察不能混用页面或输入依据");
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidEventRequest {
    pub run_id: Uuid,
    pub interaction_id: Uuid,
    pub challenge: LostVoidChallenge,
    pub options: ArtifactSelection,
    pub entry: bool,
    pub completed_first: Vec<ArtifactCandidate>,
    pub members: u8,
    pub allow_challenge_confirm: bool,
    pub allow_gear_notice: bool,
    pub timeout_ms: u32,
}
impl LostVoidEventRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        LostVoidGearRequest {
            run_id: self.run_id,
            interaction_id: self.interaction_id,
            challenge: self.challenge.clone(),
            options: self.options.clone(),
            entry: self.entry,
            completed_first: self.completed_first.clone(),
            timeout_ms: self.timeout_ms,
        }
        .validate()?;
        LostVoidStoreRequest {
            run_id: self.run_id,
            interaction_id: self.interaction_id,
            challenge: self.challenge.clone(),
            options: self.options.clone(),
            members: self.members,
            timeout_ms: self.timeout_ms,
        }
        .validate()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventAction {
    Advance,
    Choose(u8),
    Confirm,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventChildKind {
    Choice,
    Gear,
    Store,
    Lottery,
    RouteChange,
}
impl EventChildKind {
    fn from_screen(s: S) -> Option<Self> {
        Some(match s {
            S::CommonChoice => Self::Choice,
            S::Gear | S::GearConfirm => Self::Gear,
            S::Store => Self::Store,
            S::Lottery => Self::Lottery,
            S::RouteChange => Self::RouteChange,
            _ => return None,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventStep {
    Input(EventAction),
    Child,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "request", rename_all = "snake_case")]
pub enum EventChildRequest {
    Choice(LostVoidChoiceRequest),
    Gear(LostVoidGearRequest),
    Store(LostVoidStoreRequest),
    Interact(LostVoidInteractRequest),
}
#[derive(Debug, Clone)]
pub enum EventChildReceipt {
    Choice(LostVoidChoiceReceipt),
    Gear(LostVoidGearReceipt),
    Store(LostVoidStoreReceipt),
    Interact(LostVoidInteractReceipt),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidEventReceipt {
    pub run_id: Uuid,
    pub interaction_id: Uuid,
    pub returned_to: S,
    pub actions: u16,
    pub children: u16,
    pub completed_first: Vec<ArtifactCandidate>,
    pub append_priority: Vec<String>,
    pub gear_repeats: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Used {
    page: EventPage,
    action: EventAction,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    used: Used,
    posted: bool,
    cursor_before: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Child {
    id: Uuid,
    kind: EventChildKind,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidEventFlow {
    request: LostVoidEventRequest,
    current: Option<EventPage>,
    stable_reads: u8,
    ready: Option<EventAction>,
    pending: Option<Pending>,
    used: Vec<Used>,
    cursor: u8,
    entered: bool,
    world_reads: u8,
    child: Option<Child>,
    actions: u16,
    children: u16,
    completed_first: Vec<ArtifactCandidate>,
    append_priority: Vec<String>,
    gear_repeats: u8,
    receipt: Option<LostVoidEventReceipt>,
    #[serde(default)]
    departure: Option<crate::lost_void_loading::LoadingObservation>,
    #[serde(default)]
    challenge_confirmed: bool,
}
impl LostVoidEventFlow {
    pub fn new(request: LostVoidEventRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            completed_first: request.completed_first.clone(),
            request,
            current: None,
            stable_reads: 0,
            ready: None,
            pending: None,
            used: vec![],
            cursor: 0,
            entered: false,
            world_reads: 0,
            child: None,
            actions: 0,
            children: 0,
            append_priority: vec![],
            gear_repeats: 0,
            receipt: None,
            departure: None,
            challenge_confirmed: false,
        })
    }
    pub fn request(&self) -> &LostVoidEventRequest {
        &self.request
    }
    pub fn departure(&self) -> Option<&crate::lost_void_loading::LoadingObservation> {
        self.departure.as_ref()
    }
    pub fn challenge_confirmed(&self) -> bool {
        self.challenge_confirmed
    }
    pub fn confirmed_challenge(&self) -> Option<&EventPage> {
        self.challenge_confirmed
            .then(|| {
                self.used
                    .iter()
                    .find(|u| {
                        u.page.kind == EventPageKind::ChallengeConfirm
                            && u.action == EventAction::Confirm
                    })
                    .map(|u| &u.page)
            })
            .flatten()
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn receipt(&self) -> Option<&LostVoidEventReceipt> {
        self.receipt.as_ref()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if let Some(departure) = &self.departure {
            departure.validate()?;
            if !self.entered || self.actions == 0 && self.children == 0 && self.pending.is_none() {
                return Err("事件加载历史缺少本次已进行交互");
            }
        }
        if self.challenge_confirmed
            && (!self.request.allow_challenge_confirm
                || self.actions == 0
                || !self.used.iter().any(|u| {
                    u.page.kind == EventPageKind::ChallengeConfirm
                        && u.action == EventAction::Confirm
                }))
        {
            return Err("事件挑战确认历史缺少对应动作");
        }
        if let Some(p) = &self.current {
            p.validate()?;
        }
        for u in &self.used {
            u.page.validate()?;
            if !u.page.permits(u.action) {
                return Err("迷失之地事件动作与页面不匹配");
            }
            if u.page.kind == EventPageKind::ChallengeConfirm
                && !self.request.allow_challenge_confirm
            {
                return Err("迷失之地事件确认没有冻结的挑战来源");
            }
            if u.page.kind == EventPageKind::GearNotice && !self.request.allow_gear_notice {
                return Err("迷失之地事件通知没有冻结的战斗后来源");
            }
        }
        if self.stable_reads > 2
            || self.world_reads > 2
            || self.cursor > 8
            || self.actions > 128
            || self.children > 128
            || self.used.len() > 128
            || self.gear_repeats > 128
            || (self.current.is_none() != (self.stable_reads == 0))
            || (self.ready.is_some() && (self.pending.is_some() || self.child.is_some()))
            || (self.child.is_some() && self.pending.is_some())
            || self.ready.is_some_and(|a| {
                self.stable_reads != 2 || self.current.as_ref().is_none_or(|p| !p.permits(a))
            })
            || self.child.as_ref().is_some_and(|c| c.id.is_nil())
            || self
                .pending
                .as_ref()
                .is_some_and(|p| self.used.last() != Some(&p.used) || p.cursor_before > 8)
            || self
                .used
                .iter()
                .enumerate()
                .any(|(i, u)| self.used[..i].contains(u))
            || self.append_priority.len() > 16
            || self
                .append_priority
                .iter()
                .enumerate()
                .any(|(i, p)| !valid_text(p, 128) || self.append_priority[..i].contains(p))
            || self.receipt.as_ref().is_some_and(|r| {
                r.run_id != self.request.run_id
                    || r.interaction_id != self.request.interaction_id
                    || ![S::World, S::BattleResult, S::BattleFail].contains(&r.returned_to)
                    || r.actions != self.actions
                    || r.children != self.children
                    || r.completed_first != self.completed_first
                    || r.append_priority != self.append_priority
                    || r.gear_repeats != self.gear_repeats
                    || self.child.is_some()
                    || self.pending.is_some()
            })
        {
            return Err("迷失之地事件检查点不一致");
        }
        let mut gear = self.request.clone();
        gear.completed_first = self.completed_first.clone();
        gear.validate()
    }
    fn acknowledge(&mut self) {
        if let Some(pending) = self.pending.take() {
            self.challenge_confirmed |= pending.used.page.kind == EventPageKind::ChallengeConfirm
                && pending.used.action == EventAction::Confirm;
            self.actions += 1;
        }
    }
    fn finish(&mut self, returned_to: S) {
        self.acknowledge();
        self.receipt = Some(LostVoidEventReceipt {
            run_id: self.request.run_id,
            interaction_id: self.request.interaction_id,
            returned_to,
            actions: self.actions,
            children: self.children,
            completed_first: self.completed_first.clone(),
            append_priority: self.append_priority.clone(),
            gear_repeats: self.gear_repeats,
        });
    }
    pub fn next(&mut self, v: &LostVoidEventView) -> Result<Option<EventStep>, &'static str> {
        self.validate()?;
        v.validate()?;
        self.ready = None;
        if self.completed() || self.child.is_some() {
            return Ok(None);
        }
        if let Some(loading) = &v.loading {
            if self.entered && (self.actions > 0 || self.children > 0 || self.pending.is_some()) {
                self.departure = Some(loading.clone());
            }
            self.current = None;
            self.stable_reads = 0;
            self.world_reads = 0;
            return Ok(None);
        }
        if v.scene.screen == S::World && v.page.is_none() {
            self.current = None;
            self.stable_reads = 0;
            if self.entered {
                self.world_reads = (self.world_reads + 1).min(2);
                if self.world_reads == 2 {
                    self.finish(S::World);
                }
            }
            return Ok(None);
        }
        self.world_reads = 0;
        if [S::BattleResult, S::BattleFail].contains(&v.scene.screen) {
            self.finish(v.scene.screen);
            return Ok(None);
        }
        if let Some(kind) = EventChildKind::from_screen(v.scene.screen) {
            if self.children >= 128 {
                return Err("迷失之地事件子流程达到上限");
            }
            self.acknowledge();
            self.entered = true;
            self.current = None;
            self.stable_reads = 0;
            self.child = Some(Child {
                id: Uuid::new_v4(),
                kind,
            });
            return Ok(Some(EventStep::Child));
        }
        if v.scene.screen == S::Unknown {
            return Ok(None);
        }
        if v.scene.screen != S::Event {
            return Err("迷失之地事件遇到非预期页面");
        }
        let Some(p) = &v.page else { return Ok(None) };
        p.validate()?;
        self.entered = true;
        if p.kind == EventPageKind::ChallengeConfirm && !self.request.allow_challenge_confirm {
            return Err("迷失之地本次事件没有挑战确认来源");
        }
        if p.kind == EventPageKind::GearNotice && !self.request.allow_gear_notice {
            return Err("迷失之地本次事件没有战斗后通知来源");
        }
        if self.current.as_ref() == Some(p) {
            self.stable_reads = (self.stable_reads + 1).min(2);
        } else {
            self.current = Some(p.clone());
            self.stable_reads = 1;
        }
        if self.stable_reads < 2 {
            return Ok(None);
        }
        if let Some(pending) = &self.pending {
            if &pending.used.page == p {
                return Ok(None);
            }
            self.acknowledge();
        }
        if self.actions >= 128 || self.used.len() >= 128 {
            return Err("迷失之地事件动作达到上限");
        }
        let candidates = if p.kind != EventPageKind::Dialogue {
            vec![EventAction::Confirm]
        } else if p.options.is_empty() {
            vec![EventAction::Advance]
        } else {
            let slots = p.option_slots();
            let start = if (self.cursor as usize) < slots.len() {
                self.cursor as usize
            } else {
                0
            };
            (0..slots.len())
                .map(|i| EventAction::Choose(slots[(start + i) % slots.len()]))
                .collect()
        };
        let Some(a) = candidates.into_iter().find(|a| {
            !self.used.contains(&Used {
                page: p.clone(),
                action: *a,
            })
        }) else {
            return Ok(None);
        };
        self.ready = Some(a);
        Ok(Some(EventStep::Input(a)))
    }
    pub fn prepare(&mut self, a: EventAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.ready != Some(a)
            || self.pending.is_some()
            || self.child.is_some()
            || self.completed()
        {
            return Err("迷失之地没有待保存的事件动作");
        }
        let used = Used {
            page: self.current.clone().ok_or("迷失之地事件页面缺失")?,
            action: a,
        };
        if self.used.contains(&used) {
            return Err("迷失之地事件动作不能重复");
        }
        let cursor_before = self.cursor;
        self.cursor = match a {
            EventAction::Choose(slot) => {
                used.page
                    .option_slots()
                    .iter()
                    .position(|s| *s == slot)
                    .ok_or("迷失之地选项不是本次可用文字或图标")? as u8
                    + 1
            }
            _ => 0,
        };
        self.used.push(used.clone());
        self.pending = Some(Pending {
            used,
            posted: false,
            cursor_before,
        });
        self.ready = None;
        self.validate()
    }
    pub fn allows(&self, a: EventAction, v: &LostVoidEventView) -> bool {
        self.validate().is_ok()
            && v.loading.is_none()
            && v.scene.screen == S::Event
            && self.pending.as_ref().is_some_and(|p| {
                !p.posted && p.used.action == a && v.page.as_ref() == Some(&p.used.page)
            })
    }
    pub fn posted(&mut self, a: EventAction) -> Result<(), &'static str> {
        let p = self
            .pending
            .as_mut()
            .filter(|p| !p.posted && p.used.action == a)
            .ok_or("迷失之地事件投递回执不匹配")?;
        p.posted = true;
        self.validate()
    }
    pub fn not_posted(&mut self, a: EventAction) -> Result<(), &'static str> {
        let p = self
            .pending
            .as_ref()
            .filter(|p| !p.posted && p.used.action == a)
            .ok_or("迷失之地已投递事件动作不能撤销")?;
        self.cursor = p.cursor_before;
        self.used.pop();
        self.pending = None;
        self.validate()
    }
    pub fn child_request(&self) -> Result<EventChildRequest, &'static str> {
        self.validate()?;
        let c = self.child.as_ref().ok_or("迷失之地事件子流程缺失")?;
        let r = &self.request;
        let mut options = r.options.clone();
        options.battle_priority.extend(self.append_priority.clone());
        Ok(match c.kind {
            EventChildKind::Choice => EventChildRequest::Choice(LostVoidChoiceRequest {
                run_id: r.run_id,
                interaction_id: c.id,
                challenge: r.challenge.clone(),
                options,
                return_to: vec![
                    S::World,
                    S::Event,
                    S::Gear,
                    S::Store,
                    S::Lottery,
                    S::RouteChange,
                ],
                timeout_ms: r.timeout_ms,
            }),
            EventChildKind::Gear => EventChildRequest::Gear(LostVoidGearRequest {
                run_id: r.run_id,
                interaction_id: c.id,
                challenge: r.challenge.clone(),
                options,
                entry: r.entry,
                completed_first: self.completed_first.clone(),
                timeout_ms: r.timeout_ms,
            }),
            EventChildKind::Store => EventChildRequest::Store(LostVoidStoreRequest {
                run_id: r.run_id,
                interaction_id: c.id,
                challenge: r.challenge.clone(),
                options,
                members: r.members,
                timeout_ms: r.timeout_ms,
            }),
            EventChildKind::Lottery | EventChildKind::RouteChange => {
                EventChildRequest::Interact(LostVoidInteractRequest {
                    run_id: r.run_id,
                    interaction_id: c.id,
                    challenge: r.challenge.clone(),
                    options,
                    kind: if c.kind == EventChildKind::Lottery {
                        InteractionKind::Lottery
                    } else {
                        InteractionKind::RouteChange
                    },
                    timeout_ms: r.timeout_ms,
                })
            }
        })
    }
    pub fn accept_child(&mut self, r: &EventChildReceipt) -> Result<(), &'static str> {
        self.validate()?;
        let c = self.child.as_ref().ok_or("迷失之地事件子流程缺失")?;
        let valid = match r {
            EventChildReceipt::Choice(r) => (1..=16).contains(&r.confirmed_pages),
            EventChildReceipt::Gear(r) => {
                [S::World, S::Event].contains(&r.returned_to)
                    && (!r.repeated
                        || (self.request.entry && r.selected.is_none() && !r.append_priority))
                    && (!r.append_priority
                        || r.selected
                            .as_ref()
                            .is_some_and(crate::lost_void_gear::valid_gear))
                    && r.first
                        .as_ref()
                        .is_none_or(|a| a.slot == 0 && crate::lost_void_gear::valid_gear(a))
            }
            EventChildReceipt::Store(r) => {
                [S::World, S::Event].contains(&r.returned_to)
                    && r.purchases <= 4096
                    && r.refreshes <= 4096
            }
            EventChildReceipt::Interact(r) => {
                [S::World, S::Event].contains(&r.returned_to)
                    && r.draws <= 32
                    && (r.kind != InteractionKind::RouteChange || r.draws == 0)
            }
        };
        let (run, id, kind, returned) = match r {
            EventChildReceipt::Choice(r) => (
                r.run_id,
                r.interaction_id,
                EventChildKind::Choice,
                r.returned_to,
            ),
            EventChildReceipt::Gear(r) => (
                r.run_id,
                r.interaction_id,
                EventChildKind::Gear,
                r.returned_to,
            ),
            EventChildReceipt::Store(r) => (
                r.run_id,
                r.interaction_id,
                EventChildKind::Store,
                r.returned_to,
            ),
            EventChildReceipt::Interact(r) => (
                r.run_id,
                r.interaction_id,
                if r.kind == InteractionKind::Lottery {
                    EventChildKind::Lottery
                } else {
                    EventChildKind::RouteChange
                },
                r.returned_to,
            ),
        };
        if !valid
            || run != self.request.run_id
            || id != c.id
            || kind != c.kind
            || ![
                S::World,
                S::Event,
                S::Gear,
                S::Store,
                S::Lottery,
                S::RouteChange,
            ]
            .contains(&returned)
        {
            return Err("迷失之地事件回执不属于本次子流程");
        }
        if let EventChildReceipt::Gear(r) = r {
            if r.repeated {
                self.gear_repeats += 1;
            } else if self.request.entry
                && let Some(first) = &r.first
                && !self.completed_first.contains(first)
            {
                self.completed_first.push(first.clone());
            }
            if r.append_priority
                && let Some(selected) = &r.selected
                && !self.append_priority.contains(&selected.category)
            {
                self.append_priority.push(selected.category.clone());
            }
        }
        self.children += 1;
        self.child = None;
        self.current = None;
        self.stable_reads = 0;
        self.world_reads = 0;
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.receipt = None;
        self.ready = None;
        self.current = None;
        self.stable_reads = 0;
        self.world_reads = 0;
        Ok(())
    }
}
