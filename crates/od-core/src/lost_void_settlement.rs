//! 本次结果页的确定、完成与失败退出；返回入口本身不证明通关。
use crate::{
    lost_void::{ArtifactSelection, LostVoidChallenge, LostVoidMission, choose_artifacts},
    lost_void_battle::BattleOutcome,
    lost_void_event::LostVoidEventReceipt,
    lost_void_level::{LevelOutcome, LostVoidLevelReceipt},
    lost_void_roster::{RosterMember, validate_members},
    lost_void_scene::{BountyProgress, LostVoidScreen as S},
    lost_void_team_plan::LostVoidTeamPlan,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub const REWARD_PANEL: [i32; 4] = [1300, 310, 550, 530];
pub const REWARD_GRID: [i32; 4] = [1312, 486, 522, 348];
pub const REWARD_MIN_SAMPLES: u8 = 3;
pub const REWARD_MIN_SPAN_MS: u64 = 1000;
pub const REWARD_SCAN_MS: u64 = 5000;
pub fn settlement_source() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../resources/lost-void/settlement-source.json"
    ))
    .expect("固定结算布局来源")
}
pub fn reward_sources() -> [(&'static str, &'static [u8], &'static [u8]); 2] {
    [
        (
            "challenge_reward_eval",
            include_bytes!("../../../resources/lost-void/scene/challenge_reward_eval-raw.png"),
            include_bytes!("../../../resources/lost-void/scene/challenge_reward_eval-mask.png"),
        ),
        (
            "critical_reward_money",
            include_bytes!("../../../resources/lost-void/scene/critical_reward_money-raw.png"),
            include_bytes!("../../../resources/lost-void/scene/critical_reward_money-mask.png"),
        ),
    ]
}
pub fn quantity_rect(slot: usize) -> Option<[i32; 4]> {
    if slot >= 8 {
        return None;
    }
    let x = REWARD_GRID[0] + REWARD_GRID[2] * (slot % 4) as i32 / 4;
    let next = REWARD_GRID[0] + REWARD_GRID[2] * ((slot % 4) + 1) as i32 / 4;
    Some([
        x + 4,
        REWARD_GRID[1] + 174 * (slot / 4) as i32 + 135,
        next - x - 8,
        39,
    ])
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RewardSlot {
    Empty,
    Quantity(u32),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewardRead {
    pub evaluation: bool,
    pub money: bool,
    pub slots: Option<[RewardSlot; 8]>,
}
impl RewardRead {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.slots.as_ref().is_some_and(|s| {
            s.iter()
                .any(|v| matches!(v,RewardSlot::Quantity(n) if *n>999_999_999))
        }) {
            return Err("奖励格数量超限");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewardEvidence {
    pub evaluation_seen: bool,
    pub money_seen: bool,
    pub samples: u8,
    pub span_ms: u64,
    pub adequate: bool,
    pub grid: Option<[RewardSlot; 8]>,
}
impl RewardEvidence {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.samples > 64
            || self.span_ms > 300_000
            || self.adequate
                && (self.samples < REWARD_MIN_SAMPLES
                    || self.span_ms < REWARD_MIN_SPAN_MS
                    || self.grid.is_none())
            || self.grid.is_some() != self.adequate
        {
            return Err("奖励采样证据无效");
        }
        RewardRead {
            evaluation: self.evaluation_seen,
            money: self.money_seen,
            slots: self.grid,
        }
        .validate()
    }
    pub fn caps(&self) -> RewardCaps {
        RewardCaps {
            evaluation: if self.evaluation_seen {
                Some(false)
            } else {
                self.adequate.then_some(true)
            },
            period: if self.evaluation_seen || self.money_seen {
                Some(false)
            } else {
                self.adequate.then_some(true)
            },
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewardCaps {
    pub evaluation: Option<bool>,
    pub period: Option<bool>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementRequest {
    pub source: LostVoidLevelReceipt,
    pub settlement_id: Uuid,
    pub next_level_id: Uuid,
    pub mission: LostVoidMission,
    pub challenge: LostVoidChallenge,
    pub plan: LostVoidTeamPlan,
    pub expected_count: u8,
    pub members: Option<Vec<RosterMember>>,
    pub timeout_ms: u32,
}
impl SettlementRequest {
    pub fn defeated(&self) -> bool {
        matches!(
            self.source.outcome,
            LevelOutcome::AgentDead { .. }
                | LevelOutcome::Aborted { .. }
                | LevelOutcome::ObservedResult {
                    screen: S::BattleFail,
                    ..
                }
        )
    }
    pub fn victory_candidate(&self) -> bool {
        self.source.region == "战斗-终结之役"
            && matches!(&self.source.outcome,LevelOutcome::ObservedResult{screen:S::BattleResult,battle:Some(b)} if b.combat_hud_observed&&b.outcome==BattleOutcome::ObservedPage{screen:S::BattleResult})
    }
    pub fn validate_party(&self, members: &[RosterMember]) -> Result<(), &'static str> {
        validate_members(members, self.expected_count)?;
        let mut ids: Vec<_> = members.iter().map(|m| m.id.clone()).collect();
        let mut matches_plan = false;
        for _ in 0..ids.len() {
            matches_plan |= self.plan.verify_roster(&ids)?;
            ids.rotate_left(1);
        }
        let matches_observed = self.members.as_ref().is_none_or(|expected| {
            expected.len() == members.len()
                && (0..members.len()).any(|offset| {
                    members
                        .iter()
                        .enumerate()
                        .all(|(i, m)| m == &expected[(i + offset) % expected.len()])
                })
        });
        if !matches_plan || !matches_observed {
            return Err("结算后完整队伍与当前运行不符");
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.source.validate()?;
        self.challenge.validate()?;
        choose_artifacts(&self.challenge, &[], 0, &self.source.options)?;
        if matches!(self.source.outcome, LevelOutcome::NextLevel { .. })
            || self.settlement_id.is_nil()
            || self.next_level_id.is_nil()
            || self.next_level_id == self.source.level_id
            || !(1..=3).contains(&self.expected_count)
            || !(1000..=300_000).contains(&self.timeout_ms)
            || self.plan.script_name.is_empty()
        {
            return Err("结算缺少本次层结果、计划或预算");
        }
        if let Some(m) = &self.members {
            self.validate_party(m)?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementPage {
    Unknown,
    Result { confirm: bool, finish: bool },
    Failure,
    World,
    Entry(S),
    Content(S),
    Modal,
}
pub fn content(screen: S) -> bool {
    matches!(
        screen,
        S::Event
            | S::CommonChoice
            | S::Gear
            | S::GearConfirm
            | S::Store
            | S::Lottery
            | S::RouteChange
    )
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementView {
    pub page: SettlementPage,
    pub reward: Option<RewardRead>,
    pub members: Option<Vec<RosterMember>>,
    pub bounty: Option<BountyProgress>,
}
impl SettlementView {
    pub fn validate(&self) -> Result<(), &'static str> {
        if matches!(
            self.page,
            SettlementPage::Result {
                confirm: true,
                finish: true
            }
        ) || matches!(self.page,SettlementPage::Entry(s) if !matches!(s,S::EntryStandard|S::EntryPeriodic))
            || matches!(self.page,SettlementPage::Content(s) if !content(s))
        {
            return Err("结算页面或按钮冲突");
        }
        if let Some(r) = &self.reward {
            r.validate()?;
            if self.page
                != (SettlementPage::Result {
                    confirm: false,
                    finish: true,
                })
            {
                return Err("奖励采样不是最终完成页");
            }
        }
        if let Some(m) = &self.members {
            validate_members(m, m.len() as u8)?;
            if self.page != SettlementPage::World {
                return Err("结算队伍不是世界新帧");
            }
        }
        if self
            .bounty
            .is_some_and(|b| b.total != 8000 || b.current > b.total)
            || self.bounty.is_some() && !matches!(self.page, SettlementPage::Entry(_))
        {
            return Err("结算入口进度无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementAction {
    Confirm,
    Finish,
    Retreat,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementContinuation {
    pub level_id: Uuid,
    pub region: String,
    pub members: Vec<RosterMember>,
    pub front: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum CompletionKind {
    Completed,
    Defeated,
    Aborted,
    Unattributed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SettlementOutcome {
    Continue {
        arrival: SettlementContinuation,
    },
    Finished {
        completion: CompletionKind,
        rewards: RewardEvidence,
        caps: RewardCaps,
        entry: S,
        bounty: Option<BountyProgress>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementReceipt {
    pub run_id: Uuid,
    pub level_id: Uuid,
    pub settlement_id: Uuid,
    pub options: ArtifactSelection,
    pub actions: Vec<SettlementAction>,
    pub outcome: SettlementOutcome,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    action: SettlementAction,
    posted: bool,
    rewards: Option<RewardEvidence>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementFlow {
    request: SettlementRequest,
    defeated: bool,
    pending: Option<Pending>,
    history: Vec<SettlementAction>,
    current: Option<SettlementView>,
    stable: u8,
    world: Option<Vec<RosterMember>>,
    world_reads: u8,
    entry: Option<(S, Option<BountyProgress>)>,
    entry_reads: u8,
    ready: Option<SettlementAction>,
    evaluation_seen: bool,
    money_seen: bool,
    scan_started: Option<u64>,
    scan_first: Option<u64>,
    scan_last: Option<u64>,
    scan_reads: u8,
    grid: Option<[RewardSlot; 8]>,
    grid_reads: u8,
    last_ms: u64,
    event: Option<LostVoidEventReceipt>,
    event_visits: u8,
    receipt: Option<SettlementReceipt>,
}
impl SettlementFlow {
    pub fn new(request: SettlementRequest) -> Result<Self, &'static str> {
        request.validate()?;
        let defeated = request.defeated();
        Ok(Self {
            request,
            defeated,
            pending: None,
            history: vec![],
            current: None,
            stable: 0,
            world: None,
            world_reads: 0,
            entry: None,
            entry_reads: 0,
            ready: None,
            evaluation_seen: false,
            money_seen: false,
            scan_started: None,
            scan_first: None,
            scan_last: None,
            scan_reads: 0,
            grid: None,
            grid_reads: 0,
            last_ms: 0,
            event: None,
            event_visits: 0,
            receipt: None,
        })
    }
    pub fn request(&self) -> &SettlementRequest {
        &self.request
    }
    pub fn receipt(&self) -> Option<&SettlementReceipt> {
        self.receipt.as_ref()
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn pending_action(&self) -> Option<SettlementAction> {
        self.pending.as_ref().map(|p| p.action)
    }
    pub fn event_screen_ready(&self) -> bool {
        self.pending_action() == Some(SettlementAction::Confirm)
            && self.stable == 2
            && self
                .current
                .as_ref()
                .is_some_and(|v| matches!(v.page, SettlementPage::Content(_)))
    }
    pub fn options(&self) -> ArtifactSelection {
        let mut out = self.request.source.options.clone();
        if let Some(e) = &self.event {
            for rule in &e.append_priority {
                if !out.battle_priority.contains(rule) {
                    out.battle_priority.push(rule.clone());
                }
            }
        }
        out
    }
    fn completion(&self) -> CompletionKind {
        if matches!(&self.request.source.outcome, LevelOutcome::Aborted { failure }
            if failure.reason != crate::lost_void_recovery::RetryReason::AgentDead)
        {
            CompletionKind::Aborted
        } else if self.defeated {
            CompletionKind::Defeated
        } else if self.request.victory_candidate() {
            CompletionKind::Completed
        } else {
            CompletionKind::Unattributed
        }
    }
    pub fn reward_evidence(&self) -> RewardEvidence {
        let span = self
            .scan_first
            .zip(self.scan_last)
            .map_or(0, |(a, b)| b.saturating_sub(a));
        let adequate = self.scan_reads >= REWARD_MIN_SAMPLES
            && span >= REWARD_MIN_SPAN_MS
            && self.grid_reads >= 2;
        RewardEvidence {
            evaluation_seen: self.evaluation_seen,
            money_seen: self.money_seen,
            samples: self.scan_reads,
            span_ms: span,
            adequate,
            grid: if adequate { self.grid } else { None },
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.last_ms > u64::from(self.request.timeout_ms) {
            return Err("结算检查点活动时钟超出预算");
        }
        if self.request.defeated() && !self.defeated
            || self.stable > 2
            || self.world_reads > 2
            || self.entry_reads > 2
            || self.grid_reads > 2
            || self.scan_reads > 64
            || self.history.len() > 3
            || self.event_visits > 128
            || self.event.is_some() != (self.event_visits > 0)
            || self
                .history
                .iter()
                .enumerate()
                .any(|(i, a)| self.history[..i].contains(a))
            || self.world.is_some() != (self.world_reads > 0)
            || self.entry.is_some() != (self.entry_reads > 0)
        {
            return Err("结算计数或输入历史无效");
        }
        if self.scan_first.is_some() != self.scan_last.is_some()
            || self.scan_first.is_some() != (self.scan_reads > 0)
            || self
                .scan_first
                .zip(self.scan_last)
                .is_some_and(|(a, b)| a > b || b > self.last_ms)
            || self.scan_started.is_some_and(|t| t > self.last_ms)
            || self.grid.is_some() != (self.grid_reads > 0)
        {
            return Err("结算奖励时序无效");
        }
        if let Some(v) = &self.current {
            v.validate()?;
        }
        if let Some(m) = &self.world {
            self.request.validate_party(m)?;
        }
        if let Some((screen, bounty)) = &self.entry {
            SettlementView {
                page: SettlementPage::Entry(*screen),
                reward: None,
                members: None,
                bounty: *bounty,
            }
            .validate()?;
        }
        if self.history.contains(&SettlementAction::Retreat) && !self.defeated
            || self.history.contains(&SettlementAction::Confirm)
                && self.history.len() != 1
                && !self.defeated
        {
            return Err("结算撤退或继续历史不符");
        }
        if let Some(p) = &self.pending {
            if p.rewards.is_some() != (p.action == SettlementAction::Finish)
                || p.action == SettlementAction::Retreat && !self.defeated
            {
                return Err("结算意图与奖励或失败来源不符");
            }
            if let Some(r) = &p.rewards {
                r.validate()?;
            }
        }
        if let Some(e) = &self.event {
            self.validate_event(e)?;
            if self.pending_action() != Some(SettlementAction::Confirm)
                && !(self.defeated && self.history.contains(&SettlementAction::Confirm))
            {
                return Err("结算自动事件缺少继续意图");
            }
        }
        self.reward_evidence().validate()?;
        if let Some(r) = &self.receipt {
            if r.run_id != self.request.source.run_id
                || r.level_id != self.request.source.level_id
                || r.settlement_id != self.request.settlement_id
                || r.actions != self.history
                || r.options != self.options()
            {
                return Err("结算回执身份或规则不符");
            }
            let valid = match &r.outcome {
                SettlementOutcome::Continue { arrival } => {
                    !self.defeated
                        && self.pending_action() == Some(SettlementAction::Confirm)
                        && self.history == [SettlementAction::Confirm]
                        && self.world_reads == 2
                        && self.world.as_ref() == Some(&arrival.members)
                        && arrival.level_id == self.request.next_level_id
                        && arrival.region == "挚交会谈"
                        && arrival
                            .members
                            .first()
                            .is_some_and(|m| m.id == arrival.front)
                }
                SettlementOutcome::Finished {
                    completion,
                    rewards,
                    caps,
                    entry,
                    bounty,
                } => {
                    self.pending.as_ref().is_some_and(|p| {
                        p.action == SettlementAction::Finish && p.rewards.as_ref() == Some(rewards)
                    }) && self.history.contains(&SettlementAction::Finish)
                        && self.entry_reads == 2
                        && self.entry == Some((*entry, *bounty))
                        && *completion == self.completion()
                        && *caps
                            == if *completion == CompletionKind::Completed {
                                rewards.caps()
                            } else {
                                RewardCaps {
                                    evaluation: None,
                                    period: None,
                                }
                            }
                }
            };
            if !valid {
                return Err("结算回执缺少实际后态");
            }
        }
        Ok(())
    }
    fn clear_fresh(&mut self) {
        self.current = None;
        self.stable = 0;
        self.world = None;
        self.world_reads = 0;
        self.entry = None;
        self.entry_reads = 0;
        self.ready = None;
        self.scan_started = None;
        self.scan_first = None;
        self.scan_last = None;
        self.scan_reads = 0;
        self.grid = None;
        self.grid_reads = 0;
    }
    pub fn invalidate_observation(&mut self) {
        self.clear_fresh();
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.clear_fresh();
        self.receipt = None;
        self.validate()
    }
    fn scan(&mut self, v: &SettlementView, now: u64) {
        self.scan_started.get_or_insert(now);
        if let Some(r) = &v.reward {
            self.evaluation_seen |= r.evaluation;
            self.money_seen |= r.money;
            if let Some(grid) = r.slots {
                self.scan_first.get_or_insert(now);
                self.scan_last = Some(now);
                self.scan_reads = self.scan_reads.saturating_add(1).min(64);
                self.grid_reads = if self.grid == Some(grid) {
                    (self.grid_reads + 1).min(2)
                } else {
                    1
                };
                self.grid = Some(grid);
                return;
            }
        }
        self.grid = None;
        self.grid_reads = 0;
    }
    fn finish(&mut self, outcome: SettlementOutcome, action: SettlementAction) {
        if !self.history.contains(&action) {
            self.history.push(action);
        }
        self.receipt = Some(SettlementReceipt {
            run_id: self.request.source.run_id,
            level_id: self.request.source.level_id,
            settlement_id: self.request.settlement_id,
            options: self.options(),
            actions: self.history.clone(),
            outcome,
        });
    }
    pub fn next(
        &mut self,
        v: &SettlementView,
        now_ms: u64,
    ) -> Result<Option<SettlementAction>, &'static str> {
        let mut next = self.clone();
        let action = next.next_inner(v, now_ms)?;
        next.validate()?;
        *self = next;
        Ok(action)
    }
    fn next_inner(
        &mut self,
        v: &SettlementView,
        now: u64,
    ) -> Result<Option<SettlementAction>, &'static str> {
        self.validate()?;
        v.validate()?;
        if now < self.last_ms || now > u64::from(self.request.timeout_ms) {
            return Err("结算活动时钟倒退或超限");
        }
        self.last_ms = now;
        self.ready = None;
        if self.completed() {
            return Ok(None);
        }
        self.stable = if self.current.as_ref().is_some_and(|old| old.page == v.page) {
            (self.stable + 1).min(2)
        } else {
            1
        };
        self.current = Some(v.clone());
        if v.page == SettlementPage::Failure {
            self.defeated = true;
        }
        if self.defeated && self.pending_action() == Some(SettlementAction::Confirm) {
            if !self.history.contains(&SettlementAction::Confirm) {
                self.history.push(SettlementAction::Confirm);
            }
            self.pending = None;
            self.clear_fresh();
            return Ok(None);
        }
        self.world_reads = if v.page == SettlementPage::World
            && v.members.is_some()
            && self.world.as_ref() == v.members.as_ref()
        {
            (self.world_reads + 1).min(2)
        } else if v.page == SettlementPage::World && v.members.is_some() {
            1
        } else {
            0
        };
        self.world = if self.world_reads > 0 {
            v.members.clone()
        } else {
            None
        };
        if let Some(m) = &self.world {
            self.request.validate_party(m)?;
        }
        if let SettlementPage::Entry(screen) = v.page {
            let key = (screen, v.bounty);
            self.entry_reads = if self.entry == Some(key) {
                (self.entry_reads + 1).min(2)
            } else {
                1
            };
            self.entry = Some(key);
        } else {
            self.entry = None;
            self.entry_reads = 0;
        }
        if let Some(p) = &self.pending {
            match p.action {
                SettlementAction::Confirm if self.world_reads == 2 => {
                    let members = self.world.clone().unwrap();
                    self.finish(
                        SettlementOutcome::Continue {
                            arrival: SettlementContinuation {
                                level_id: self.request.next_level_id,
                                region: "挚交会谈".into(),
                                front: members[0].id.clone(),
                                members,
                            },
                        },
                        SettlementAction::Confirm,
                    );
                }
                SettlementAction::Finish if self.entry_reads == 2 => {
                    let (entry, bounty) = self.entry.unwrap();
                    let rewards = p.rewards.clone().unwrap();
                    let completion = self.completion();
                    let caps = if completion == CompletionKind::Completed {
                        rewards.caps()
                    } else {
                        RewardCaps {
                            evaluation: None,
                            period: None,
                        }
                    };
                    self.finish(
                        SettlementOutcome::Finished {
                            completion,
                            rewards,
                            caps,
                            entry,
                            bounty,
                        },
                        SettlementAction::Finish,
                    );
                }
                SettlementAction::Retreat
                    if matches!(v.page, SettlementPage::Result { .. }) && self.stable == 2 =>
                {
                    self.history.push(SettlementAction::Retreat);
                    self.pending = None;
                    self.clear_fresh();
                }
                _ => {}
            }
            return Ok(None);
        }
        if v.page
            == (SettlementPage::Result {
                confirm: false,
                finish: true,
            })
            && !self.defeated
        {
            self.scan(v, now);
        } else {
            self.scan_started = None;
            self.scan_first = None;
            self.scan_last = None;
            self.scan_reads = 0;
            self.grid = None;
            self.grid_reads = 0;
        }
        if self.stable < 2 {
            return Ok(None);
        }
        let action = match v.page {
            SettlementPage::Failure => Some(SettlementAction::Retreat),
            SettlementPage::Result {
                confirm: true,
                finish: false,
            } if !self.defeated => Some(SettlementAction::Confirm),
            SettlementPage::Result {
                confirm: false,
                finish: true,
            } if self.defeated
                || self.reward_evidence().adequate
                || self
                    .scan_started
                    .is_some_and(|first| now.saturating_sub(first) >= REWARD_SCAN_MS) =>
            {
                Some(SettlementAction::Finish)
            }
            _ => None,
        };
        self.ready = action;
        Ok(action)
    }
    pub fn prepare(&mut self, action: SettlementAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.ready != Some(action) || self.pending.is_some() {
            return Err("结算没有对应的新帧输入许可");
        }
        self.pending = Some(Pending {
            action,
            posted: false,
            rewards: (action == SettlementAction::Finish).then(|| self.reward_evidence()),
        });
        self.ready = None;
        self.validate()
    }
    pub fn allows(&self, action: SettlementAction, v: &SettlementView) -> bool {
        if self.validate().is_err() || v.validate().is_err() || self.completed() {
            return false;
        }
        let Some(p) = &self.pending else {
            return false;
        };
        if p.action != action || p.posted || self.history.contains(&action) {
            return false;
        }
        match action {
            SettlementAction::Confirm => {
                !self.defeated
                    && v.page
                        == (SettlementPage::Result {
                            confirm: true,
                            finish: false,
                        })
            }
            SettlementAction::Retreat => self.defeated && v.page == SettlementPage::Failure,
            SettlementAction::Finish => {
                v.page
                    == (SettlementPage::Result {
                        confirm: false,
                        finish: true,
                    })
                    && p.rewards.as_ref().is_some_and(|r| {
                        !r.adequate || v.reward.as_ref().and_then(|v| v.slots) == r.grid
                    })
            }
        }
    }
    pub fn posted(&mut self, action: SettlementAction) -> Result<(), &'static str> {
        self.validate()?;
        let p = self.pending.as_mut().ok_or("结算没有待投递意图")?;
        if p.action != action {
            return Err("结算投递动作不符");
        }
        p.posted = true;
        Ok(())
    }
    pub fn not_posted(&mut self, action: SettlementAction) -> Result<(), &'static str> {
        self.validate()?;
        if self
            .pending
            .as_ref()
            .is_none_or(|p| p.action != action || p.posted)
            || self.history.contains(&action)
        {
            return Err("不能撤销已投递或不匹配的结算意图");
        }
        self.pending = None;
        self.clear_fresh();
        Ok(())
    }
    fn validate_event(&self, e: &LostVoidEventReceipt) -> Result<(), &'static str> {
        if e.run_id != self.request.source.run_id
            || e.interaction_id != self.request.settlement_id
            || e.returned_to != S::World
            || e.actions == 0 && e.children == 0
            || e.actions > 128
            || e.children > 128
            || e.gear_repeats > 128
            || !e.completed_first.is_empty()
        {
            return Err("中途结算自动事件不属于当前继续意图");
        }
        let mut options = self.request.source.options.clone();
        options
            .battle_priority
            .extend(e.append_priority.iter().cloned());
        choose_artifacts(&self.request.challenge, &[], 0, &options)?;
        Ok(())
    }
    pub fn accept_event(&mut self, e: &LostVoidEventReceipt) -> Result<(), &'static str> {
        self.validate()?;
        self.validate_event(e)?;
        if !self.event_screen_ready() || self.event_visits >= 128 {
            return Err("结算没有等待中的自动事件");
        }
        if self.event.as_ref().is_some_and(|old| {
            e.actions < old.actions
                || e.children < old.children
                || e.gear_repeats < old.gear_repeats
                || !e.append_priority.starts_with(&old.append_priority)
        }) {
            return Err("中途结果事件历史倒退");
        }
        self.event = Some(e.clone());
        self.event_visits += 1;
        self.clear_fresh();
        self.validate()
    }
}
