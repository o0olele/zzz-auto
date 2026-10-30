//! 恢复菜单的独立意图与后态；没有实际加载和到达时不能重置本层。
use crate::{
    lost_void_loading::LoadingObservation,
    lost_void_recovery::LayerRetryBudget,
    lost_void_roster::{RosterMember, validate_members},
    lost_void_scene::LostVoidScreen as S,
    lost_void_team_plan::LostVoidTeamPlan,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryMode {
    Restart,
    Exit,
}

/// 固定菜单上下文之外不使用；正文须明确询问本次挑战 / 战斗的动作。
pub fn confirmation_purpose(words: &[String]) -> Option<RecoveryMode> {
    if words.is_empty() || words.len() > 24 {
        return None;
    }
    let text: String = words
        .iter()
        .flat_map(|s| s.chars())
        .filter(|c| !c.is_whitespace())
        .collect();
    if text.chars().count() > 512
        || [
            "取消", "购买", "支付", "消耗", "消费", "不要", "不能", "无法",
        ]
        .iter()
        .any(|s| text.contains(s))
        || text.contains("重新开始") && text.contains("退出")
    {
        return None;
    }
    let question = if let Some((_, q)) = text.split_once("是否") {
        q
    } else {
        text.strip_prefix("确定")
            .or_else(|| text.strip_prefix("确认"))?
    };
    let mut question = question;
    for _ in 0..3 {
        if let Some(rest) = ["要", "将", "立即", "现在", "确认", "确定"]
            .iter()
            .find_map(|p| question.strip_prefix(p))
        {
            question = rest;
        } else {
            break;
        }
    }
    let (mode, mut rest) = if let Some(q) = question.strip_prefix("重新开始") {
        (RecoveryMode::Restart, q)
    } else if let Some(q) = question.strip_prefix("退出") {
        (RecoveryMode::Exit, q)
    } else {
        return None;
    };
    if let Some(q) = ["本次", "当前", "本层", "该次"]
        .iter()
        .find_map(|p| rest.strip_prefix(p))
    {
        rest = q;
    }
    rest = rest.strip_prefix('的').unwrap_or(rest);
    (rest.starts_with("挑战") || rest.starts_with("战斗")).then_some(mode)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRequest {
    pub recovery_id: Uuid,
    pub budget: LayerRetryBudget,
    pub plan: LostVoidTeamPlan,
    pub members: Vec<RosterMember>,
}
impl RecoveryRequest {
    pub fn mode(&self) -> Result<RecoveryMode, &'static str> {
        match (self.budget.pending_restart(), self.budget.exit_failure()) {
            (Some(_), None) => Ok(RecoveryMode::Restart),
            (None, Some(_)) => Ok(RecoveryMode::Exit),
            _ => Err("恢复菜单缺少本层待执行的重开或退出"),
        }
    }
    pub fn validate_party(&self, members: &[RosterMember]) -> Result<(), &'static str> {
        validate_members(members, self.members.len() as u8)?;
        if !(0..members.len()).any(|offset| {
            members
                .iter()
                .enumerate()
                .all(|(i, m)| m == &self.members[(i + offset) % self.members.len()])
        }) {
            return Err("恢复后队伍不是原实际完整队伍的轮换");
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.budget.validate()?;
        self.mode()?;
        validate_members(&self.members, self.members.len() as u8)?;
        if self.recovery_id.is_nil()
            || self.budget.uses_identity(self.recovery_id)
            || self.plan.script_name.is_empty()
        {
            return Err("恢复菜单身份或冻结计划无效");
        }
        let mut ids: Vec<_> = self.members.iter().map(|m| m.id.clone()).collect();
        for _ in 0..ids.len() {
            if self.plan.verify_roster(&ids)? {
                return Ok(());
            }
            ids.rotate_left(1);
        }
        Err("恢复队伍与冻结出战计划不符")
    }
    fn command(&self) -> RecoveryAction {
        match self.mode().expect("已验证恢复模式") {
            RecoveryMode::Restart => RecoveryAction::Restart,
            RecoveryMode::Exit => RecoveryAction::Exit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RecoveryPage {
    Unknown,
    World,
    BattleHud,
    Menu,
    Confirm(RecoveryMode),
    Loading,
    Initial(S),
    Result(S),
}
fn initial(screen: S) -> bool {
    matches!(screen, S::Gear | S::Event | S::CommonChoice)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryView {
    pub page: RecoveryPage,
    pub menu_button: bool,
    pub members: Option<Vec<RosterMember>>,
    pub loading: Option<LoadingObservation>,
}
impl RecoveryView {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.menu_button && !matches!(self.page, RecoveryPage::World | RecoveryPage::BattleHud)
            || self.members.is_some() && self.page != RecoveryPage::World
            || self.loading.is_some() != (self.page == RecoveryPage::Loading)
            || matches!(self.page,RecoveryPage::Initial(s) if !initial(s))
            || matches!(self.page,RecoveryPage::Result(s) if !matches!(s,S::BattleResult|S::BattleFail))
        {
            return Err("恢复菜单页面与同帧证据不一致");
        }
        if let Some(m) = &self.members {
            validate_members(m, m.len() as u8)?;
        }
        if let Some(l) = &self.loading {
            l.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    OpenMenu,
    Restart,
    Exit,
    Confirm,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Dispatch {
    Intent,
    Posted,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryOutcome {
    Restarted {
        attempt_id: Uuid,
        screen: S,
        members: Option<Vec<RosterMember>>,
    },
    ObservedResult {
        screen: S,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryReceipt {
    pub run_id: Uuid,
    pub recovery_id: Uuid,
    pub source_attempt_id: Uuid,
    pub outcome: RecoveryOutcome,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryFlow {
    request: RecoveryRequest,
    dispatch: BTreeMap<RecoveryAction, Dispatch>,
    current: Option<RecoveryView>,
    stable: u8,
    world: Option<Vec<RosterMember>>,
    world_reads: u8,
    loading: Option<LoadingObservation>,
    offered: Option<RecoveryAction>,
    receipt: Option<RecoveryReceipt>,
}
impl RecoveryFlow {
    pub fn new(request: RecoveryRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            dispatch: BTreeMap::new(),
            current: None,
            stable: 0,
            world: None,
            world_reads: 0,
            loading: None,
            offered: None,
            receipt: None,
        })
    }
    pub fn request(&self) -> &RecoveryRequest {
        &self.request
    }
    pub fn receipt(&self) -> Option<&RecoveryReceipt> {
        self.receipt.as_ref()
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn loading_seen(&self) -> bool {
        self.loading.is_some()
    }
    fn fresh_action(&self, v: &RecoveryView) -> Option<RecoveryAction> {
        if self.completed() {
            return None;
        }
        let command = self.request.command();
        if self.dispatch.is_empty() && v.menu_button {
            Some(RecoveryAction::OpenMenu)
        } else if v.page == RecoveryPage::Menu && !self.dispatch.contains_key(&command) {
            Some(command)
        } else if v.page == RecoveryPage::Confirm(self.request.mode().ok()?)
            && self.dispatch.contains_key(&command)
            && !self.dispatch.contains_key(&RecoveryAction::Confirm)
        {
            Some(RecoveryAction::Confirm)
        } else {
            None
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        let command = self.request.command();
        if self.stable > 2
            || self.world_reads > 2
            || self.current.is_some() != (self.stable > 0)
            || self.dispatch.len() > 3
            || self
                .dispatch
                .keys()
                .any(|a| ![RecoveryAction::OpenMenu, command, RecoveryAction::Confirm].contains(a))
            || self.dispatch.contains_key(&RecoveryAction::Confirm)
                && !self.dispatch.contains_key(&command)
            || self.loading.is_some() && !self.dispatch.contains_key(&RecoveryAction::Confirm)
            || self.world.is_some() != (self.world_reads > 0)
            || self.world_reads > 0 && self.loading.is_none()
            || self.world_reads > 0
                && (self.request.mode()? != RecoveryMode::Restart
                    || self
                        .current
                        .as_ref()
                        .is_none_or(|v| v.page != RecoveryPage::World || v.members != self.world))
        {
            return Err("恢复菜单意图、加载或新帧计数无效");
        }
        if let Some(l) = &self.loading {
            l.validate()?;
        }
        if let Some(v) = &self.current {
            v.validate()?;
            if let Some(m) = &v.members {
                self.request.validate_party(m)?;
            }
        }
        if let Some(m) = &self.world {
            self.request.validate_party(m)?;
        }
        if self.offered.is_some_and(|a| {
            self.stable != 2
                || self
                    .current
                    .as_ref()
                    .is_none_or(|v| self.fresh_action(v) != Some(a))
        }) {
            return Err("恢复菜单缺少新帧输入许可");
        }
        if let Some(r) = &self.receipt {
            if r.run_id != self.request.budget.run_id()
                || r.recovery_id != self.request.recovery_id
                || r.source_attempt_id != self.request.budget.current_attempt_id()
            {
                return Err("恢复菜单回执身份不符");
            }
            let valid = match &r.outcome {
                RecoveryOutcome::ObservedResult { screen } => {
                    self.stable == 2
                        && self
                            .current
                            .as_ref()
                            .is_some_and(|v| v.page == RecoveryPage::Result(*screen))
                }
                RecoveryOutcome::Restarted {
                    attempt_id,
                    screen,
                    members,
                } => {
                    self.request
                        .budget
                        .pending_restart()
                        .is_some_and(|i| i.next_attempt_id == *attempt_id)
                        && self.dispatch.contains_key(&RecoveryAction::Confirm)
                        && self.loading.is_some()
                        && if *screen == S::World {
                            self.world_reads == 2 && self.world.as_ref() == members.as_ref()
                        } else {
                            initial(*screen)
                                && members.is_none()
                                && self.stable == 2
                                && self
                                    .current
                                    .as_ref()
                                    .is_some_and(|v| v.page == RecoveryPage::Initial(*screen))
                        }
                }
            };
            if !valid {
                return Err("恢复菜单回执缺少实际加载及返回后态");
            }
        }
        Ok(())
    }
    fn clear_fresh(&mut self) {
        self.current = None;
        self.stable = 0;
        self.world = None;
        self.world_reads = 0;
        self.offered = None;
    }
    pub fn invalidate_observation(&mut self) {
        self.clear_fresh();
        self.receipt = None;
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.clear_fresh();
        self.receipt = None;
        self.validate()
    }
    fn finish(&mut self, outcome: RecoveryOutcome) {
        self.offered = None;
        self.receipt = Some(RecoveryReceipt {
            run_id: self.request.budget.run_id(),
            recovery_id: self.request.recovery_id,
            source_attempt_id: self.request.budget.current_attempt_id(),
            outcome,
        });
    }
    pub fn next(&mut self, v: &RecoveryView) -> Result<Option<RecoveryAction>, &'static str> {
        self.validate()?;
        v.validate()?;
        if let Some(m) = &v.members {
            self.request.validate_party(m)?;
        }
        if self.completed() {
            return Ok(None);
        }
        let mut next = self.clone();
        next.offered = None;
        next.stable = if next.current.as_ref().is_some_and(|old| old.page == v.page) {
            (next.stable + 1).min(2)
        } else {
            1
        };
        next.current = Some(v.clone());
        if next.dispatch.contains_key(&RecoveryAction::Confirm)
            && let Some(l) = &v.loading
        {
            next.loading = Some(l.clone());
        }
        next.world_reads = if next.loading.is_some()
            && next.request.mode()? == RecoveryMode::Restart
            && v.page == RecoveryPage::World
        {
            if v.members.is_some() {
                if next.world == v.members {
                    (next.world_reads + 1).min(2)
                } else {
                    1
                }
            } else {
                0
            }
        } else {
            0
        };
        next.world = if next.world_reads > 0 {
            v.members.clone()
        } else {
            None
        };
        if let RecoveryPage::Result(screen) = v.page
            && next.stable == 2
        {
            next.finish(RecoveryOutcome::ObservedResult { screen });
        } else if let Some(i) = next.request.budget.pending_restart() {
            let attempt_id = i.next_attempt_id;
            if next.loading.is_some()
                && next.dispatch.contains_key(&RecoveryAction::Confirm)
                && (next.world_reads == 2
                    || matches!(v.page, RecoveryPage::Initial(_)) && next.stable == 2)
            {
                let (screen, members) = match v.page {
                    RecoveryPage::World => (S::World, next.world.clone()),
                    RecoveryPage::Initial(s) => (s, None),
                    _ => return Err("恢复返回页面不一致"),
                };
                next.finish(RecoveryOutcome::Restarted {
                    attempt_id,
                    screen,
                    members,
                });
            }
        }
        if !next.completed() && next.stable == 2 {
            next.offered = next.fresh_action(v);
        }
        next.validate()?;
        let offered = next.offered;
        *self = next;
        Ok(offered)
    }
    pub fn prepare(&mut self, a: RecoveryAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.offered != Some(a) || self.dispatch.contains_key(&a) {
            return Err("恢复菜单没有对应的新帧意图");
        }
        self.offered = None;
        self.dispatch.insert(a, Dispatch::Intent);
        self.validate()
    }
    pub fn allows(&self, a: RecoveryAction, v: &RecoveryView) -> bool {
        if self.validate().is_err()
            || v.validate().is_err()
            || self.completed()
            || self.dispatch.get(&a) != Some(&Dispatch::Intent)
        {
            return false;
        }
        if v.members
            .as_ref()
            .is_some_and(|m| self.request.validate_party(m).is_err())
        {
            return false;
        }
        match a {
            RecoveryAction::OpenMenu => v.menu_button && self.dispatch.len() == 1,
            RecoveryAction::Restart | RecoveryAction::Exit => {
                a == self.request.command()
                    && v.page == RecoveryPage::Menu
                    && !self.dispatch.contains_key(&RecoveryAction::Confirm)
            }
            RecoveryAction::Confirm => {
                v.page == RecoveryPage::Confirm(self.request.mode().expect("已验证模式"))
            }
        }
    }
    pub fn posted(&mut self, a: RecoveryAction) -> Result<(), &'static str> {
        self.validate()?;
        let dispatch = self.dispatch.get_mut(&a).ok_or("恢复菜单没有待投递意图")?;
        if *dispatch == Dispatch::Unknown {
            return Err("未知恢复输入不能凭保存动作改为已投递");
        }
        *dispatch = Dispatch::Posted;
        self.validate()
    }
    pub fn unknown(&mut self, a: RecoveryAction) -> Result<(), &'static str> {
        self.validate()?;
        let dispatch = self.dispatch.get_mut(&a).ok_or("恢复菜单没有待投递意图")?;
        if *dispatch == Dispatch::Posted {
            return Err("已知恢复输入不能退回未知状态");
        }
        *dispatch = Dispatch::Unknown;
        self.validate()
    }
    pub fn not_posted(&mut self, a: RecoveryAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.dispatch.get(&a) != Some(&Dispatch::Intent) {
            return Err("已投递或未知恢复动作不能撤销");
        }
        let mut next = self.clone();
        next.dispatch.remove(&a);
        next.clear_fresh();
        next.validate()?;
        *self = next;
        Ok(())
    }
}
