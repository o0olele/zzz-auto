//! 实际 HUD 队伍、移动前台与动态规则。人数由父流程的出战页证明提供。
use crate::{
    InputMode, InputProbeAction, InputProbeConfig,
    lost_void::{ArtifactSelection, LostVoidChallenge, artifacts, choose_artifacts},
    lost_void_scene::LostVoidScreen as S,
    lost_void_team_plan::{LostVoidTeamKind, LostVoidTeamPlan},
    predefined_teams::UNKNOWN_AGENT,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

pub const AGENT_KINDS: [&str; 7] = ["强攻", "击破", "支援", "防护", "异常", "命破", "锋御"];
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterMember {
    pub id: String,
    pub kind: String,
}
impl RosterMember {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !crate::random_play::agents().iter().any(|a| a.id == self.id)
            || !AGENT_KINDS.contains(&self.kind.as_str())
        {
            return Err("迷失之地实际成员身份或类型无效");
        }
        Ok(())
    }
}
pub fn validate_members(members: &[RosterMember], count: u8) -> Result<(), &'static str> {
    if !(1..=3).contains(&count) || members.len() != usize::from(count) {
        return Err("实际队伍尚未完整识别");
    }
    let mut ids = BTreeSet::new();
    for member in members {
        member.validate()?;
        if !ids.insert(&member.id) {
            return Err("实际队伍代理人重复");
        }
    }
    Ok(())
}
pub fn best_for_moving(members: &[RosterMember]) -> Result<&str, &'static str> {
    validate_members(members, members.len() as u8)?;
    let rank = |m: &RosterMember| match m.id.as_str() {
        "astra_yao" => 5,
        "anby" | "nekomata" | "corin" | "koleda" | "soukaku" | "lucy" | "qingyi" | "piper"
        | "ju_fufu" | "dialyn" => 0,
        "hoshimi_miyabi" | "yixuan" | "billy" | "ben" | "panyinhu" | "zhao" => 4,
        _ => match m.kind.as_str() {
            "支援" => 1,
            "防护" => 2,
            _ => 3,
        },
    };
    Ok(&members
        .iter()
        .enumerate()
        .min_by_key(|(i, m)| (rank(m), *i))
        .unwrap()
        .1
        .id)
}
fn category_key(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && !['·', ':', '：', '[', ']', '【', '】'].contains(c))
        .collect()
}
fn kind_key(s: &str) -> String {
    let normalized = category_key(s);
    if matches!(normalized.as_str(), "异常" | "击破") {
        "异常击破".into()
    } else {
        normalized
    }
}
pub fn dynamic_options(
    challenge: &LostVoidChallenge,
    initial: &ArtifactSelection,
    members: &[RosterMember],
) -> Result<ArtifactSelection, &'static str> {
    validate_members(members, members.len() as u8)?;
    choose_artifacts(challenge, &[], 0, initial)?;
    let catalog = artifacts()?;
    let category_for = |kind: &str| {
        catalog
            .iter()
            .find(|a| category_key(&a.category) == kind_key(kind))
            .map(|a| a.category.clone())
    };
    let mut present = Vec::<String>::new();
    for m in members {
        if let Some(category) = category_for(&m.kind)
            && !present.contains(&category)
        {
            present.push(category);
        }
    }
    let key = |rule: &str| kind_key(rule);
    let mut out = initial.clone();
    for category in &present {
        if !out
            .dynamic_priority
            .iter()
            .any(|r| !r.contains(' ') && key(r) == category_key(category))
        {
            out.dynamic_priority.push(category.clone());
        }
        out.abandon.retain(|r| key(r) != category_key(category));
    }
    let protected: BTreeSet<_> = challenge
        .artifact_priority
        .iter()
        .chain(&challenge.artifact_priority_2)
        .chain(&initial.battle_priority)
        .filter_map(|r| r.split_once(' ').map(|(c, _)| key(c)))
        .collect();
    // 层循环会复用上一轮自动生成的放弃组；新武备规则也须撤销旧分类放弃。
    out.abandon.retain(|rule| !protected.contains(&key(rule)));
    for kind in AGENT_KINDS {
        if let Some(category) = category_for(kind)
            && !present.contains(&category)
            && !protected.contains(&category_key(&category))
            && !out
                .abandon
                .iter()
                .any(|r| key(r) == category_key(&category))
        {
            out.abandon.push(category);
        }
    }
    choose_artifacts(challenge, &[], 0, &out)?;
    Ok(out)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RosterAction {
    Next,
    Previous,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterRequest {
    pub run_id: Uuid,
    pub level_id: Uuid,
    pub plan: LostVoidTeamPlan,
    pub expected_count: u8,
    pub challenge: LostVoidChallenge,
    pub options: ArtifactSelection,
    pub next_inputs: Vec<InputProbeAction>,
    pub previous_inputs: Vec<InputProbeAction>,
    pub switch_for_moving: bool,
    pub timeout_ms: u32,
}
impl RosterRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.challenge.validate()?;
        choose_artifacts(&self.challenge, &[], 0, &self.options)?;
        let p = &self.plan;
        let known: BTreeSet<_> = crate::random_play::agents()
            .into_iter()
            .map(|a| a.id)
            .collect();
        let mut seen = BTreeSet::new();
        if self.run_id.is_nil()
            || self.level_id.is_nil()
            || !(1..=3).contains(&self.expected_count)
            || !(1000..=600_000).contains(&self.timeout_ms)
            || p.script_name.is_empty()
            || p.script_name.len() > 240
            || matches!(p.script_name.as_str(), "." | "..")
            || p.script_name.contains(['/', '\\', ':'])
            || p.script_name.chars().any(char::is_control)
            || p.expected_agents
                .iter()
                .any(|id| id != UNKNOWN_AGENT && (!known.contains(id) || !seen.insert(id)))
            || !matches!(p.up_agents.len(), 0 | 3)
            || p.up_agents.iter().any(|id| !known.contains(id))
            || p.up_agents.iter().collect::<BTreeSet<_>>().len() != p.up_agents.len()
            || p.chose_by_up && p.up_agents.len() != 3
            || match &p.kind {
                LostVoidTeamKind::KeepCurrent => {
                    p.expected_agents.iter().any(|id| id != UNKNOWN_AGENT)
                }
                LostVoidTeamKind::Predefined { index } => *index >= 20,
                LostVoidTeamKind::MatrixManual { agents } => {
                    agents != &p.expected_agents
                        || self.expected_count != 3
                        || agents.iter().any(|id| id == UNKNOWN_AGENT)
                }
            }
        {
            return Err("迷失之地队伍请求与冻结选队不符");
        }
        validate_switch_inputs(&self.next_inputs, &self.previous_inputs)?;
        Ok(())
    }
    pub fn inputs(&self, action: RosterAction) -> &[InputProbeAction] {
        match action {
            RosterAction::Next => &self.next_inputs,
            RosterAction::Previous => &self.previous_inputs,
        }
    }
    fn canonical(&self, members: &[RosterMember]) -> Result<Vec<RosterMember>, &'static str> {
        validate_members(members, self.expected_count)?;
        let mut rotated = members.to_vec();
        for _ in 0..members.len() {
            if self
                .plan
                .verify_roster(&rotated.iter().map(|m| m.id.clone()).collect::<Vec<_>>())?
            {
                return Ok(rotated);
            }
            rotated.rotate_left(1);
        }
        Err("HUD 实际成员与冻结出战计划不符")
    }
}
/// 与实际人数 / 名单无关，可在首次入口输入前预检；不生成队伍证明。
pub fn validate_switch_inputs(
    next_inputs: &[InputProbeAction],
    previous_inputs: &[InputProbeAction],
) -> Result<(), &'static str> {
    for inputs in [next_inputs, previous_inputs] {
        if inputs.is_empty()
            || inputs.len() > 8
            || inputs
                .iter()
                .enumerate()
                .any(|(i, a)| inputs[..i].contains(a))
        {
            return Err("迷失之地切人绑定无效");
        }
        for input in inputs {
            if !matches!(
                input,
                InputProbeAction::Key { .. } | InputProbeAction::MouseButton { .. }
            ) || matches!(input,InputProbeAction::Key{key} if [87,65,83,68,70,18,27].contains(key))
            {
                return Err("切人绑定与导航或页面控制冲突");
            }
            InputProbeConfig {
                mode: InputMode::Foreground,
                action: *input,
                hold_ms: 100,
                start_delay_ms: 0,
            }
            .validate()?;
        }
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterView {
    pub screen: S,
    pub members: Option<Vec<RosterMember>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterReceipt {
    pub run_id: Uuid,
    pub level_id: Uuid,
    pub plan: LostVoidTeamPlan,
    pub members: Vec<RosterMember>,
    pub front: String,
    pub options: ArtifactSelection,
    pub switch_observed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    action: RosterAction,
    before: Vec<RosterMember>,
    posted: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterFlow {
    request: RosterRequest,
    baseline: Option<Vec<RosterMember>>,
    current: Option<RosterView>,
    last: Option<Vec<RosterMember>>,
    stable: u8,
    pending: Option<Pending>,
    switch_observed: bool,
    attempts: u8,
    receipt: Option<RosterReceipt>,
}
fn rotation_equal(a: &[RosterMember], b: &[RosterMember]) -> bool {
    a.len() == b.len()
        && (0..a.len()).any(|offset| {
            a.iter()
                .enumerate()
                .all(|(i, m)| Some(m) == b.get((i + offset) % b.len()))
        })
}
fn after(before: &[RosterMember], action: RosterAction) -> Vec<RosterMember> {
    let mut v = before.to_vec();
    match action {
        RosterAction::Next => v.rotate_left(1),
        RosterAction::Previous => v.rotate_right(1),
    };
    v
}
impl RosterFlow {
    pub fn new(request: RosterRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            baseline: None,
            current: None,
            last: None,
            stable: 0,
            pending: None,
            switch_observed: false,
            attempts: 0,
            receipt: None,
        })
    }
    pub fn request(&self) -> &RosterRequest {
        &self.request
    }
    pub fn receipt(&self) -> Option<&RosterReceipt> {
        self.receipt.as_ref()
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.stable > 2 || self.attempts > 4 || (self.last.is_none() && self.stable > 0) {
            return Err("实际队伍检查点无效");
        }
        if self.stable > 0
            && self
                .current
                .as_ref()
                .is_none_or(|v| v.screen != S::World || v.members != self.last)
        {
            return Err("实际队伍稳定读数缺少同帧成员");
        }
        if self.switch_observed && self.attempts == 0 {
            return Err("实际队伍切换缺少输入意图");
        }
        for members in self.baseline.iter().chain(self.last.iter()) {
            validate_members(members, self.request.expected_count)?;
            self.request.canonical(members)?;
        }
        if let Some(view) = &self.current
            && let Some(members) = &view.members
        {
            validate_members(members, self.request.expected_count)?;
        }
        if let Some(p) = &self.pending {
            validate_members(&p.before, self.request.expected_count)?;
            if self.suggested(&p.before)? != Some(p.action)
                || self.attempts == 0
                || self
                    .baseline
                    .as_ref()
                    .is_none_or(|b| !rotation_equal(b, &p.before))
            {
                return Err("切人意图缺少本次完整队伍");
            }
        }
        if let Some(r) = &self.receipt
            && (r.run_id != self.request.run_id
                || r.level_id != self.request.level_id
                || r.plan != self.request.plan
                || self.baseline.as_ref() != Some(&r.members)
                || self.stable != 2
                || self.pending.is_some()
                || self.current.as_ref().is_none_or(|v| {
                    v.screen != S::World || v.members.as_ref().is_none_or(|m| m[0].id != r.front)
                })
                || r.options
                    != dynamic_options(&self.request.challenge, &self.request.options, &r.members)?
                || r.switch_observed != self.switch_observed
                || self.request.switch_for_moving
                    && self
                        .last
                        .as_ref()
                        .is_none_or(|m| best_for_moving(m) != Ok(r.front.as_str())))
        {
            return Err("实际队伍回执不符合本次新帧证明");
        }
        Ok(())
    }
    pub fn invalidate_observation(&mut self) {
        self.current = None;
        self.last = None;
        self.stable = 0;
        self.receipt = None;
    }
    fn suggested(&self, members: &[RosterMember]) -> Result<Option<RosterAction>, &'static str> {
        if !self.request.switch_for_moving || best_for_moving(members)? == members[0].id {
            return Ok(None);
        }
        let i = members
            .iter()
            .position(|m| Ok(m.id.as_str()) == best_for_moving(members))
            .unwrap();
        Ok(Some(if i == 2 {
            RosterAction::Previous
        } else {
            RosterAction::Next
        }))
    }
    pub fn next(&mut self, view: &RosterView) -> Result<Option<RosterAction>, &'static str> {
        self.validate()?;
        if view.screen != S::World || view.members.is_none() {
            self.invalidate_observation();
            return Ok(None);
        }
        let members = view.members.as_ref().unwrap();
        let canonical = self.request.canonical(members)?;
        if self
            .baseline
            .as_ref()
            .is_some_and(|b| !rotation_equal(b, members))
        {
            return Err("实际队伍在层内发生变化");
        }
        self.current = Some(view.clone());
        self.receipt = None;
        if self.last.as_ref() == Some(members) {
            self.stable = (self.stable + 1).min(2);
        } else {
            self.last = Some(members.clone());
            self.stable = 1;
        }
        if let Some(p) = &self.pending {
            let expected = after(&p.before, p.action);
            if members != &expected {
                self.last = None;
                self.stable = 0;
                return Ok(None);
            }
            if self.stable < 2 {
                return Ok(None);
            }
            self.pending = None;
            self.switch_observed = true;
        }
        if self.stable < 2 {
            return Ok(None);
        }
        if self.baseline.is_none() {
            self.baseline = Some(canonical);
        }
        if let Some(action) = self.suggested(members)? {
            if self.attempts >= 4 {
                return Err("移动代理人切换达到上限");
            }
            return Ok(Some(action));
        }
        let baseline = self.baseline.as_ref().unwrap();
        self.receipt = Some(RosterReceipt {
            run_id: self.request.run_id,
            level_id: self.request.level_id,
            plan: self.request.plan.clone(),
            members: baseline.clone(),
            front: members[0].id.clone(),
            options: dynamic_options(&self.request.challenge, &self.request.options, baseline)?,
            switch_observed: self.switch_observed,
        });
        self.validate()?;
        Ok(None)
    }
    pub fn prepare(&mut self, action: RosterAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some() || self.stable != 2 || self.attempts >= 4 || self.completed() {
            return Err("切人尚未取得本次稳定队伍");
        }
        let view = self.current.as_ref().ok_or("切人缺少世界画面")?;
        let members = view.members.as_ref().ok_or("切人缺少完整队伍")?;
        if view.screen != S::World || self.suggested(members)? != Some(action) {
            return Err("切人动作与实际队伍不符");
        }
        self.pending = Some(Pending {
            action,
            before: members.clone(),
            posted: false,
        });
        self.attempts += 1;
        Ok(())
    }
    pub fn allows(&self, action: RosterAction, view: &RosterView) -> bool {
        self.validate().is_ok()
            && view.screen == S::World
            && self.pending.as_ref().is_some_and(|p| {
                !p.posted
                    && p.action == action
                    && view.members.as_ref() == Some(&p.before)
                    && self.suggested(&p.before) == Ok(Some(action))
            })
    }
    pub fn posted(&mut self, action: RosterAction) -> Result<(), &'static str> {
        self.validate()?;
        let p = self.pending.as_mut().ok_or("切人缺少输入意图")?;
        if p.action != action || p.posted {
            return Err("切人投递状态不符");
        }
        p.posted = true;
        self.invalidate_observation();
        Ok(())
    }
    pub fn not_posted(&mut self, action: RosterAction) -> Result<(), &'static str> {
        self.validate()?;
        if self
            .pending
            .as_ref()
            .is_none_or(|p| p.action != action || p.posted)
        {
            return Err("切人未投递状态不符");
        }
        self.pending = None;
        self.invalidate_observation();
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        // 未知投递仍保留，只有新帧完整轮换才能确认，不重复切人。
        if let Some(p) = self.pending.as_mut() {
            p.posted = true;
        }
        self.invalidate_observation();
        Ok(())
    }
}
