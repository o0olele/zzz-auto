//! 调查战略及周期增益准备，到达出战页后交回；不投递出战或写挑战次数。
use crate::{
    lost_void::{LostVoidChallenge, LostVoidMission as M, strategies},
    lost_void_scene::{LostVoidObservation, LostVoidScreen as S},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyProgress {
    New,
    NotMax,
    Max,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyCandidate {
    pub slot: u8,
    pub name: String,
    pub progress: StrategyProgress,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyPage {
    pub candidates: Vec<StrategyCandidate>,
    pub selected_name: Option<String>,
    pub confirm_enabled: bool,
}
impl StrategyPage {
    pub fn validate(&self) -> Result<(), &'static str> {
        let names = strategies()?;
        if self.candidates.is_empty()
            || self.candidates.len() > 12
            || self.candidates.iter().enumerate().any(|(i, c)| {
                c.slot != i as u8
                    || !names.contains(&c.name)
                    || self.candidates[..i].iter().any(|a| a.name == c.name)
            })
            || self
                .selected_name
                .as_ref()
                .is_some_and(|n| !self.candidates.iter().any(|c| &c.name == n))
        {
            return Err("迷失之地战略目录或槽位无效");
        }
        Ok(())
    }
    fn same_candidates(&self, other: &Self) -> bool {
        self.candidates.len() == other.candidates.len()
            && self
                .candidates
                .iter()
                .zip(&other.candidates)
                .all(|(a, b)| a.slot == b.slot && a.name == b.name)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionSettings {
    pub mission: M,
    pub buff_markers_valid: bool,
    pub selected_buff: Option<u8>,
    pub up_agents: Option<Vec<String>>,
    pub next_enabled: bool,
}
impl MissionSettings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.mission == M::MatrixAction
            || self.mission == M::TaskForce
                && (self.buff_markers_valid || self.selected_buff.is_some())
            || self.selected_buff.is_some_and(|b| !(1..=3).contains(&b))
            || (!self.buff_markers_valid && self.selected_buff.is_some())
        {
            return Err("迷失之地副本或周期增益读数无效");
        }
        if let Some(up) = &self.up_agents {
            let known = crate::random_play::agents();
            if self.mission != M::TaskForce
                || up.len() != 3
                || up
                    .iter()
                    .enumerate()
                    .any(|(i, id)| !known.iter().any(|a| &a.id == id) || up[..i].contains(id))
            {
                return Err("迷失之地本周 UP 名单缺失、重复或不在目录");
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidStrategyView {
    pub scene: LostVoidObservation,
    pub page: Option<StrategyPage>,
    pub settings: Option<MissionSettings>,
}
impl LostVoidStrategyView {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(p) = &self.page {
            p.validate()?;
        }
        if let Some(s) = &self.settings {
            s.validate()?;
        }
        if self.page.is_some() && self.scene.screen != S::Strategy
            || self.settings.as_ref().is_some_and(|s| {
                self.scene.screen
                    != if s.mission == M::Purge {
                        S::Purge
                    } else {
                        S::TaskForce
                    }
            })
            || self.page.is_some() && self.settings.is_some()
        {
            return Err("迷失之地准备页面与区域数据不一致");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidStrategyRequest {
    pub run_id: Uuid,
    pub mission: M,
    pub challenge: LostVoidChallenge,
    pub timeout_ms: u32,
}
impl LostVoidStrategyRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.challenge.validate()?;
        if self.run_id.is_nil()
            || self.mission == M::MatrixAction
            || !(1000..=300_000).contains(&self.timeout_ms)
        {
            return Err("调查战略准备仅适用于战线肃清及特遣调查，身份或预算无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyAction {
    Open,
    ScrollLeft,
    ScrollRight,
    Pick(u8),
    Confirm,
    Buff(u8),
    Next,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategySelectionProof {
    Named,
    OwnPickEnabled,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidStrategyReceipt {
    pub run_id: Uuid,
    pub mission: M,
    pub strategy: String,
    pub selection_proof: StrategySelectionProof,
    pub buff: Option<u8>,
    pub up_agents: Vec<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    Inspect,
    Strategy,
    Return,
    Buff,
    Next,
    Done,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    action: StrategyAction,
    posted: bool,
    before: LostVoidStrategyView,
    reads: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidStrategyFlow {
    request: LostVoidStrategyRequest,
    stage: Stage,
    ready: Option<StrategyAction>,
    pending: Option<Pending>,
    current: Option<LostVoidStrategyView>,
    selected: Option<String>,
    proof: Option<StrategySelectionProof>,
    up_agents: Option<Vec<String>>,
    scrolls: u8,
    returned: bool,
    receipt: Option<LostVoidStrategyReceipt>,
}
impl LostVoidStrategyFlow {
    pub fn new(request: LostVoidStrategyRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            stage: Stage::Inspect,
            ready: None,
            pending: None,
            current: None,
            selected: None,
            proof: None,
            up_agents: None,
            scrolls: 0,
            returned: false,
            receipt: None,
        })
    }
    pub fn request(&self) -> &LostVoidStrategyRequest {
        &self.request
    }
    pub fn up_agents(&self) -> Option<&[String]> {
        self.up_agents.as_deref()
    }
    pub fn completed(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn receipt(&self) -> Option<&LostVoidStrategyReceipt> {
        self.receipt.as_ref()
    }
    fn mission_screen(&self) -> S {
        if self.request.mission == M::Purge {
            S::Purge
        } else {
            S::TaskForce
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.up_agents.is_some() {
            MissionSettings {
                mission: self.request.mission,
                buff_markers_valid: false,
                selected_buff: None,
                up_agents: self.up_agents.clone(),
                next_enabled: false,
            }
            .validate()?;
        }
        if let Some(v) = &self.current {
            v.validate()?;
            if v.settings.as_ref().is_some_and(|s| {
                s.mission != self.request.mission
                    || self
                        .up_agents
                        .as_ref()
                        .is_some_and(|up| s.up_agents.as_ref() != Some(up))
            }) {
                return Err("迷失之地准备存档的副本或 UP 身份不一致");
            }
        }
        if let Some(p) = &self.pending {
            p.before.validate()?;
            if p.before.settings.as_ref().is_some_and(|s| {
                s.mission != self.request.mission
                    || self
                        .up_agents
                        .as_ref()
                        .is_some_and(|up| s.up_agents.as_ref() != Some(up))
            }) {
                return Err("迷失之地准备意图的副本或 UP 身份不一致");
            }
            let valid = match p.action {
                StrategyAction::Open => {
                    self.stage == Stage::Inspect && p.before.scene.screen == self.mission_screen()
                }
                StrategyAction::ScrollLeft | StrategyAction::ScrollRight => {
                    self.stage == Stage::Strategy && p.before.page.is_some()
                }
                StrategyAction::Pick(i) => {
                    self.stage == Stage::Strategy
                        && p.before
                            .page
                            .as_ref()
                            .and_then(|p| p.candidates.get(i as usize))
                            .is_some_and(|c| Some(&c.name) == self.selected.as_ref())
                        && self.proof.is_none()
                }
                StrategyAction::Confirm => {
                    self.stage == Stage::Return
                        && self.proof.is_some()
                        && p.before.page.as_ref().is_some_and(|p| p.confirm_enabled)
                }
                StrategyAction::Buff(i) => {
                    self.stage == Stage::Buff
                        && self.request.mission == M::Purge
                        && i == self.request.challenge.period_buff_no
                        && p.before
                            .settings
                            .as_ref()
                            .is_some_and(|s| s.buff_markers_valid && s.selected_buff != Some(i))
                }
                StrategyAction::Next => {
                    self.stage == Stage::Next
                        && p.before.settings.as_ref().is_some_and(|s| {
                            s.next_enabled
                                && (self.request.mission != M::Purge
                                    || s.buff_markers_valid
                                        && s.selected_buff
                                            == Some(self.request.challenge.period_buff_no))
                        })
                }
            };
            if !valid {
                return Err("迷失之地准备动作与检查点阶段不一致");
            }
        }
        if self.scrolls > 12
            || self.stage == Stage::Return
                && !self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.action == StrategyAction::Confirm)
            || self.request.mission == M::TaskForce
                && self.stage != Stage::Inspect
                && self.up_agents.is_none()
            || (self.ready.is_some() && self.pending.is_some())
            || self
                .selected
                .as_ref()
                .is_some_and(|s| strategies().is_ok_and(|names| !names.contains(s)))
            || self.pending.as_ref().is_some_and(|p| p.reads > 2)
            || self.proof.is_some() && self.selected.is_none()
            || (matches!(
                self.stage,
                Stage::Return | Stage::Buff | Stage::Next | Stage::Done
            ) && (self.selected.is_none() || self.proof.is_none()))
            || self.completed() != self.receipt.is_some()
            || self.receipt.as_ref().is_some_and(|r| {
                r.run_id != self.request.run_id
                    || r.mission != self.request.mission
                    || Some(&r.strategy) != self.selected.as_ref()
                    || Some(r.selection_proof) != self.proof
                    || r.buff
                        != (self.request.mission == M::Purge)
                            .then_some(self.request.challenge.period_buff_no)
                    || r.up_agents != self.up_agents.clone().unwrap_or_default()
                    || !self.returned
            })
        {
            return Err("迷失之地战略准备检查点不一致");
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<(), &'static str> {
        self.receipt = Some(LostVoidStrategyReceipt {
            run_id: self.request.run_id,
            mission: self.request.mission,
            strategy: self.selected.clone().ok_or("迷失之地尚无本次战略选择")?,
            selection_proof: self.proof.ok_or("迷失之地尚无战略确认依据")?,
            buff: (self.request.mission == M::Purge)
                .then_some(self.request.challenge.period_buff_no),
            up_agents: self.up_agents.clone().unwrap_or_default(),
        });
        self.pending = None;
        self.returned = true;
        self.stage = Stage::Done;
        Ok(())
    }
    pub fn next(
        &mut self,
        v: &LostVoidStrategyView,
    ) -> Result<Option<StrategyAction>, &'static str> {
        self.validate()?;
        v.validate()?;
        self.ready = None;
        if self.completed() {
            return Ok(None);
        }
        if v.scene.screen == S::Unknown {
            return Ok(None);
        }
        if v.scene.screen == S::Deploy {
            if self
                .pending
                .as_ref()
                .is_some_and(|p| p.action == StrategyAction::Next)
                || self.returned
            {
                self.finish()?;
            } else {
                return Err("迷失之地已有出战页缺少本次战略准备证明");
            }
            return Ok(None);
        }
        if self.returned {
            return Err("迷失之地准备完成后的返回页面已改变");
        }
        if v.scene.screen == self.mission_screen() {
            let Some(s) = &v.settings else {
                return Ok(None);
            };
            s.validate()?;
            if s.mission != self.request.mission {
                return Err("迷失之地战略返回了其他副本");
            }
            if self.request.mission == M::TaskForce {
                let Some(up) = &s.up_agents else {
                    return Ok(None);
                };
                if self.up_agents.as_ref().is_some_and(|old| old != up) {
                    return Err("迷失之地本周 UP 名单改变");
                }
                self.up_agents = Some(up.clone());
            }
            if self
                .pending
                .as_ref()
                .is_some_and(|p| matches!(p.action, StrategyAction::Open | StrategyAction::Next))
            {
                return Ok(None);
            }
            if self
                .pending
                .as_ref()
                .is_some_and(|p| p.action == StrategyAction::Confirm)
            {
                self.pending = None;
                self.stage = if self.request.mission == M::Purge {
                    Stage::Buff
                } else {
                    Stage::Next
                };
            }
            if let Some(p) = &self.pending
                && let StrategyAction::Buff(b) = p.action
            {
                if !s.buff_markers_valid || s.selected_buff != Some(b) {
                    return Ok(None);
                }
                self.pending = None;
                self.stage = Stage::Next;
            }
            self.current = Some(v.clone());
            let action = match self.stage {
                Stage::Inspect => StrategyAction::Open,
                Stage::Buff if s.buff_markers_valid => {
                    if s.selected_buff == Some(self.request.challenge.period_buff_no) {
                        self.stage = Stage::Next;
                        StrategyAction::Next
                    } else {
                        StrategyAction::Buff(self.request.challenge.period_buff_no)
                    }
                }
                Stage::Next if s.next_enabled => StrategyAction::Next,
                _ => return Ok(None),
            };
            if action == StrategyAction::Next && !s.next_enabled {
                return Ok(None);
            }
            self.ready = Some(action);
            return Ok(Some(action));
        }
        if v.scene.screen != S::Strategy {
            return Err("迷失之地战略准备遇到非预期页面");
        }
        let Some(page) = &v.page else { return Ok(None) };
        page.validate()?;
        if self.stage == Stage::Inspect {
            if self.request.mission == M::TaskForce && self.up_agents.is_none() {
                return Err("特遣战略列表缺少本次 UP 名单");
            }
            self.stage = Stage::Strategy;
        }
        if !matches!(self.stage, Stage::Strategy | Stage::Return) {
            return Err("迷失之地已经确认战略，不能返回旧选择页再次确认");
        }
        if let Some(p) = &mut self.pending {
            match p.action {
                StrategyAction::Open => {
                    self.stage = Stage::Strategy;
                    self.pending = None;
                }
                StrategyAction::ScrollLeft | StrategyAction::ScrollRight => {
                    self.scrolls += 1;
                    self.pending = None;
                }
                StrategyAction::Pick(slot) => {
                    let old = p.before.page.as_ref().ok_or("迷失之地战略点击缺少原页面")?;
                    if !old.same_candidates(page) {
                        return Err("迷失之地战略槽位身份改变");
                    }
                    let target = old
                        .candidates
                        .get(slot as usize)
                        .ok_or("迷失之地战略槽位缺失")?;
                    if !page.confirm_enabled
                        || page
                            .selected_name
                            .as_ref()
                            .is_some_and(|n| n != &target.name)
                    {
                        return Ok(None);
                    }
                    p.reads += 1;
                    if p.reads < 2 {
                        return Ok(None);
                    }
                    self.proof = Some(if page.selected_name.as_ref() == Some(&target.name) {
                        StrategySelectionProof::Named
                    } else {
                        StrategySelectionProof::OwnPickEnabled
                    });
                    self.pending = None;
                }
                StrategyAction::Confirm => return Ok(None),
                _ => return Err("迷失之地战略动作与当前页面不符"),
            }
        }
        if self.proof.is_some() {
            if self
                .current
                .as_ref()
                .and_then(|v| v.page.as_ref())
                .is_some_and(|old| !old.same_candidates(page))
                || page
                    .selected_name
                    .as_ref()
                    .is_some_and(|n| Some(n) != self.selected.as_ref())
            {
                return Err("迷失之地已选择战略的身份发生变化");
            }
            if self.proof == Some(StrategySelectionProof::Named) && page.selected_name.is_none() {
                return Ok(None);
            }
            self.current = Some(v.clone());
            if page.confirm_enabled {
                self.ready = Some(StrategyAction::Confirm);
                return Ok(self.ready);
            }
            return Ok(None);
        }
        self.current = Some(v.clone());
        let candidate = if self.request.challenge.chase_new_mode {
            page.candidates
                .iter()
                .find(|c| c.progress == StrategyProgress::New)
                .or_else(|| {
                    page.candidates
                        .iter()
                        .find(|c| c.progress == StrategyProgress::NotMax)
                })
                .or_else(|| {
                    (self.scrolls >= 3)
                        .then(|| {
                            page.candidates
                                .iter()
                                .find(|c| c.progress != StrategyProgress::Unknown)
                        })
                        .flatten()
                })
        } else {
            page.candidates
                .iter()
                .find(|c| c.name == self.request.challenge.investigation_strategy)
        };
        let action = if let Some(c) = candidate {
            self.selected = Some(c.name.clone());
            StrategyAction::Pick(c.slot)
        } else {
            let limit = if self.request.challenge.chase_new_mode {
                3
            } else {
                12
            };
            if self.scrolls >= limit {
                return Err("迷失之地未找到可确认的目标战略");
            }
            if self.request.challenge.chase_new_mode {
                StrategyAction::ScrollLeft
            } else {
                let names = strategies()?;
                let target = names
                    .iter()
                    .position(|n| n == &self.request.challenge.investigation_strategy)
                    .ok_or("目标战略不在目录")?;
                let visible = page
                    .candidates
                    .iter()
                    .filter_map(|c| names.iter().position(|n| n == &c.name))
                    .min()
                    .ok_or("战略列表没有目录名称")?;
                if target < visible {
                    StrategyAction::ScrollRight
                } else {
                    StrategyAction::ScrollLeft
                }
            }
        };
        self.ready = Some(action);
        Ok(Some(action))
    }
    pub fn prepare(&mut self, a: StrategyAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.ready != Some(a) || self.pending.is_some() {
            return Err("迷失之地没有待保存的战略准备动作");
        }
        self.pending = Some(Pending {
            action: a,
            posted: false,
            before: self.current.clone().ok_or("迷失之地准备页面缺失")?,
            reads: 0,
        });
        if a == StrategyAction::Confirm {
            self.stage = Stage::Return;
        }
        self.ready = None;
        self.validate()
    }
    pub fn allows(&self, a: StrategyAction, v: &LostVoidStrategyView) -> bool {
        self.validate().is_ok()
            && self
                .pending
                .as_ref()
                .is_some_and(|p| !p.posted && p.action == a && &p.before == v)
    }
    pub fn posted(&mut self, a: StrategyAction) -> Result<(), &'static str> {
        let p = self
            .pending
            .as_mut()
            .filter(|p| !p.posted && p.action == a)
            .ok_or("迷失之地战略投递回执不匹配")?;
        p.posted = true;
        self.validate()
    }
    pub fn not_posted(&mut self, a: StrategyAction) -> Result<(), &'static str> {
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| !p.posted && p.action == a)
        {
            return Err("迷失之地已投递战略动作不能撤销");
        }
        self.pending = None;
        if a == StrategyAction::Confirm {
            self.stage = Stage::Strategy;
        }
        self.validate()
    }
    /// 已释放的半次拖动不计为完整翻页。恢复后重新读取本页身份。
    pub fn partial_scroll(&mut self, a: StrategyAction) -> Result<(), &'static str> {
        if !matches!(a, StrategyAction::ScrollLeft | StrategyAction::ScrollRight) {
            return Err("战略准备的半次拖动动作不匹配");
        }
        self.not_posted(a)?;
        self.current = None;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.receipt = None;
        self.ready = None;
        if self.stage == Stage::Done {
            self.stage = Stage::Next;
        }
        Ok(())
    }
}
