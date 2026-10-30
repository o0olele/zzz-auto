//! 运行历史摘要和仅记账恢复；这些数据不提供游戏执行资格。
use crate::{
    RunState,
    config::valid_identifier,
    daily::GameRegion,
    lost_void::{LostVoidConfig, LostVoidDecision, plan},
    lost_void_claim_record::LostVoidClaimRecord,
    lost_void_record::LostVoidSettlementRecord,
    lost_void_record_store::{
        LostVoidApplicationPlanning, LostVoidRecordError, LostVoidRecordStore,
    },
    lost_void_settlement::CompletionKind,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LostVoidAppPhase {
    Plan,
    Challenge,
    CommitChallenge,
    Hub,
    Bounty,
    CommitClaim,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LostVoidAppEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LostVoidAppCompletion {
    Completed { decision: LostVoidDecision },
    Skipped { decision: LostVoidDecision },
    ChallengeFailed { completion: CompletionKind },
}
// ts-rs 的枚举派生不识别 deny_unknown_fields；反序列化仍独立使用严格枚举。
impl<'de> Deserialize<'de> for LostVoidAppCompletion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Completed { decision: LostVoidDecision },
            Skipped { decision: LostVoidDecision },
            ChallengeFailed { completion: CompletionKind },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Completed { decision } => Self::Completed { decision },
            Wire::Skipped { decision } => Self::Skipped { decision },
            Wire::ChallengeFailed { completion } => Self::ChallengeFailed { completion },
        })
    }
}
impl LostVoidAppCompletion {
    pub fn successful(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::Skipped { .. })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LostVoidAppContext {
    pub run_id: Uuid,
    pub instance_id: u32,
    pub group_id: String,
    pub config_revision: u32,
    pub region: GameRegion,
}
impl LostVoidAppContext {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.run_id.is_nil()
            || !(1..=9999).contains(&self.instance_id)
            || !valid_identifier(&self.group_id)
        {
            return Err("迷失之地应用的冻结运行身份无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LostVoidAppProgress {
    pub context: LostVoidAppContext,
    pub config: LostVoidConfig,
    pub state: RunState,
    pub phase: LostVoidAppPhase,
    pub end: Option<LostVoidAppEnd>,
    pub completion: Option<LostVoidAppCompletion>,
    pub timeout_ms: u32,
    #[ts(type = "number")]
    pub active_ms: u64,
    pub inputs_released: bool,
    pub returned_world: bool,
    pub challenges_started: u16,
    pub successful_challenges: u16,
    pub completed_claim_checks: u16,
    pub current_challenge_id: Option<Uuid>,
    pub planning: Option<LostVoidApplicationPlanning>,
    #[ts(type = "unknown | null")]
    pub pending_settlement: Option<LostVoidSettlementRecord>,
    #[ts(type = "unknown | null")]
    pub pending_claim: Option<LostVoidClaimRecord>,
    pub committed_settlements: Vec<Uuid>,
    pub committed_claims: Vec<Uuid>,
}
pub const DEFAULT_APPLICATION_TIMEOUT_MS: u32 = 7_200_000;
fn valid_ids(ids: &[Uuid], limit: usize) -> bool {
    ids.len() <= limit
        && ids.iter().all(|id| !id.is_nil())
        && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
}
impl LostVoidAppProgress {
    pub(crate) fn request_state(&mut self, state: RunState) {
        self.state = state;
        self.end = None;
        self.completion = None;
        self.returned_world = false;
        if state == RunState::Running {
            self.inputs_released = false;
        }
    }
    pub fn new(
        context: LostVoidAppContext,
        config: LostVoidConfig,
        timeout_ms: u32,
    ) -> Result<Self, &'static str> {
        let progress = Self {
            context,
            config,
            state: RunState::Running,
            phase: LostVoidAppPhase::Plan,
            end: None,
            completion: None,
            timeout_ms,
            active_ms: 0,
            inputs_released: true,
            returned_world: false,
            challenges_started: 0,
            successful_challenges: 0,
            completed_claim_checks: 0,
            current_challenge_id: None,
            planning: None,
            pending_settlement: None,
            pending_claim: None,
            committed_settlements: vec![],
            committed_claims: vec![],
        };
        progress.validate()?;
        Ok(progress)
    }
    pub fn matches_record(&self, record: &crate::RunRecord) -> bool {
        self.validate().is_ok()
            && matches!(record.app_id.as_str(), "lost_void" | "application_group")
            && record.context.as_ref().is_some_and(|context| {
                self.context.run_id == record.id
                    && self.context.instance_id == context.instance_id
                    && self.context.group_id == context.group_id
                    && self.context.config_revision == context.config_revision
                    && context
                        .game_day
                        .as_ref()
                        .is_some_and(|day| day.region == self.context.region)
                    && context.app_ids.iter().any(|id| id == "lost_void")
                    && (record.group.is_some() || record.state == self.state)
            })
    }
    pub fn validate_successor(&self, old: &Self) -> Result<(), &'static str> {
        self.validate()?;
        old.validate()?;
        if matches!(
            old.end,
            Some(
                LostVoidAppEnd::Ready
                    | LostVoidAppEnd::Stopped
                    | LostVoidAppEnd::TimedOut
                    | LostVoidAppEnd::Failed
            )
        ) && self != old
        {
            return Err("已结束的迷失之地摘要不能改写");
        }
        if self.context != old.context
            || self.config != old.config
            || self.timeout_ms != old.timeout_ms
            || self.active_ms < old.active_ms
            || self.challenges_started < old.challenges_started
            || self.successful_challenges < old.successful_challenges
            || self.completed_claim_checks < old.completed_claim_checks
            || !self
                .committed_settlements
                .starts_with(&old.committed_settlements)
            || !self.committed_claims.starts_with(&old.committed_claims)
            || self.challenges_started == old.challenges_started
                && self.current_challenge_id != old.current_challenge_id
            || self.challenges_started > old.challenges_started.saturating_add(1)
        {
            return Err("迷失之地摘要不能改绑、回退计数或改写已提交身份");
        }
        if old
            .pending_settlement
            .as_ref()
            .is_some_and(|record| match &self.pending_settlement {
                Some(next) => next != record,
                None => !self.committed_settlements.contains(&record.settlement_id()),
            })
            || old
                .pending_claim
                .as_ref()
                .is_some_and(|record| match &self.pending_claim {
                    Some(next) => next != record,
                    None => !self.committed_claims.contains(&record.claim_id()),
                })
        {
            return Err("迷失之地待提交证据不能丢弃或改写");
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.context.validate()?;
        self.config.validate()?;
        if !(1000..=7_200_000).contains(&self.timeout_ms)
            || self.challenges_started > 1024
            || self.successful_challenges > self.challenges_started
            || self.completed_claim_checks > 1024
            || !valid_ids(&self.committed_settlements, 1024)
            || !valid_ids(&self.committed_claims, 1024)
            || self.committed_settlements.len() > usize::from(self.challenges_started)
            || self.committed_settlements.len() < usize::from(self.successful_challenges)
            || self.committed_claims.len() != usize::from(self.completed_claim_checks)
            || self
                .current_challenge_id
                .is_some_and(|id| id.is_nil() || id == self.context.run_id)
            || self.challenges_started == 0 && self.current_challenge_id.is_some()
            || self.challenges_started > 0 && self.current_challenge_id.is_none()
        {
            return Err("迷失之地应用次数、预算或提交身份不一致");
        }
        if let Some(p) = &self.planning {
            p.progress.day.validate()?;
            if p.progress.day.region != self.context.region
                || p.progress.daily_run_times > p.progress.weekly_run_times
                || !valid_ids(
                    &p.pending_claims,
                    crate::lost_void_claim_record::MAX_CLAIM_SOURCES,
                )
            {
                return Err("迷失之地计划进度与冻结区服不符");
            }
        }
        if let Some(record) = &self.pending_settlement {
            record.validate()?;
            if record.instance_id() != self.context.instance_id
                || record.day().region != self.context.region
                || Some(record.run_id()) != self.current_challenge_id
                || self.phase != LostVoidAppPhase::CommitChallenge
                    && self.phase != LostVoidAppPhase::Challenge
            {
                return Err("待补交结算不属于本次应用与实际挑战");
            }
        }
        if let Some(record) = &self.pending_claim {
            record.validate()?;
            if record.instance_id() != self.context.instance_id
                || record.day().region != self.context.region
                || record.receipt().run_id != self.context.run_id
                || self.phase != LostVoidAppPhase::CommitClaim
                    && self.phase != LostVoidAppPhase::Bounty
            {
                return Err("待补交领取不属于本次应用");
            }
        }
        if self.pending_claim.is_some() && self.pending_settlement.is_some() {
            return Err("应用不能同时等待补交结算与领取");
        }
        let terminal_ok = match self.end {
            Some(LostVoidAppEnd::Ready) => {
                self.inputs_released
                    && self.phase == LostVoidAppPhase::Done
                    && self.pending_settlement.is_none()
                    && self.pending_claim.is_none()
                    && self
                        .planning
                        .as_ref()
                        .is_some_and(|p| p.pending_claims.is_empty())
                    && self.completion.as_ref().is_some_and(|c| {
                        self.state
                            == if c.successful() {
                                RunState::Succeeded
                            } else {
                                RunState::Failed
                            }
                    })
            }
            Some(LostVoidAppEnd::Paused) => {
                self.state == RunState::Paused && self.inputs_released && self.completion.is_none()
            }
            Some(LostVoidAppEnd::Stopped) => {
                self.state == RunState::Stopped && self.inputs_released && self.completion.is_none()
            }
            Some(LostVoidAppEnd::TimedOut) => {
                self.state == RunState::Failed && self.inputs_released && self.completion.is_none()
            }
            Some(LostVoidAppEnd::Failed) => {
                self.state == RunState::Failed && self.completion.is_none()
            }
            None => {
                matches!(
                    self.state,
                    RunState::Running | RunState::Pausing | RunState::Stopping
                ) && self.completion.is_none()
            }
        };
        if !terminal_ok
            || self.returned_world
                && (self.end != Some(LostVoidAppEnd::Ready) || self.completed_claim_checks == 0)
        {
            return Err("迷失之地应用终态与实际交回证明不一致");
        }
        if let Some(completion) = &self.completion {
            match completion {
                LostVoidAppCompletion::Completed { decision }
                | LostVoidAppCompletion::Skipped { decision } => {
                    let p = self.planning.as_ref().ok_or("完成缺少账号计划进度")?;
                    if *decision == LostVoidDecision::Run
                        || plan(&self.config, &p.progress.day, &p.progress.history())? != *decision
                    {
                        return Err("迷失之地完成决定与账号目标不符");
                    }
                    if matches!(completion, LostVoidAppCompletion::Skipped { .. }) {
                        if self.returned_world
                            || self.challenges_started != 0
                            || self.completed_claim_checks != 0
                            || !self.committed_settlements.is_empty()
                        {
                            return Err("零输入跳过不能包含游戏工作");
                        }
                    } else if !self.returned_world
                        || self.completed_claim_checks == 0
                        || self.committed_settlements.len()
                            != usize::from(self.successful_challenges)
                    {
                        return Err("应用完成缺少领取返回或通关提交");
                    }
                }
                LostVoidAppCompletion::ChallengeFailed { completion } => {
                    if *completion == CompletionKind::Completed
                        || !self.returned_world
                        || self.committed_settlements.len()
                            != usize::from(self.successful_challenges) + 1
                    {
                        return Err("失败应用的结算性质或安全返回不符");
                    }
                }
            }
        }
        Ok(())
    }
    pub fn completed(&self) -> bool {
        self.validate().is_ok()
            && self.end == Some(LostVoidAppEnd::Ready)
            && self
                .completion
                .as_ref()
                .is_some_and(LostVoidAppCompletion::successful)
    }
    /// 只幂等提交紧凑记录，不修改本摘要，不创建或继续任何游戏执行器。
    pub fn replay_pending(
        &self,
        store: &LostVoidRecordStore,
    ) -> Result<LostVoidPendingReplay, LostVoidRecordError> {
        self.validate().map_err(LostVoidRecordError::Invalid)?;
        let settlement = self
            .pending_settlement
            .as_ref()
            .map(|record| store.commit(record))
            .transpose()?;
        let claim = self
            .pending_claim
            .as_ref()
            .map(|record| store.acknowledge_claim(record))
            .transpose()?;
        Ok(LostVoidPendingReplay {
            settlement_id: settlement.map(|r| r.settlement_id),
            settlement_changed: settlement.is_some_and(|r| r.changed),
            claim_id: claim.map(|r| r.claim_id),
            claim_changed: claim.is_some_and(|r| r.changed),
        })
    }
    /// 历史只能引用原先落盘的提交；后来的其他运行不改变本次证明。
    pub fn verify_committed(&self, store: &LostVoidRecordStore) -> Result<(), LostVoidRecordError> {
        use LostVoidRecordError::Invalid;
        self.validate().map_err(Invalid)?;
        if let Some(pending) = &self.pending_settlement
            && self
                .committed_settlements
                .contains(&pending.settlement_id())
            && store.committed_record(pending.settlement_id())? != *pending
        {
            return Err(Invalid("待核对结算与已提交内容不符"));
        }
        if let Some(pending) = &self.pending_claim
            && self.committed_claims.contains(&pending.claim_id())
            && store.claim_record(pending.claim_id())? != *pending
        {
            return Err(Invalid("待核对领取与已提交内容不符"));
        }
        let mut runs = BTreeSet::new();
        let mut successes = 0;
        let mut failures = Vec::new();
        for id in &self.committed_settlements {
            let record = store.committed_record(*id)?;
            if record.instance_id() != self.context.instance_id
                || record.day().region != self.context.region
                || record.run_id() == self.context.run_id
                || !runs.insert(record.run_id())
            {
                return Err(Invalid("历史结算不属于冻结账号或挑战重复"));
            }
            successes += u16::from(record.completion() == CompletionKind::Completed);
            if record.completion() != CompletionKind::Completed {
                failures.push(record.completion());
            }
        }
        if successes != self.successful_challenges {
            return Err(Invalid("历史通关次数与原始结算不符"));
        }
        if let Some(LostVoidAppCompletion::ChallengeFailed { completion }) = &self.completion
            && failures != [*completion]
        {
            return Err(Invalid("历史失败性质与原始结算不符"));
        }
        let mut claimed = BTreeSet::new();
        for id in &self.committed_claims {
            let record = store.claim_record(*id)?;
            if record.instance_id() != self.context.instance_id
                || record.day().region != self.context.region
                || record.receipt().run_id != self.context.run_id
            {
                return Err(Invalid("历史领取不属于冻结账号与应用"));
            }
            for source_id in record.source_ids() {
                let source = store.committed_record(*source_id)?;
                if source.instance_id() != self.context.instance_id
                    || !crate::lost_void_claim_record::same_claim_week(source.day(), record.day())
                        .map_err(Invalid)?
                    || source.day().date > record.day().date
                {
                    return Err(Invalid("历史领取绑定了其他账号或游戏周的结算"));
                }
                claimed.insert(*source_id);
            }
        }
        if self.end == Some(LostVoidAppEnd::Ready)
            && self
                .committed_settlements
                .iter()
                .any(|id| !claimed.contains(id))
        {
            return Err(Invalid("历史结束仍有未被本次领取绑定的结算"));
        }
        Ok(())
    }
}

impl crate::RunSlot {
    pub fn begin_lost_void(
        &mut self,
        context: crate::run_plan::RunContext,
        progress: LostVoidAppProgress,
    ) -> Result<crate::RunRecord, &'static str> {
        context.validate()?;
        progress.validate()?;
        if context.app_ids != ["lost_void"]
            || progress
                != LostVoidAppProgress::new(
                    progress.context.clone(),
                    progress.config.clone(),
                    progress.timeout_ms,
                )?
            || progress.context.instance_id != context.instance_id
            || progress.context.group_id != context.group_id
            || progress.context.config_revision != context.config_revision
            || !context
                .game_day
                .as_ref()
                .is_some_and(|day| day.region == progress.context.region)
        {
            return Err("迷失之地独立运行必须从冻结的未执行状态开始");
        }
        let mut record = self.begin("lost_void")?;
        record.id = progress.context.run_id;
        record.context = Some(context);
        record.lost_void = Some(Box::new(progress));
        self.current = Some(record.clone());
        Ok(record)
    }
    pub fn update_lost_void(
        &mut self,
        id: Uuid,
        mut progress: LostVoidAppProgress,
        message: String,
    ) -> Result<crate::RunRecord, &'static str> {
        let record = self.current.as_mut().ok_or("当前没有迷失之地任务")?;
        if record.id != id || record.state.is_terminal() || !record.lost_void_is_current() {
            return Err("迷失之地不是当前有效分项");
        }
        progress.validate_successor(record.lost_void.as_ref().ok_or("迷失之地缺少起始摘要")?)?;
        if matches!(record.state, RunState::Pausing | RunState::Stopping)
            && (progress.state == RunState::Running
                || progress.end == Some(LostVoidAppEnd::Ready)
                || record.state == RunState::Stopping && progress.state == RunState::Paused)
        {
            progress.state = record.state;
            progress.end = None;
            progress.completion = None;
            progress.returned_world = false;
        }
        progress.validate()?;
        if let Some(schedule) = &mut self.group_schedule {
            if progress.state == RunState::Pausing && schedule.progress().state == RunState::Running
            {
                schedule.request_pause()?;
            }
            if progress.state == RunState::Stopping {
                schedule.request_stop();
            }
            if progress.state == RunState::Paused {
                if schedule.progress().state == RunState::Running {
                    schedule.request_pause()?;
                }
                if schedule.progress().state == RunState::Pausing {
                    schedule.acknowledge_pause(progress.inputs_released)?;
                }
            } else if progress.state.is_terminal() {
                use crate::group_run::AppOutcome;
                let outcome = if progress.completed() {
                    AppOutcome::Succeeded
                } else if progress.state == RunState::Stopped {
                    AppOutcome::Stopped
                } else {
                    AppOutcome::Failed
                };
                schedule.complete_current(
                    "lost_void",
                    outcome,
                    progress.inputs_released,
                    chrono::Utc::now(),
                )?;
            }
            record.group = Some(schedule.progress().clone());
            record.state = schedule.progress().state;
        } else {
            record.state = progress.state;
        }
        record.lost_void = Some(Box::new(progress));
        record.message = message;
        record.finished_at = record.state.is_terminal().then(chrono::Utc::now);
        Ok(record.clone())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LostVoidPendingReplay {
    pub settlement_id: Option<Uuid>,
    pub settlement_changed: bool,
    pub claim_id: Option<Uuid>,
    pub claim_changed: bool,
}
