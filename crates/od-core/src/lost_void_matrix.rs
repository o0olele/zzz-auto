//! 矩阵主战、协战准备；完成回执停留在编队页，不发送开始挑战。
use crate::{
    lost_void::{LostVoidChallenge, LostVoidMission as M, artifacts},
    lost_void_scene::LostVoidScreen as S,
    lost_void_team_plan::{LostVoidTeamKind, LostVoidTeamPlan, plan_team},
    predefined_teams::TeamConfig,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixPane {
    Unknown,
    Agents,
    Presets,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixRole {
    Main,
    Support,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum MatrixSlot {
    Unknown,
    Empty,
    Occupied {
        agent_id: Option<String>,
        signature: String,
        role: MatrixRole,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MatrixCandidateKind {
    Agent {
        agent_id: Option<String>,
        level: Option<u8>,
        up: bool,
    },
    Preset {
        team_index: Option<u8>,
        name: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatrixCandidate {
    pub slot: u8,
    pub signature: String,
    pub item: MatrixCandidateKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatrixView {
    pub screen: S,
    pub tutorial: bool,
    pub pane: MatrixPane,
    pub main: [MatrixSlot; 3],
    pub support: MatrixSlot,
    pub candidates: Vec<MatrixCandidate>,
    pub support_rules: Vec<String>,
    pub next_enabled: bool,
    pub start_enabled: bool,
}
impl Default for MatrixView {
    fn default() -> Self {
        Self {
            screen: S::Unknown,
            tutorial: false,
            pane: MatrixPane::Unknown,
            main: std::array::from_fn(|_| MatrixSlot::Unknown),
            support: MatrixSlot::Unknown,
            candidates: vec![],
            support_rules: vec![],
            next_enabled: false,
            start_enabled: false,
        }
    }
}
fn known(id: &str) -> bool {
    crate::random_play::agents().iter().any(|a| a.id == id)
}
pub fn matrix_signature(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
/// 只采用完整类别；兼容一对完整括号，不从任意说明中截取两字。
pub fn matrix_property(text: &str) -> Option<String> {
    let s: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let s = s
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .or_else(|| s.strip_prefix('【').and_then(|s| s.strip_suffix('】')))
        .unwrap_or(&s);
    artifacts()
        .ok()?
        .into_iter()
        .any(|a| a.category == s)
        .then(|| s.to_string())
}
impl MatrixView {
    pub fn validate(&self) -> Result<(), &'static str> {
        let slot = |s: &MatrixSlot, role| match s {
            MatrixSlot::Occupied {
                agent_id,
                signature,
                role: r,
            } => {
                *r == role
                    && matrix_signature(signature)
                    && agent_id.as_ref().is_none_or(|id| known(id))
            }
            _ => true,
        };
        if !self.main.iter().all(|s| slot(s, MatrixRole::Main))
            || !slot(&self.support, MatrixRole::Support)
            || self.candidates.len() > 64
            || self.candidates.iter().any(|c| match c.item {
                MatrixCandidateKind::Agent { .. } => self.pane != MatrixPane::Agents,
                MatrixCandidateKind::Preset { .. } => self.pane != MatrixPane::Presets,
            })
            || self.candidates.iter().enumerate().any(|(i, c)| {
                c.slot != i as u8
                    || !matrix_signature(&c.signature)
                    || match &c.item {
                        MatrixCandidateKind::Agent {
                            agent_id, level, ..
                        } => {
                            agent_id.as_ref().is_some_and(|id| !known(id))
                                || level.is_some_and(|v| v > 90)
                        }
                        MatrixCandidateKind::Preset { team_index, name } => {
                            team_index.is_some_and(|v| v >= 20)
                                || name.is_empty()
                                || name.chars().count() > 80
                                || name.chars().any(char::is_control)
                        }
                    }
            })
            || self.support_rules.len() > 4
            || self.support_rules.iter().enumerate().any(|(i, s)| {
                matrix_property(s).as_ref() != Some(s) || self.support_rules[..i].contains(s)
            })
            || self.tutorial
                && (self.screen != S::MatrixTeam
                    || self.pane != MatrixPane::Unknown
                    || !self.candidates.is_empty()
                    || self.main.iter().any(|s| *s != MatrixSlot::Unknown)
                    || self.support != MatrixSlot::Unknown)
        {
            return Err("矩阵页面身份、槽位、候选或属性无效");
        }
        Ok(())
    }
    pub fn main_agents(&self) -> Option<Vec<String>> {
        let mut out = Vec::new();
        let mut empty = false;
        for s in &self.main {
            match s {
                MatrixSlot::Empty => empty = true,
                MatrixSlot::Occupied {
                    agent_id: Some(id),
                    role: MatrixRole::Main,
                    ..
                } if !empty && !out.contains(id) => out.push(id.clone()),
                _ => return None,
            }
        }
        Some(out)
    }
    pub fn support_agent(&self) -> Option<&str> {
        if let MatrixSlot::Occupied {
            agent_id: Some(id),
            role: MatrixRole::Support,
            ..
        } = &self.support
        {
            Some(id)
        } else {
            None
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidMatrixRequest {
    pub run_id: Uuid,
    pub challenge: LostVoidChallenge,
    pub teams: TeamConfig,
    pub timeout_ms: u32,
}
impl LostVoidMatrixRequest {
    pub fn plan(&self) -> Result<LostVoidTeamPlan, &'static str> {
        if self.run_id.is_nil() || !(1000..=600_000).contains(&self.timeout_ms) {
            return Err("矩阵准备运行身份或预算无效");
        }
        plan_team(M::MatrixAction, &self.challenge, &self.teams, None, false)?
            .ok_or("矩阵主战方案无效")
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixAction {
    Next,
    DismissTutorial,
    AgentsTab,
    PresetsTab,
    FocusMain,
    RemoveMain(u8),
    Up,
    Down,
    PickAgent(u8),
    PickPreset(u8),
    FocusSupport,
    PickSupport(u8),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    Entry,
    Mode,
    MainFocus,
    Clear,
    Reset,
    MainSelect,
    PresetSelect,
    SupportFocus,
    SupportSelect,
    Verify,
    Done,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    action: MatrixAction,
    posted: bool,
    before: MatrixView,
    stable: Option<MatrixView>,
    reads: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidMatrixReceipt {
    pub run_id: Uuid,
    pub plan: LostVoidTeamPlan,
    pub main_agents: Vec<String>,
    pub support_agent: String,
    pub support_rules: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidMatrixFlow {
    request: LostVoidMatrixRequest,
    plan: LostVoidTeamPlan,
    stage: Stage,
    current: Option<MatrixView>,
    ready: Option<MatrixAction>,
    pending: Option<Pending>,
    chosen: u8,
    reset_left: u8,
    down: u8,
    main_agents: Vec<String>,
    support_wanted: Option<String>,
    support_rules: Vec<String>,
    verify_reads: u8,
    actions: u16,
    receipt: Option<LostVoidMatrixReceipt>,
}
impl LostVoidMatrixFlow {
    pub fn new(request: LostVoidMatrixRequest) -> Result<Self, &'static str> {
        let plan = request.plan()?;
        Ok(Self {
            request,
            plan,
            stage: Stage::Entry,
            current: None,
            ready: None,
            pending: None,
            chosen: 0,
            reset_left: 0,
            down: 0,
            main_agents: vec![],
            support_wanted: None,
            support_rules: vec![],
            verify_reads: 0,
            actions: 0,
            receipt: None,
        })
    }
    pub fn request(&self) -> &LostVoidMatrixRequest {
        &self.request
    }
    pub fn plan(&self) -> &LostVoidTeamPlan {
        &self.plan
    }
    pub fn current(&self) -> Option<&MatrixView> {
        self.current.as_ref()
    }
    pub fn before(&self) -> Option<&MatrixView> {
        self.pending.as_ref().map(|p| &p.before)
    }
    pub fn completed(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn receipt(&self) -> Option<&LostVoidMatrixReceipt> {
        self.receipt.as_ref()
    }
    pub fn needs_main_identity(&self) -> bool {
        matches!(
            self.stage,
            Stage::MainSelect
                | Stage::SupportFocus
                | Stage::SupportSelect
                | Stage::Verify
                | Stage::Done
        ) || self
            .pending
            .as_ref()
            .is_some_and(|p| matches!(p.action, MatrixAction::PickPreset(_)))
    }
    pub fn selecting_support(&self) -> bool {
        matches!(
            self.stage,
            Stage::SupportSelect | Stage::Verify | Stage::Done
        ) || self
            .pending
            .as_ref()
            .is_some_and(|p| p.action == MatrixAction::FocusSupport)
    }
    fn manual(&self) -> Option<&[String; 3]> {
        match &self.plan.kind {
            LostVoidTeamKind::MatrixManual { agents } => Some(agents),
            _ => None,
        }
    }
    fn reset(&mut self) {
        self.reset_left = if self.manual().is_some() { 4 } else { 8 };
        self.down = 0;
        self.stage = Stage::Reset;
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(v) = &self.current {
            v.validate()?;
        }
        if let Some(p) = &self.pending {
            p.before.validate()?;
            if !self.valid_intent(p) {
                return Err("矩阵准备动作、槽位与阶段不一致");
            }
            if let Some(v) = &p.stable {
                v.validate()?;
            }
        }
        if self.plan != self.request.plan()?
            || matches!(
                self.stage,
                Stage::MainFocus | Stage::Clear | Stage::MainSelect
            ) && self.manual().is_none()
            || self.stage == Stage::MainSelect && self.chosen >= 3
            || matches!(self.stage, Stage::MainFocus | Stage::Clear) && self.chosen != 0
            || self.stage == Stage::PresetSelect && self.manual().is_some()
            || matches!(
                self.stage,
                Stage::SupportFocus | Stage::SupportSelect | Stage::Verify | Stage::Done
            ) && (self.main_agents.is_empty()
                || self.plan.verify_roster(&self.main_agents) != Ok(true)
                || self.manual().is_some() && self.chosen != 3)
            || matches!(self.stage, Stage::Verify | Stage::Done)
                && (self.support_wanted.is_none() || self.support_rules.is_empty())
            || self.chosen > 3
            || self.reset_left > 8
            || self.down > 7
            || self.verify_reads > 2
            || self.actions > 192
            || self
                .pending
                .as_ref()
                .is_some_and(|p| p.reads > 1 || (p.reads == 0) != p.stable.is_none())
            || self.ready.is_some() && self.pending.is_some()
            || self.completed() != self.receipt.is_some()
            || self.main_agents.len() > 3
            || self
                .main_agents
                .iter()
                .enumerate()
                .any(|(i, id)| !known(id) || self.main_agents[..i].contains(id))
            || self.support_wanted.as_ref().is_some_and(|id| !known(id))
            || self
                .support_rules
                .iter()
                .any(|s| matrix_property(s).as_ref() != Some(s))
            || self.receipt.as_ref().is_some_and(|r| {
                r.run_id != self.request.run_id
                    || r.plan != self.plan
                    || r.main_agents != self.main_agents
                    || r.main_agents.is_empty()
                    || Some(&r.support_agent) != self.support_wanted.as_ref()
                    || r.support_rules != self.support_rules
                    || r.support_rules.is_empty()
                    || self.verify_reads != 2
            })
        {
            return Err("矩阵准备检查点与冻结方案或后态不一致");
        }
        Ok(())
    }
    fn valid_intent(&self, p: &Pending) -> bool {
        let v = &p.before;
        if !matches!(p.action, MatrixAction::Next | MatrixAction::DismissTutorial)
            && (v.screen != S::MatrixTeam || v.tutorial)
        {
            return false;
        }
        let agent = |slot: u8| {
            v.candidates.get(slot as usize).and_then(|c| {
                if let MatrixCandidateKind::Agent {
                    agent_id: Some(id), ..
                } = &c.item
                {
                    Some(id)
                } else {
                    None
                }
            })
        };
        match p.action {
            MatrixAction::Next => {
                self.stage == Stage::Entry && v.screen == S::Matrix && v.next_enabled
            }
            MatrixAction::DismissTutorial => v.screen == S::MatrixTeam && v.tutorial,
            MatrixAction::AgentsTab => {
                self.stage == Stage::Mode && self.manual().is_some() && v.pane != MatrixPane::Agents
            }
            MatrixAction::PresetsTab => {
                self.stage == Stage::Mode
                    && self.manual().is_none()
                    && v.pane != MatrixPane::Presets
            }
            MatrixAction::FocusMain => self.stage == Stage::MainFocus,
            MatrixAction::RemoveMain(i) => {
                self.stage == Stage::Clear
                    && !v.main.contains(&MatrixSlot::Unknown)
                    && v.main
                        .iter()
                        .rposition(|s| matches!(s, MatrixSlot::Occupied { .. }))
                        == Some(i as usize)
                    && v.main.get(i as usize).is_some_and(|s| {
                        matches!(
                            s,
                            MatrixSlot::Occupied {
                                role: MatrixRole::Main,
                                ..
                            }
                        )
                    })
            }
            MatrixAction::Up => {
                self.stage == Stage::Reset
                    && self.reset_left > 0
                    && v.pane
                        == if self.manual().is_some() {
                            MatrixPane::Agents
                        } else {
                            MatrixPane::Presets
                        }
            }
            MatrixAction::Down => {
                self.stage == Stage::MainSelect && v.pane == MatrixPane::Agents && self.down < 4
                    || self.stage == Stage::PresetSelect
                        && v.pane == MatrixPane::Presets
                        && self.down < 7
            }
            MatrixAction::PickAgent(i) => {
                self.stage == Stage::MainSelect
                    && self.prefix_matches(v, self.chosen as usize)
                    && self.manual().and_then(|a| a.get(self.chosen as usize)) == agent(i)
            }
            MatrixAction::PickPreset(i) => {
                self.stage == Stage::PresetSelect
                    && v.candidates.get(i as usize).is_some_and(|c| {
                        match (&c.item, &self.plan.kind) {
                            (
                                MatrixCandidateKind::Preset {
                                    team_index: Some(i),
                                    name,
                                },
                                LostVoidTeamKind::Predefined { index },
                            ) => {
                                i == index
                                    && crate::predefined_teams::normalize_name(name)
                                        == crate::predefined_teams::normalize_name(
                                            &self.request.teams.teams[*index as usize].name,
                                        )
                            }
                            _ => false,
                        }
                    })
            }
            MatrixAction::FocusSupport => self.stage == Stage::SupportFocus && self.main_matches(v),
            MatrixAction::PickSupport(i) => {
                self.stage == Stage::SupportSelect
                    && self.main_matches(v)
                    && self.support_wanted.is_some()
                    && agent(i) == self.support_wanted.as_ref()
            }
        }
    }
    fn main_matches(&self, v: &MatrixView) -> bool {
        v.main_agents().is_some_and(|a| a == self.main_agents)
    }
    fn prefix_matches(&self, v: &MatrixView, n: usize) -> bool {
        let Some(expected) = self.manual() else {
            return false;
        };
        expected
            .get(..n)
            .is_some_and(|expected| v.main_agents().is_some_and(|a| a == expected))
    }
    fn support_choice(v: &MatrixView) -> Option<&MatrixCandidate> {
        let up = v
            .candidates
            .iter()
            .any(|c| matches!(c.item, MatrixCandidateKind::Agent { up: true, .. }));
        let candidate = v.candidates.iter().find(|c| {
            matches!(c.item, MatrixCandidateKind::Agent { up: true, .. })
                || !up
                    && matches!(
                        c.item,
                        MatrixCandidateKind::Agent {
                            level: Some(60),
                            ..
                        }
                    )
        })?;
        matches!(
            candidate.item,
            MatrixCandidateKind::Agent {
                agent_id: Some(_),
                ..
            }
        )
        .then_some(candidate)
    }
    pub fn next(&mut self, v: &MatrixView) -> Result<Option<MatrixAction>, &'static str> {
        let mut next = self.clone();
        let result = next.next_inner(v)?;
        *self = next;
        Ok(result)
    }
    fn next_inner(&mut self, v: &MatrixView) -> Result<Option<MatrixAction>, &'static str> {
        self.validate()?;
        v.validate()?;
        self.ready = None;
        if self.completed() {
            return Ok(None);
        }
        if v.screen == S::Unknown {
            self.verify_reads = 0;
            if let Some(p) = &mut self.pending {
                p.stable = None;
                p.reads = 0;
            }
            return Ok(None);
        }
        if !matches!(v.screen, S::Matrix | S::MatrixTeam) {
            return Err("矩阵准备遇到非矩阵页面或未知确认");
        }
        // 只缓存已读到的身份；待执行动作仍绑定 pending.before，不能据此重放。
        self.current = Some(v.clone());
        if let Some(p) = &mut self.pending {
            let effect = match p.action {
                MatrixAction::Next => v.screen == S::MatrixTeam,
                MatrixAction::DismissTutorial => v.screen == S::MatrixTeam && !v.tutorial,
                MatrixAction::AgentsTab | MatrixAction::FocusMain | MatrixAction::FocusSupport => {
                    !v.tutorial && v.pane == MatrixPane::Agents
                }
                MatrixAction::PresetsTab => !v.tutorial && v.pane == MatrixPane::Presets,
                MatrixAction::RemoveMain(i) => {
                    matches!(v.main[i as usize], MatrixSlot::Empty)
                        && p.before
                            .main
                            .iter()
                            .zip(&v.main)
                            .enumerate()
                            .all(|(j, (a, b))| {
                                j == i as usize
                                    || match (a, b) {
                                        (MatrixSlot::Empty, MatrixSlot::Empty) => true,
                                        (
                                            MatrixSlot::Occupied { signature: a, .. },
                                            MatrixSlot::Occupied { signature: b, .. },
                                        ) => a == b,
                                        _ => false,
                                    }
                            })
                }
                MatrixAction::Up | MatrixAction::Down => {
                    v.screen == S::MatrixTeam && !v.tutorial && v.pane == p.before.pane
                }
                MatrixAction::PickAgent(_) => {
                    let n = usize::from(self.chosen) + 1;
                    let expected = match &self.plan.kind {
                        LostVoidTeamKind::MatrixManual { agents } => agents,
                        _ => return Err("矩阵手选阶段错误"),
                    };
                    v.main_agents().is_some_and(|a| a == expected[..n])
                }
                MatrixAction::PickPreset(_) => v
                    .main_agents()
                    .filter(|a| !a.is_empty())
                    .is_some_and(|a| self.plan.verify_roster(&a) == Ok(true)),
                MatrixAction::PickSupport(_) => {
                    v.support_agent() == self.support_wanted.as_deref()
                        && !v.support_rules.is_empty()
                        && v.main_agents() == Some(self.main_agents.clone())
                }
            };
            if !effect {
                p.stable = None;
                p.reads = 0;
                return Ok(None);
            }
            // 主战/协战后态要求成员与属性稳定，列表翻页内容本身可以变化。
            let same = p.stable.as_ref().is_some_and(|old| {
                old.main == v.main
                    && old.support == v.support
                    && old.support_rules == v.support_rules
                    && old.tutorial == v.tutorial
                    && old.pane == v.pane
            });
            p.reads = if same { p.reads + 1 } else { 1 };
            p.stable = Some(v.clone());
            if p.reads < 2 {
                return Ok(None);
            }
            let action = p.action;
            self.pending = None;
            match action {
                MatrixAction::Next | MatrixAction::DismissTutorial => {}
                MatrixAction::AgentsTab => self.stage = Stage::MainFocus,
                MatrixAction::PresetsTab => self.reset(),
                MatrixAction::FocusMain => self.stage = Stage::Clear,
                MatrixAction::RemoveMain(_) => {}
                MatrixAction::Up => {
                    self.reset_left = self.reset_left.saturating_sub(1);
                }
                MatrixAction::Down => self.down += 1,
                MatrixAction::PickAgent(_) => {
                    self.chosen += 1;
                    if self.chosen == 3 {
                        self.main_agents = v.main_agents().ok_or("矩阵主战后态缺失")?;
                        self.stage = Stage::SupportFocus;
                    } else {
                        self.reset();
                    }
                }
                MatrixAction::PickPreset(_) => {
                    self.main_agents = v.main_agents().ok_or("矩阵预备主战后态缺失")?;
                    self.stage = Stage::SupportFocus;
                }
                MatrixAction::FocusSupport => self.stage = Stage::SupportSelect,
                MatrixAction::PickSupport(_) => {
                    self.support_rules = v.support_rules.clone();
                    self.stage = Stage::Verify;
                    self.verify_reads = 0;
                }
            }
        }
        self.current = Some(v.clone());
        if v.tutorial {
            return self.offer(MatrixAction::DismissTutorial);
        }
        if v.screen == S::Matrix {
            if self.stage == Stage::Entry && v.next_enabled {
                return self.offer(MatrixAction::Next);
            }
            return Ok(None);
        }
        if self.stage == Stage::Entry {
            self.stage = Stage::Mode;
        }
        if matches!(
            self.stage,
            Stage::MainFocus | Stage::Clear | Stage::MainSelect
        ) && v.pane != MatrixPane::Agents
            || self.stage == Stage::PresetSelect && v.pane != MatrixPane::Presets
            || self.stage == Stage::Reset
                && v.pane
                    != if self.manual().is_some() {
                        MatrixPane::Agents
                    } else {
                        MatrixPane::Presets
                    }
        {
            return Ok(None);
        }
        for _ in 0..8 {
            let action = match self.stage {
                Stage::Mode => {
                    if self.manual().is_some() {
                        if v.pane == MatrixPane::Agents {
                            self.stage = Stage::MainFocus;
                            continue;
                        } else {
                            MatrixAction::AgentsTab
                        }
                    } else if v.pane == MatrixPane::Presets {
                        self.reset();
                        continue;
                    } else {
                        MatrixAction::PresetsTab
                    }
                }
                Stage::MainFocus => {
                    if v.main.iter().all(|s| *s == MatrixSlot::Empty) {
                        self.stage = Stage::Clear;
                        continue;
                    } else {
                        MatrixAction::FocusMain
                    }
                }
                Stage::Clear => {
                    if v.main.contains(&MatrixSlot::Unknown) {
                        return Ok(None);
                    }
                    if let Some(i) = v
                        .main
                        .iter()
                        .rposition(|s| matches!(s, MatrixSlot::Occupied { .. }))
                    {
                        MatrixAction::RemoveMain(i as u8)
                    } else {
                        self.reset();
                        continue;
                    }
                }
                Stage::Reset => {
                    if self.reset_left > 0 {
                        MatrixAction::Up
                    } else {
                        self.stage = if self.manual().is_some() {
                            Stage::MainSelect
                        } else {
                            Stage::PresetSelect
                        };
                        continue;
                    }
                }
                Stage::MainSelect => {
                    if !self.prefix_matches(v, self.chosen as usize) {
                        return Err("矩阵已选主战成员发生变化");
                    }
                    let expected = &self.manual().ok_or("矩阵手选方案缺失")?[self.chosen as usize];
                    if let Some(c)=v.candidates.iter().find(|c|matches!(&c.item,MatrixCandidateKind::Agent{agent_id:Some(id),..}if id==expected)){MatrixAction::PickAgent(c.slot)}else{if self.down>=4{return Err("矩阵主战代理人未在五页中定位");}MatrixAction::Down}
                }
                Stage::PresetSelect => {
                    let index = match self.plan.kind {
                        LostVoidTeamKind::Predefined { index } => index,
                        _ => return Err("矩阵预备方案缺失"),
                    };
                    if let Some(c)=v.candidates.iter().find(|c|matches!(c.item,MatrixCandidateKind::Preset{team_index:Some(i),..}if i==index)){MatrixAction::PickPreset(c.slot)}else{if self.down>=7{return Err("矩阵预备编队未能唯一定位");}MatrixAction::Down}
                }
                Stage::SupportFocus => {
                    if !self.main_matches(v) {
                        return Err("矩阵协战前主战成员改变");
                    }
                    MatrixAction::FocusSupport
                }
                Stage::SupportSelect => {
                    if !self.main_matches(v) {
                        return Err("矩阵协战选取时主战成员改变");
                    }
                    let Some(c) = Self::support_choice(v) else {
                        return Ok(None);
                    };
                    let MatrixCandidateKind::Agent {
                        agent_id: Some(id), ..
                    } = &c.item
                    else {
                        return Ok(None);
                    };
                    self.support_wanted = Some(id.clone());
                    if v.support_agent() == Some(id.as_str()) {
                        if v.support_rules.is_empty() {
                            return Ok(None);
                        }
                        self.support_rules = v.support_rules.clone();
                        self.stage = Stage::Verify;
                        continue;
                    }
                    MatrixAction::PickSupport(c.slot)
                }
                Stage::Verify => {
                    if !self.main_matches(v)
                        || v.support_agent() != self.support_wanted.as_deref()
                        || v.support_rules != self.support_rules
                    {
                        return Err("矩阵已选成员或属性发生变化");
                    }
                    if !v.start_enabled || self.support_rules.is_empty() {
                        self.verify_reads = 0;
                        return Ok(None);
                    }
                    self.verify_reads += 1;
                    if self.verify_reads >= 2 {
                        self.stage = Stage::Done;
                        self.receipt = Some(LostVoidMatrixReceipt {
                            run_id: self.request.run_id,
                            plan: self.plan.clone(),
                            main_agents: self.main_agents.clone(),
                            support_agent: self.support_wanted.clone().ok_or("矩阵协战身份缺失")?,
                            support_rules: self.support_rules.clone(),
                        });
                    }
                    self.validate()?;
                    return Ok(None);
                }
                _ => return Ok(None),
            };
            return self.offer(action);
        }
        Err("矩阵准备阶段未能推进")
    }
    fn offer(&mut self, a: MatrixAction) -> Result<Option<MatrixAction>, &'static str> {
        if self.actions >= 192 {
            return Err("矩阵准备动作达到上限");
        }
        self.ready = Some(a);
        Ok(self.ready)
    }
    pub fn prepare(&mut self, a: MatrixAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.ready != Some(a) || self.pending.is_some() {
            return Err("矩阵动作尚无可保存意图");
        }
        self.pending = Some(Pending {
            action: a,
            posted: false,
            before: self.current.clone().ok_or("矩阵原页面缺失")?,
            stable: None,
            reads: 0,
        });
        self.ready = None;
        self.actions += 1;
        self.validate()
    }
    pub fn allows(&self, a: MatrixAction, v: &MatrixView) -> bool {
        self.validate().is_ok()
            && self
                .pending
                .as_ref()
                .is_some_and(|p| !p.posted && p.action == a && &p.before == v)
    }
    pub fn posted(&mut self, a: MatrixAction) -> Result<(), &'static str> {
        let p = self
            .pending
            .as_mut()
            .filter(|p| !p.posted && p.action == a)
            .ok_or("矩阵投递回执不匹配")?;
        p.posted = true;
        self.validate()
    }
    pub fn not_posted(&mut self, a: MatrixAction) -> Result<(), &'static str> {
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| !p.posted && p.action == a)
        {
            return Err("矩阵已投递动作不能撤销");
        }
        self.pending = None;
        self.actions = self.actions.saturating_sub(1);
        if matches!(a, MatrixAction::PickSupport(_)) {
            self.support_wanted = None;
        }
        self.validate()
    }
    pub fn partial_scroll(&mut self, a: MatrixAction) -> Result<(), &'static str> {
        if !matches!(a, MatrixAction::Up | MatrixAction::Down) {
            return Err("矩阵半次拖动动作错误");
        }
        self.not_posted(a)
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.ready = None;
        self.receipt = None;
        self.verify_reads = 0;
        if self.stage == Stage::Done {
            self.stage = Stage::Verify;
        }
        if let Some(p) = &mut self.pending {
            p.stable = None;
            p.reads = 0;
        }
        self.validate()
    }
}
