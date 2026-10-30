//! 从大世界、手册或迷失之地入口到冻结副本详情；只交接入口，不出战或记次数。
use crate::{
    lost_void::{LostVoidConfig, LostVoidMission as M, LostVoidTask},
    lost_void_scene::{BountyProgress, LostVoidObservation as V, LostVoidScreen as S},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidEntryRequest {
    pub run_id: Uuid,
    pub config: LostVoidConfig,
    pub timeout_ms: u32,
}
impl LostVoidEntryRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate()?;
        if self.run_id.is_nil() || !(1000..=300_000).contains(&self.timeout_ms) {
            return Err("迷失之地入口身份或时间预算无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryAction {
    Open,
    CombatTab,
    Category,
    Scroll(u8),
    Go,
    Confirm,
    Standard,
    Periodic,
    Mission,
    Back,
    CloseOverlay,
}
/// 仅供入口执行器消费；同一关闭区域内的悬赏面板不能冒充信息提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryOverlayKind {
    Information,
    Bounty,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryOverlay {
    kind: EntryOverlayKind,
    reads: u8,
    dispatch: Option<Posted>,
    clear_screen: Option<S>,
    clear_reads: u8,
    cleared: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryOutcome {
    MissionReady,
    BountyComplete,
    HubReady,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidEntryReceipt {
    pub run_id: Uuid,
    pub mission: M,
    pub bounty: Option<BountyProgress>,
    pub outcome: EntryOutcome,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Inspect,
    Open,
    Tab,
    Category,
    Search,
    Go,
    Confirm,
    Arrival,
    Hub,
    TabSwitch,
    Mission,
    MissionArrival,
    Back,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Posted {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidEntryFlow {
    request: LostVoidEntryRequest,
    phase: Phase,
    pending: Option<(EntryAction, Posted)>,
    category: Option<String>,
    scrolls: u8,
    #[serde(default)]
    hub_only: bool,
    #[serde(default)]
    hub_screen: Option<S>,
    #[serde(default)]
    hub_reads: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    overlay: Option<EntryOverlay>,
    receipt: Option<LostVoidEntryReceipt>,
}
impl LostVoidEntryFlow {
    pub fn new(request: LostVoidEntryRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            phase: Phase::Inspect,
            pending: None,
            category: None,
            scrolls: 0,
            hub_only: false,
            hub_screen: None,
            hub_reads: 0,
            overlay: None,
            receipt: None,
        })
    }
    /// 仅到入口枢纽，不选择副本；供已达到挑战目标后的领取任务使用。
    pub fn for_bounty(request: LostVoidEntryRequest) -> Result<Self, &'static str> {
        let mut flow = Self::new(request)?;
        flow.hub_only = true;
        Ok(flow)
    }
    pub fn hub_only(&self) -> bool {
        self.hub_only
    }
    pub fn request(&self) -> &LostVoidEntryRequest {
        &self.request
    }
    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }
    pub fn receipt(&self) -> Option<&LostVoidEntryReceipt> {
        self.receipt.as_ref()
    }
    pub fn completed(&self) -> bool {
        self.phase == Phase::Done
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.overlay.as_ref().is_some_and(|o| {
            o.reads > 2
                || o.clear_reads > 2
                || o.clear_screen.is_some() != (o.clear_reads > 0)
                || o.clear_screen.is_some_and(|s| !Self::entry_page(s))
                || o.dispatch == Some(Posted::Intent) && o.reads != 2
                || o.dispatch != Some(Posted::Posted) && o.clear_reads != 0
                || o.cleared != (o.clear_reads == 2)
                || self.completed() && !o.cleared
        }) {
            return Err("迷失之地入口遮罩检查点不一致");
        }
        if self.scrolls > 10
            || self.hub_reads > 2
            || self.hub_screen.is_some() != (self.hub_reads > 0)
            || self
                .hub_screen
                .is_some_and(|s| ![S::EntryStandard, S::EntryPeriodic].contains(&s))
            || !self.hub_only && self.hub_reads != 0
            || self.hub_only
                && matches!(
                    self.phase,
                    Phase::TabSwitch | Phase::Mission | Phase::MissionArrival
                )
            || (self.category.is_none()
                && (matches!(
                    self.phase,
                    Phase::Search | Phase::Go | Phase::Confirm | Phase::Arrival
                ) || self
                    .pending
                    .is_some_and(|(a, _)| a == EntryAction::Category)))
            || self
                .category
                .as_ref()
                .is_some_and(|s| !["周期征讨", "零号空洞"].contains(&s.as_str()))
            || self.completed() != self.receipt.is_some()
            || self.pending.is_some_and(|(a, _)| !self.phase_allows(a))
            || self.receipt.as_ref().is_some_and(|r| {
                r.run_id != self.request.run_id
                    || r.mission != self.request.config.mission_name
                    || (r.outcome == EntryOutcome::HubReady) != self.hub_only
                    || r.outcome == EntryOutcome::HubReady && self.hub_reads != 2
                    || r.bounty
                        .is_some_and(|p| p.total != 8000 || p.current > p.total)
                    || (r.outcome == EntryOutcome::BountyComplete
                        && (self.request.config.extra_task != LostVoidTask::BountyCommission
                            || !r.bounty.is_some_and(BountyProgress::completed)))
            })
        {
            return Err("迷失之地入口检查点不一致");
        }
        Ok(())
    }
    fn mission_screen(&self) -> S {
        match self.request.config.mission_name {
            M::Purge => S::Purge,
            M::TaskForce => S::TaskForce,
            M::MatrixAction => S::Matrix,
        }
    }
    fn hub(&self) -> S {
        if self.request.config.mission_name == M::MatrixAction {
            S::EntryPeriodic
        } else {
            S::EntryStandard
        }
    }
    fn phase_allows(&self, a: EntryAction) -> bool {
        use EntryAction as A;
        if self.hub_only && matches!(a, A::Standard | A::Periodic | A::Mission) {
            return false;
        }
        matches!(
            (self.phase, a),
            (Phase::Open, A::Open)
                | (Phase::Tab, A::CombatTab)
                | (Phase::Category, A::Category)
                | (Phase::Go, A::Go)
                | (Phase::Confirm | Phase::Arrival, A::Confirm)
                | (Phase::TabSwitch, A::Standard | A::Periodic)
                | (Phase::Mission | Phase::MissionArrival, A::Mission)
                | (Phase::Back, A::Back)
        ) || matches!((self.phase,a),(Phase::Search,A::Scroll(n)) if n == self.scrolls && n < 10)
    }
    fn page_allows(&self, a: EntryAction, v: &V) -> bool {
        use EntryAction as A;
        if v.entry_overlay.is_some() {
            return a == A::CloseOverlay
                && v.screen == S::Unknown
                && self
                    .overlay
                    .as_ref()
                    .is_some_and(|o| !o.cleared && Some(o.kind) == v.entry_overlay);
        }
        let h = v.handbook.as_ref();
        let valid_types = |h: &crate::lost_void_scene::LostVoidHandbook| {
            !h.types.is_empty()
                && h.types.len() <= 32
                && h.types
                    .iter()
                    .all(|s| crate::lost_void_scene::HANDBOOK_MISSIONS.contains(&s.as_str()))
        };
        match a {
            A::Open => v.screen == S::NormalWorld,
            A::CombatTab => v.screen == S::Handbook && h.is_some_and(|h| !h.combat),
            A::Category => {
                v.screen == S::Handbook
                    && h.is_some_and(|h| {
                        h.combat
                            && self
                                .category
                                .as_ref()
                                .is_some_and(|c| h.categories.contains(c))
                    })
            }
            A::Go => {
                v.screen == S::Handbook
                    && self.category.is_some()
                    && h.is_some_and(|h| {
                        h.combat
                            && valid_types(h)
                            && h.selected_category == self.category
                            && h.target_go
                            && h.types.iter().filter(|t| *t == "迷失之地").count() == 1
                    })
            }
            A::Scroll(_) => {
                v.screen == S::Handbook
                    && self.category.is_some()
                    && h.is_some_and(|h| {
                        h.combat
                            && h.selected_category == self.category
                            && !h.target_go
                            && valid_types(h)
                            && !h.types.iter().any(|t| t == "迷失之地")
                    })
            }
            A::Confirm => v.screen == S::TransportConfirm,
            A::Standard => v.screen == S::EntryPeriodic && self.hub() == S::EntryStandard,
            A::Periodic => v.screen == S::EntryStandard && self.hub() == S::EntryPeriodic,
            A::Mission => {
                v.screen == self.hub() && v.missions.contains(&self.request.config.mission_name)
            }
            A::Back => {
                [S::Purge, S::TaskForce, S::Matrix].contains(&v.screen)
                    && (self.hub_only || v.screen != self.mission_screen())
            }
            A::CloseOverlay => false,
        }
    }
    fn entry_page(s: S) -> bool {
        matches!(
            s,
            S::EntryStandard | S::EntryPeriodic | S::Purge | S::TaskForce | S::Matrix
        )
    }
    /// 遮罩不改变原导航阶段和传送意图。已尝试关闭后只等待新入口帧，不重发。
    fn overlay_next(&mut self, v: &V) -> Result<bool, &'static str> {
        if let Some(kind) = v.entry_overlay {
            if v.screen != S::Unknown {
                return Err("迷失之地入口遮罩与底层画面冲突");
            }
            self.hub_reads = 0;
            self.hub_screen = None;
            if let Some(o) = &mut self.overlay {
                o.clear_reads = 0;
                o.clear_screen = None;
                o.cleared = false;
                if o.dispatch.is_some() {
                    return Ok(true);
                }
                o.reads = if o.kind == kind {
                    (o.reads + 1).min(2)
                } else {
                    1
                };
                o.kind = kind;
            } else {
                self.overlay = Some(EntryOverlay {
                    kind,
                    reads: 1,
                    dispatch: None,
                    clear_screen: None,
                    clear_reads: 0,
                    cleared: false,
                });
            }
            return Ok(true);
        }
        if let Some(o) = &mut self.overlay {
            if o.dispatch.is_none() {
                self.overlay = None;
            } else if !o.cleared {
                if o.dispatch == Some(Posted::Intent) {
                    // 未取得投递结果，后续画面也不能授权重发。
                    o.dispatch = Some(Posted::Posted);
                }
                o.clear_reads = if Self::entry_page(v.screen) {
                    if o.clear_screen == Some(v.screen) {
                        (o.clear_reads + 1).min(2)
                    } else {
                        1
                    }
                } else {
                    0
                };
                o.clear_screen = (o.clear_reads > 0).then_some(v.screen);
                o.cleared = o.clear_reads == 2;
                return Ok(!o.cleared);
            }
        }
        Ok(false)
    }
    fn finish(&mut self, v: &V, outcome: EntryOutcome) {
        self.receipt = Some(LostVoidEntryReceipt {
            run_id: self.request.run_id,
            mission: self.request.config.mission_name,
            bounty: v.bounty,
            outcome,
        });
        self.phase = Phase::Done;
        self.pending = None;
    }
    pub fn next(&mut self, v: &V) -> Result<Option<EntryAction>, &'static str> {
        self.validate()?;
        if v.bounty
            .is_some_and(|p| p.total != 8000 || p.current > p.total)
        {
            return Err("迷失之地悬赏进度无效");
        }
        if self.completed() {
            return Ok(None);
        }
        if self.overlay_next(v)? {
            return Ok(self
                .overlay
                .as_ref()
                .is_some_and(|o| o.reads == 2 && o.dispatch.is_none())
                .then_some(EntryAction::CloseOverlay));
        }
        if [
            S::Modal,
            S::World,
            S::CommonChoice,
            S::Gear,
            S::GearConfirm,
            S::Event,
            S::Store,
            S::Lottery,
            S::RouteChange,
            S::BattleResult,
            S::BattleFail,
            S::Deploy,
            S::MatrixTeam,
        ]
        .contains(&v.screen)
        {
            return Err("迷失之地入口遇到局内或未知弹窗画面，不能继续导航");
        }
        if self.hub_only && ![S::EntryStandard, S::EntryPeriodic].contains(&v.screen) {
            self.hub_reads = 0;
            self.hub_screen = None;
        }
        if self.phase == Phase::Inspect {
            self.phase = match v.screen {
                S::NormalWorld => Phase::Open,
                S::Handbook => Phase::Tab,
                S::EntryStandard | S::EntryPeriodic | S::Purge | S::TaskForce | S::Matrix => {
                    Phase::Hub
                }
                S::TransportConfirm => return Err("缺少本次迷失之地传送意图，不能采用已有确认框"),
                _ => return Ok(None),
            };
        }
        if let Some((a, _)) = self.pending {
            let p = match a {
                EntryAction::Open if v.screen == S::Handbook => Some(Phase::Tab),
                EntryAction::CombatTab
                    if v.screen == S::Handbook && v.handbook.as_ref().is_some_and(|h| h.combat) =>
                {
                    Some(Phase::Category)
                }
                EntryAction::Category
                    if v.screen == S::Handbook
                        && v.handbook
                            .as_ref()
                            .is_some_and(|h| h.combat && h.selected_category == self.category) =>
                {
                    Some(Phase::Search)
                }
                EntryAction::Go if v.screen == S::TransportConfirm => Some(Phase::Confirm),
                EntryAction::Confirm
                    if [S::EntryStandard, S::EntryPeriodic].contains(&v.screen) =>
                {
                    Some(Phase::Hub)
                }
                EntryAction::Standard | EntryAction::Periodic if v.screen == self.hub() => {
                    Some(Phase::Hub)
                }
                EntryAction::Mission if v.screen == self.mission_screen() => Some(Phase::Hub),
                EntryAction::Back if [S::EntryStandard, S::EntryPeriodic].contains(&v.screen) => {
                    Some(Phase::Hub)
                }
                _ => None,
            };
            let Some(p) = p else {
                return Ok(None);
            };
            self.pending = None;
            self.phase = p;
        }
        if self.phase == Phase::Hub && self.hub_only {
            if [S::EntryStandard, S::EntryPeriodic].contains(&v.screen) {
                self.hub_reads = if self.hub_screen == Some(v.screen) {
                    (self.hub_reads + 1).min(2)
                } else {
                    1
                };
                self.hub_screen = Some(v.screen);
                if self.hub_reads == 2 {
                    self.finish(v, EntryOutcome::HubReady);
                }
                self.validate()?;
                return Ok(None);
            }
            if [S::Purge, S::TaskForce, S::Matrix].contains(&v.screen) {
                self.phase = Phase::Back;
            } else {
                return Ok(None);
            }
        }
        if self.phase == Phase::Hub {
            if self.request.config.extra_task == LostVoidTask::BountyCommission {
                let Some(progress) = v.bounty else {
                    return Ok(None);
                };
                if progress.completed() {
                    self.finish(v, EntryOutcome::BountyComplete);
                    return Ok(None);
                }
            }
            if v.screen == self.mission_screen() {
                self.finish(v, EntryOutcome::MissionReady);
                return Ok(None);
            }
            self.phase = if [S::Purge, S::TaskForce, S::Matrix].contains(&v.screen) {
                Phase::Back
            } else if v.screen == self.hub() {
                Phase::Mission
            } else if [S::EntryStandard, S::EntryPeriodic].contains(&v.screen) {
                Phase::TabSwitch
            } else {
                return Ok(None);
            };
        }
        if self.phase == Phase::Tab
            && v.screen == S::Handbook
            && v.handbook.as_ref().is_some_and(|h| h.combat)
        {
            self.phase = Phase::Category;
        }
        if self.phase == Phase::Category
            && self.category.is_none()
            && let Some(h) = &v.handbook
        {
            self.category = ["周期征讨", "零号空洞"]
                .into_iter()
                .find(|c| h.categories.iter().any(|name| name == c))
                .map(String::from);
        }
        if self.phase == Phase::Category
            && v.screen == S::Handbook
            && self.category.is_some()
            && v.handbook
                .as_ref()
                .is_some_and(|h| h.combat && h.selected_category == self.category)
        {
            self.phase = Phase::Search;
        }
        if self.phase == Phase::Search && self.page_allows(EntryAction::Go, v) {
            self.phase = Phase::Go;
        }
        if self.phase == Phase::Search && self.scrolls >= 10 {
            return Err("迷失之地手册查找达到十次滚动上限");
        }
        let a = match self.phase {
            Phase::Open => EntryAction::Open,
            Phase::Tab => EntryAction::CombatTab,
            Phase::Category => EntryAction::Category,
            Phase::Search => EntryAction::Scroll(self.scrolls),
            Phase::Go => EntryAction::Go,
            Phase::Confirm => EntryAction::Confirm,
            Phase::TabSwitch => {
                if self.hub() == S::EntryPeriodic {
                    EntryAction::Periodic
                } else {
                    EntryAction::Standard
                }
            }
            Phase::Mission => EntryAction::Mission,
            Phase::Back => EntryAction::Back,
            _ => return Ok(None),
        };
        Ok(self.page_allows(a, v).then_some(a))
    }
    pub fn prepare(&mut self, a: EntryAction) -> Result<(), &'static str> {
        self.validate()?;
        if a == EntryAction::CloseOverlay {
            let o = self.overlay.as_mut().ok_or("缺少本次入口遮罩观察")?;
            if o.reads != 2 || o.dispatch.is_some() || o.cleared {
                return Err("迷失之地入口遮罩尚未稳定或已尝试关闭");
            }
            o.dispatch = Some(Posted::Intent);
            return self.validate();
        }
        if self.pending.is_some()
            || self.overlay.as_ref().is_some_and(|o| !o.cleared)
            || !self.phase_allows(a)
            || matches!(self.phase, Phase::Arrival | Phase::MissionArrival)
        {
            return Err("迷失之地入口没有待保存的新动作");
        }
        self.pending = Some((a, Posted::Intent));
        Ok(())
    }
    pub fn allows(&self, a: EntryAction, v: &V) -> bool {
        self.validate().is_ok()
            && if a == EntryAction::CloseOverlay {
                self.overlay.as_ref().is_some_and(|o| {
                    o.dispatch == Some(Posted::Intent) && o.reads == 2 && !o.cleared
                })
            } else {
                self.pending == Some((a, Posted::Intent))
                    && self.overlay.as_ref().is_none_or(|o| o.cleared)
            }
            && self.page_allows(a, v)
    }
    pub fn posted(&mut self, a: EntryAction) -> Result<(), &'static str> {
        if a == EntryAction::CloseOverlay {
            let o = self.overlay.as_mut().ok_or("缺少本次入口遮罩意图")?;
            if o.dispatch != Some(Posted::Intent) {
                return Err("迷失之地入口遮罩投递回执不匹配");
            }
            o.dispatch = Some(Posted::Posted);
            return self.validate();
        }
        if self.pending != Some((a, Posted::Intent)) {
            return Err("迷失之地入口投递回执不匹配");
        }
        match a {
            EntryAction::Scroll(_) => {
                self.scrolls += 1;
                self.pending = None;
            }
            EntryAction::Confirm => {
                self.phase = Phase::Arrival;
                self.pending = Some((a, Posted::Posted));
            }
            EntryAction::Mission => {
                self.phase = Phase::MissionArrival;
                self.pending = Some((a, Posted::Posted));
            }
            _ => self.pending = Some((a, Posted::Posted)),
        }
        self.validate()
    }
    pub fn not_posted(&mut self, a: EntryAction) -> Result<(), &'static str> {
        if a == EntryAction::CloseOverlay {
            if !self
                .overlay
                .as_ref()
                .is_some_and(|o| o.dispatch == Some(Posted::Intent))
            {
                return Err("已尝试关闭的入口遮罩不能撤销");
            }
            self.overlay = None;
            return self.validate();
        }
        if self.pending != Some((a, Posted::Intent)) {
            return Err("已投递迷失之地入口动作不能撤销");
        }
        self.pending = None;
        self.validate()
    }
    pub fn partial_scroll(&mut self, a: EntryAction) -> Result<(), &'static str> {
        if !matches!(a, EntryAction::Scroll(_)) {
            return Err("只有手册滚动可以报告半次投递");
        }
        self.not_posted(a)
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.hub_reads = 0;
        self.hub_screen = None;
        if let Some(o) = &mut self.overlay {
            o.reads = 0;
            o.clear_reads = 0;
            o.clear_screen = None;
            o.cleared = false;
            if o.dispatch.is_some() {
                o.dispatch = Some(Posted::Posted);
            }
        }
        if self.completed() {
            self.receipt = None;
            self.phase = Phase::Inspect;
            self.overlay = None;
        }
        Ok(())
    }
}
