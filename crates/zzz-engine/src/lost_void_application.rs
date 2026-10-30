//! 迷失之地应用循环；实际挑战、记账、领取和最终返回共用控制及总活动预算。
use crate::{
    lost_void_bounty::{LostVoidBountyEnd, LostVoidBountyRun, LostVoidBountySnapshot},
    lost_void_challenge::{
        ChallengeOutcome, LostVoidChallengeEnd, LostVoidChallengeRun, LostVoidChallengeSnapshot,
    },
    lost_void_combat::FrozenLostVoidPreparation,
    lost_void_entry::{LostVoidEntryEnd, LostVoidEntryRun, LostVoidEntrySnapshot},
    lost_void_level::LostVoidAttemptLimits,
    lost_void_preparation::LostVoidPreparationRun,
};
use anyhow::{Context, Result, ensure};
use maa_adapter::NativeSession;
pub use od_core::lost_void_application::{
    LostVoidAppCompletion as ApplicationOutcome, LostVoidAppEnd as LostVoidApplicationEnd,
    LostVoidAppPhase as ApplicationPhase,
};
use od_core::lost_void_application::{LostVoidAppContext, LostVoidAppProgress};
use od_core::{
    daily::{GameDay, GameRegion},
    lost_void::{ArtifactSelection, LostVoidConfig, LostVoidDecision, choose_artifacts, plan},
    lost_void_bounty::BountyRequest,
    lost_void_claim_record::LostVoidClaimRecord,
    lost_void_entry::{EntryOutcome, LostVoidEntryRequest},
    lost_void_record::LostVoidSettlementRecord,
    lost_void_record_store::{LostVoidApplicationPlanning, LostVoidRecordStore},
    lost_void_settlement::{CompletionKind, SettlementOutcome},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::Path,
    time::{Duration, Instant},
};
use uuid::Uuid;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidApplicationReceipt {
    pub run_id: Uuid,
    pub instance_id: u32,
    pub outcome: ApplicationOutcome,
    pub challenges_started: u16,
    pub successful_challenges: u16,
    pub completed_claim_checks: u16,
    pub planning: LostVoidApplicationPlanning,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidApplicationSnapshot {
    pub run_id: Uuid,
    pub instance_id: u32,
    pub region: GameRegion,
    pub config: LostVoidConfig,
    pub options: ArtifactSelection,
    pub phase: ApplicationPhase,
    pub timeout_ms: u32,
    pub active_ms: u64,
    pub inputs_released: bool,
    pub challenges_started: u16,
    pub successful_challenges: u16,
    pub completed_claim_checks: u16,
    pub planning: Option<LostVoidApplicationPlanning>,
    pub challenge: Option<Box<LostVoidChallengeSnapshot>>,
    pub hub: Option<LostVoidEntrySnapshot>,
    pub bounty: Option<LostVoidBountySnapshot>,
    pub pending_settlement: Option<LostVoidSettlementRecord>,
    pub pending_claim: Option<LostVoidClaimRecord>,
    pub committed_settlements: Vec<Uuid>,
    pub committed_claims: Vec<Uuid>,
    pub failed_completion: Option<CompletionKind>,
    pub receipt: Option<LostVoidApplicationReceipt>,
}
pub struct LostVoidApplicationRun {
    snapshot: LostVoidApplicationSnapshot,
    run_context: Option<LostVoidAppContext>,
    frozen: FrozenLostVoidPreparation,
    store: LostVoidRecordStore,
    options: ArtifactSelection,
    control: PipelineControl,
    challenge: Option<LostVoidChallengeRun>,
    hub: Option<LostVoidEntryRun>,
    bounty: Option<LostVoidBountyRun>,
    claim_sources: Vec<Uuid>,
    counted_settlements: BTreeSet<Uuid>,
    counted_claims: BTreeSet<Uuid>,
    force_bounty: bool,
    used_native: bool,
    returned_after_work: bool,
    child_started: bool,
    attempt_limits: LostVoidAttemptLimits,
    steps: u16,
    end: Option<LostVoidApplicationEnd>,
}
enum ChildEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidApplicationSpec {
    pub instance_id: u32,
    pub region: GameRegion,
    pub config: LostVoidConfig,
    pub options: ArtifactSelection,
    pub timeout_ms: u32,
}
impl LostVoidApplicationRun {
    pub fn new(
        spec: LostVoidApplicationSpec,
        frozen: FrozenLostVoidPreparation,
        data: &Path,
        control: PipelineControl,
    ) -> Result<Self> {
        let LostVoidApplicationSpec {
            instance_id,
            region,
            config,
            options,
            timeout_ms,
        } = spec;
        config.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            (1..=9999).contains(&instance_id) && (1000..=7_200_000).contains(&timeout_ms),
            "迷失之地应用账号或总预算无效"
        );
        ensure!(control.requested().is_none(), "不能从已中断控制创建应用");
        ensure!(
            config.mission_name == frozen.seed().mission
                && config.challenge == frozen.seed().challenge,
            "应用配置与冻结准备不一致"
        );
        frozen.seed().validate_roster_settings()?;
        ensure!(
            frozen.npc_detector_root().is_some(),
            "应用首次输入前缺少冻结导航资源"
        );
        // 应用跨周或完成 UP 后仍只使用首次编译的候选。
        let _ = frozen.for_next_challenge(Uuid::new_v4(), false)?;
        choose_artifacts(&config.challenge, &[], 0, &options).map_err(anyhow::Error::msg)?;
        let run_id = frozen.seed().run_id;
        Ok(Self {
            run_context: None,
            snapshot: LostVoidApplicationSnapshot {
                run_id,
                instance_id,
                region,
                config,
                options: options.clone(),
                phase: ApplicationPhase::Plan,
                timeout_ms,
                active_ms: 0,
                inputs_released: true,
                challenges_started: 0,
                successful_challenges: 0,
                completed_claim_checks: 0,
                planning: None,
                challenge: None,
                hub: None,
                bounty: None,
                pending_settlement: None,
                pending_claim: None,
                committed_settlements: vec![],
                committed_claims: vec![],
                failed_completion: None,
                receipt: None,
            },
            frozen,
            store: LostVoidRecordStore::new(data)?,
            options,
            control,
            challenge: None,
            hub: None,
            bounty: None,
            claim_sources: vec![],
            counted_settlements: BTreeSet::new(),
            counted_claims: BTreeSet::new(),
            force_bounty: false,
            used_native: false,
            returned_after_work: false,
            child_started: false,
            attempt_limits: Default::default(),
            steps: 0,
            end: None,
        })
    }
    pub fn snapshot(&self) -> LostVoidApplicationSnapshot {
        self.snapshot.clone()
    }
    /// 运行摘要身份必须在首次执行前冻结，不能在结束后换记其他组或修订。
    pub fn with_run_context(mut self, context: LostVoidAppContext) -> Result<Self> {
        context.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.steps == 0
                && self.snapshot.active_ms == 0
                && self.end.is_none()
                && self.run_context.is_none(),
            "应用摘要身份只能在首次执行前冻结"
        );
        ensure!(
            context.run_id == self.snapshot.run_id
                && context.instance_id == self.snapshot.instance_id
                && context.region == self.snapshot.region,
            "应用摘要身份与实际执行器不符"
        );
        self.run_context = Some(context);
        Ok(self)
    }
    /// 完成仅来自当前仍有效的实际回执；快照里撤销的候选不能成为历史成功。
    pub fn progress(&self) -> Result<LostVoidAppProgress> {
        use LostVoidApplicationEnd as E;
        use od_core::RunState as S;
        let context = self
            .run_context
            .clone()
            .context("应用缺少输入前冻结的摘要身份")?;
        let requested = if matches!(self.end, Some(E::Failed | E::TimedOut | E::Stopped)) {
            None
        } else {
            self.control.requested()
        };
        let (state, end, completion) = match requested {
            Some(Interrupt::Pause) if self.end == Some(E::Paused) => {
                (S::Paused, Some(E::Paused), None)
            }
            Some(Interrupt::Pause) => (S::Pausing, None, None),
            Some(Interrupt::Stop) if self.end == Some(E::Stopped) => {
                (S::Stopped, Some(E::Stopped), None)
            }
            Some(Interrupt::Stop) => (S::Stopping, None, None),
            None => match self.end {
                Some(E::Ready) => {
                    let receipt = self.receipt().context("应用完成回执已撤销")?;
                    (
                        if receipt.outcome.successful() {
                            S::Succeeded
                        } else {
                            S::Failed
                        },
                        Some(E::Ready),
                        Some(receipt.outcome.clone()),
                    )
                }
                Some(E::Paused) => (S::Paused, Some(E::Paused), None),
                Some(E::Stopped) => (S::Stopped, Some(E::Stopped), None),
                Some(E::Failed) => (S::Failed, Some(E::Failed), None),
                Some(E::TimedOut) => (S::Failed, Some(E::TimedOut), None),
                None => (S::Running, None, None),
            },
        };
        Self::build_progress(&self.snapshot, context, state, end, completion)
    }
    fn build_progress(
        snapshot: &LostVoidApplicationSnapshot,
        context: LostVoidAppContext,
        state: od_core::RunState,
        end: Option<LostVoidApplicationEnd>,
        completion: Option<ApplicationOutcome>,
    ) -> Result<LostVoidAppProgress> {
        let progress = LostVoidAppProgress {
            context,
            config: snapshot.config.clone(),
            state,
            phase: snapshot.phase,
            end,
            returned_world: end == Some(LostVoidApplicationEnd::Ready)
                && !matches!(completion, Some(ApplicationOutcome::Skipped { .. })),
            completion,
            timeout_ms: snapshot.timeout_ms,
            active_ms: snapshot.active_ms,
            inputs_released: snapshot.inputs_released,
            challenges_started: snapshot.challenges_started,
            successful_challenges: snapshot.successful_challenges,
            completed_claim_checks: snapshot.completed_claim_checks,
            current_challenge_id: snapshot.challenge.as_ref().map(|c| c.run_id),
            planning: snapshot.planning.clone(),
            pending_settlement: snapshot.pending_settlement.clone(),
            pending_claim: snapshot.pending_claim.clone(),
            committed_settlements: snapshot.committed_settlements.clone(),
            committed_claims: snapshot.committed_claims.clone(),
        };
        progress.validate().map_err(anyhow::Error::msg)?;
        Ok(progress)
    }
    /// 工作线程保存的中间摘要只来自本次实际回调；终态仍须 execute 返回后调用 progress。
    pub fn execute_with_progress(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidAppProgress) -> Result<()>,
    ) -> Result<LostVoidApplicationEnd> {
        let context = self
            .run_context
            .clone()
            .context("应用缺少输入前冻结的摘要身份")?;
        let control = self.control.clone();
        self.execute_with_budget(session, game, parent_remaining, |snapshot| {
            let state = match control.requested() {
                Some(Interrupt::Pause) => od_core::RunState::Pausing,
                Some(Interrupt::Stop) => od_core::RunState::Stopping,
                None => od_core::RunState::Running,
            };
            publish(Self::build_progress(
                &snapshot,
                context.clone(),
                state,
                None,
                None,
            )?)
        })
    }
    /// 父运行槽在 execute 安全交回之后才暂停，也必须撤销并重新取得新帧证明。
    pub fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidApplicationEnd::Ready | LostVoidApplicationEnd::Paused)
            ) && self.snapshot.inputs_released
                && self.control.requested() == Some(Interrupt::Pause),
            "应用尚未由父流程安全暂停"
        );
        ensure!(
            self.acknowledge_parent_interrupt()? == LostVoidApplicationEnd::Paused,
            "停止请求不能继续"
        );
        self.resume()
    }
    /// 只接纳本进程已安全交回对象收到的迟到父请求，不产生输入或重建执行器。
    pub fn acknowledge_parent_interrupt(&mut self) -> Result<LostVoidApplicationEnd> {
        use LostVoidApplicationEnd as E;
        ensure!(
            (matches!(self.end, Some(E::Ready | E::Paused | E::Stopped))
                || self.end.is_none() && self.control.requested().is_some())
                && self.snapshot.inputs_released,
            "父交接尚未确认输入释放"
        );
        let end = if self.end == Some(E::Stopped) {
            E::Stopped
        } else {
            match self.control.requested() {
                Some(Interrupt::Stop) => E::Stopped,
                Some(Interrupt::Pause) => {
                    if self.control.acknowledge_pause() {
                        E::Paused
                    } else {
                        E::Stopped
                    }
                }
                None => self.end.context("实际应用尚未交回")?,
            }
        };
        self.end = Some(end);
        if end != E::Ready {
            self.snapshot.receipt = None;
        }
        Ok(end)
    }
    pub fn uses_control(&self, c: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &c.cancellation())
    }
    pub(crate) fn verify_prepared(&self) -> Result<()> {
        self.frozen.verify()
    }
    pub fn receipt(&self) -> Option<&LostVoidApplicationReceipt> {
        (self.end == Some(LostVoidApplicationEnd::Ready)
            && self.snapshot.inputs_released
            && self.control.requested().is_none())
        .then_some(self.snapshot.receipt.as_ref())
        .flatten()
    }
    pub fn with_attempt_limits(mut self, limits: LostVoidAttemptLimits) -> Result<Self> {
        ensure!(
            self.steps == 0 && self.snapshot.active_ms == 0 && self.end.is_none(),
            "应用运行后不能更换步骤预算"
        );
        limits.validate()?;
        self.attempt_limits = limits;
        Ok(self)
    }
    fn sync_children(&mut self) {
        self.snapshot.challenge = self.challenge.as_ref().map(|c| Box::new(c.snapshot()));
        self.snapshot.hub = self.hub.as_ref().map(|h| h.snapshot());
        self.snapshot.bounty = self.bounty.as_ref().map(|b| b.snapshot());
    }
    fn finish(
        &mut self,
        planning: LostVoidApplicationPlanning,
        decision: LostVoidDecision,
    ) -> Result<()> {
        ensure!(
            planning.pending_claims.is_empty(),
            "待领取尚未清除，不能结束应用"
        );
        let outcome = if let Some(completion) = self.snapshot.failed_completion {
            ApplicationOutcome::ChallengeFailed { completion }
        } else if self.used_native {
            ensure!(decision != LostVoidDecision::Run, "挑战目标尚未达到");
            ApplicationOutcome::Completed { decision }
        } else {
            ensure!(decision != LostVoidDecision::Run, "未运行应用不能交回完成");
            ApplicationOutcome::Skipped { decision }
        };
        if self.used_native {
            ensure!(
                self.returned_after_work
                    && self.bounty.as_ref().and_then(|b| b.receipt()).is_some(),
                "应用尚未安全确认最终普通大世界返回"
            );
        }
        self.snapshot.receipt = Some(LostVoidApplicationReceipt {
            run_id: self.snapshot.run_id,
            instance_id: self.snapshot.instance_id,
            outcome,
            challenges_started: self.snapshot.challenges_started,
            successful_challenges: self.snapshot.successful_challenges,
            completed_claim_checks: self.snapshot.completed_claim_checks,
            planning,
        });
        self.snapshot.phase = ApplicationPhase::Done;
        Ok(())
    }
    fn start_challenge(&mut self, up_completed: bool) -> Result<()> {
        ensure!(
            self.snapshot.failed_completion.is_none(),
            "失败后不能另开挑战绕过本层额度"
        );
        ensure!(
            self.snapshot.challenges_started < 1024,
            "应用挑战次数达到上限"
        );
        let frozen = self
            .frozen
            .for_next_challenge(Uuid::new_v4(), up_completed)?;
        let preparation = LostVoidPreparationRun::new(
            self.snapshot.config.clone(),
            frozen,
            self.control.clone(),
            1_800_000,
        )?;
        self.challenge = Some(
            LostVoidChallengeRun::new(
                preparation,
                self.options.clone(),
                7_200_000,
                self.control.clone(),
            )?
            .with_record_context(self.snapshot.instance_id, self.snapshot.region)?
            .with_attempt_limits(self.attempt_limits)?,
        );
        self.snapshot.challenges_started += 1;
        self.hub = None;
        self.bounty = None;
        self.returned_after_work = false;
        self.snapshot.phase = ApplicationPhase::Challenge;
        self.child_started = false;
        self.sync_children();
        Ok(())
    }
    fn start_hub(&mut self, sources: Vec<Uuid>) -> Result<()> {
        self.claim_sources = sources;
        self.hub = Some(LostVoidEntryRun::for_bounty(
            LostVoidEntryRequest {
                run_id: self.snapshot.run_id,
                config: self.snapshot.config.clone(),
                timeout_ms: 300_000,
            },
            self.control.clone(),
        )?);
        self.bounty = None;
        self.returned_after_work = false;
        self.snapshot.phase = ApplicationPhase::Hub;
        self.child_started = false;
        self.sync_children();
        Ok(())
    }
    fn plan_next(&mut self) -> Result<()> {
        let day =
            GameDay::at(self.snapshot.region, chrono::Utc::now()).map_err(anyhow::Error::msg)?;
        let planning = self
            .store
            .application_planning(self.snapshot.instance_id, &day)?;
        let decision = plan(&self.snapshot.config, &day, &planning.progress.history())
            .map_err(anyhow::Error::msg)?;
        self.snapshot.planning = Some(planning.clone());
        let must_claim = self.force_bounty
            || !planning.pending_claims.is_empty()
                && (self.snapshot.challenges_started == 0
                    || decision != LostVoidDecision::Run
                    || self.snapshot.failed_completion.is_some())
            || self.used_native
                && !self.returned_after_work
                && (decision != LostVoidDecision::Run || self.snapshot.failed_completion.is_some());
        if must_claim {
            return self.start_hub(planning.pending_claims);
        }
        if self.snapshot.failed_completion.is_some() || decision != LostVoidDecision::Run {
            return self.finish(planning, decision);
        }
        self.start_challenge(planning.progress.complete_task_force_with_up)
    }
    fn consume_challenge(&mut self) -> Result<()> {
        let child = self.challenge.as_ref().context("实际挑战执行器缺失")?;
        ensure!(child.uses_control(&self.control), "应用与挑战控制不一致");
        match &child.receipt().context("实际挑战尚未安全交回")?.outcome {
            ChallengeOutcome::BountyAlreadyComplete(_) => {
                self.force_bounty = true;
                self.snapshot.phase = ApplicationPhase::Plan;
            }
            ChallengeOutcome::Finished(receipt) => {
                let SettlementOutcome::Finished { completion, .. } = receipt.outcome else {
                    anyhow::bail!("应用不能记账中途结算")
                };
                let record = child.settlement_record()?;
                if let Some(old) = &self.snapshot.pending_settlement {
                    ensure!(old == &record, "暂停后实际结算记录发生变化");
                }
                self.snapshot.pending_settlement = Some(record);
                if completion != CompletionKind::Completed {
                    self.snapshot.failed_completion = Some(completion);
                    self.force_bounty = true;
                }
                self.snapshot.phase = ApplicationPhase::CommitChallenge;
            }
        }
        self.child_started = false;
        Ok(())
    }
    fn commit_challenge(&mut self) -> Result<()> {
        let record = self
            .snapshot
            .pending_settlement
            .as_ref()
            .context("应用待提交结算缺失")?;
        self.store.commit(record)?;
        if self.counted_settlements.insert(record.settlement_id()) {
            self.snapshot
                .committed_settlements
                .push(record.settlement_id());
            if self.snapshot.failed_completion.is_none() {
                self.snapshot.successful_challenges += 1;
            }
        }
        self.snapshot.pending_settlement = None;
        self.snapshot.phase = ApplicationPhase::Plan;
        Ok(())
    }
    fn consume_hub(&mut self) -> Result<()> {
        let hub = self.hub.as_ref().context("领取枢纽入口缺失")?;
        ensure!(
            hub.uses_control(&self.control)
                && hub.receipt().is_some_and(
                    |r| r.run_id == self.snapshot.run_id && r.outcome == EntryOutcome::HubReady
                ),
            "领取缺少本次实际枢纽交接"
        );
        self.bounty = Some(
            LostVoidBountyRun::new(
                BountyRequest {
                    run_id: self.snapshot.run_id,
                    claim_id: Uuid::new_v4(),
                    source_id: self
                        .claim_sources
                        .first()
                        .copied()
                        .unwrap_or(self.snapshot.run_id),
                    timeout_ms: 300_000,
                },
                self.frozen.scene_root(),
                self.control.clone(),
            )?
            .with_record_context(
                self.snapshot.instance_id,
                self.snapshot.region,
                self.claim_sources.clone(),
            )?,
        );
        self.snapshot.phase = ApplicationPhase::Bounty;
        self.child_started = false;
        self.sync_children();
        Ok(())
    }
    fn consume_bounty(&mut self) -> Result<()> {
        let bounty = self.bounty.as_ref().context("实际领取执行器缺失")?;
        ensure!(bounty.uses_control(&self.control), "领取控制与应用不一致");
        let record = bounty.claim_record()?;
        if let Some(old) = &self.snapshot.pending_claim {
            ensure!(old == &record, "暂停后领取完成记录发生变化");
        }
        self.snapshot.pending_claim = Some(record);
        self.snapshot.phase = ApplicationPhase::CommitClaim;
        self.child_started = false;
        Ok(())
    }
    fn commit_claim(&mut self) -> Result<()> {
        let record = self
            .snapshot
            .pending_claim
            .as_ref()
            .context("待提交领取记录缺失")?;
        self.store.acknowledge_claim(record)?;
        if self.counted_claims.insert(record.claim_id()) {
            ensure!(
                self.snapshot.completed_claim_checks < 1024,
                "领取批次数达到上限"
            );
            self.snapshot.completed_claim_checks += 1;
            self.snapshot.committed_claims.push(record.claim_id());
        }
        self.snapshot.pending_claim = None;
        self.force_bounty = false;
        self.returned_after_work = true;
        self.snapshot.phase = ApplicationPhase::Plan;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidApplicationEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidApplicationEnd::Paused,
            Interrupt::Stop => LostVoidApplicationEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidApplicationEnd::Paused) && self.snapshot.inputs_released,
            "应用尚未安全暂停"
        );
        use ApplicationPhase as P;
        if self.snapshot.phase == P::Done
            || self.snapshot.phase == P::Plan && self.returned_after_work
        {
            self.snapshot.phase = if self.used_native { P::Bounty } else { P::Plan };
            self.child_started = self.used_native;
        }
        match self.snapshot.phase {
            P::CommitChallenge => {
                self.snapshot.phase = P::Challenge;
                self.child_started = true;
            }
            P::CommitClaim => {
                self.snapshot.phase = P::Bounty;
                self.child_started = true;
            }
            _ => {}
        }
        if self.child_started {
            match self.snapshot.phase {
                P::Challenge => self
                    .challenge
                    .as_mut()
                    .context("暂停挑战缺失")?
                    .resume_after_parent_pause()?,
                P::Hub => self
                    .hub
                    .as_mut()
                    .context("暂停枢纽入口缺失")?
                    .resume_after_parent_pause()?,
                P::Bounty => self
                    .bounty
                    .as_mut()
                    .context("暂停领取执行器缺失")?
                    .resume_after_parent_pause()?,
                _ => anyhow::bail!("应用暂停阶段不支持子流程继续"),
            }
        } else {
            ensure!(self.control.resume(), "停止不能被继续覆盖");
        }
        self.snapshot.receipt = None;
        self.returned_after_work = false;
        self.end = None;
        self.sync_children();
        Ok(())
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidApplicationSnapshot) -> Result<()>,
    ) -> Result<LostVoidApplicationEnd> {
        use ApplicationPhase as P;
        use LostVoidApplicationEnd as E;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "应用不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget =
            Duration::from_millis(u64::from(self.snapshot.timeout_ms).saturating_sub(initial))
                .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<E> {
            if let Some(end) = self.interrupted() {
                return Ok(end);
            }
            if remaining().is_zero() {
                return Ok(E::TimedOut);
            }
            self.frozen.verify()?;
            session.check_ether_synthesis_target(game)?;
            session.release_inputs()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                if self.snapshot.phase == P::Done {
                    return Ok(E::Ready);
                }
                ensure!(self.steps < 8192, "应用交接步骤达到上限");
                self.steps += 1;
                self.frozen.verify()?;
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                match self.snapshot.phase {
                    P::Plan => self.plan_next()?,
                    P::CommitChallenge => self.commit_challenge()?,
                    P::CommitClaim => self.commit_claim()?,
                    P::Challenge | P::Hub | P::Bounty => {
                        self.used_native = true;
                        self.child_started = true;
                        self.snapshot.inputs_released = false;
                        if self.snapshot.phase == P::Hub {
                            session.load_lost_void_scene_bundle(self.frozen.scene_root())?;
                        }
                        let mut saved = self.snapshot();
                        macro_rules! save {
                            ($field:ident,$value:expr) => {{
                                let child = $value;
                                saved.inputs_released = child.inputs_released;
                                saved.$field = Some(child);
                                saved.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                publish(saved.clone())
                            }};
                        }
                        macro_rules! end {
                            ($value:expr,$kind:ident) => {
                                match $value {
                                    $kind::Ready => ChildEnd::Ready,
                                    $kind::Paused => ChildEnd::Paused,
                                    $kind::Stopped => ChildEnd::Stopped,
                                    $kind::TimedOut => ChildEnd::TimedOut,
                                    $kind::Failed => ChildEnd::Failed,
                                }
                            };
                        }
                        let child = match self.snapshot.phase {
                            P::Challenge => self
                                .challenge
                                .as_mut()
                                .context("当前挑战缺失")?
                                .execute_with_budget(session, game, remaining(), |child| {
                                    saved.inputs_released = child.inputs_released;
                                    saved.challenge = Some(Box::new(child));
                                    saved.active_ms = initial
                                        .saturating_add(started.elapsed().as_millis() as u64);
                                    publish(saved.clone())
                                })
                                .map(|e| end!(e, LostVoidChallengeEnd)),
                            P::Hub => self
                                .hub
                                .as_mut()
                                .context("当前枢纽入口缺失")?
                                .execute_with_budget(session, game, remaining(), |s| save!(hub, s))
                                .map(|e| end!(e, LostVoidEntryEnd)),
                            P::Bounty => self
                                .bounty
                                .as_mut()
                                .context("当前领取执行器缺失")?
                                .execute_with_budget(session, game, remaining(), |s| {
                                    save!(bounty, s)
                                })
                                .map(|e| end!(e, LostVoidBountyEnd)),
                            _ => unreachable!(),
                        };
                        self.snapshot.inputs_released = session.inputs_released();
                        self.sync_children();
                        let child = child?;
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(E::TimedOut);
                        }
                        match child {
                            ChildEnd::Ready => match self.snapshot.phase {
                                P::Challenge => self.consume_challenge()?,
                                P::Hub => self.consume_hub()?,
                                P::Bounty => self.consume_bounty()?,
                                _ => unreachable!(),
                            },
                            ChildEnd::Paused => return Ok(E::Paused),
                            ChildEnd::Stopped => return Ok(E::Stopped),
                            ChildEnd::TimedOut => return Ok(E::TimedOut),
                            ChildEnd::Failed => return Ok(E::Failed),
                        }
                    }
                    P::Done => unreachable!(),
                }
                self.sync_children();
                self.snapshot.inputs_released = session.inputs_released();
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot())?;
            }
        })();
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() || !self.snapshot.inputs_released {
            self.end = Some(E::Failed);
        }
        ensure!(self.snapshot.inputs_released, "应用收尾无法确认输入释放");
        let mut end = result?;
        self.frozen
            .verify()
            .inspect_err(|_| self.end = Some(E::Failed))?;
        if let Err(error) = publish(self.snapshot()) {
            self.end = Some(E::Failed);
            return Err(error);
        }
        self.frozen
            .verify()
            .inspect_err(|_| self.end = Some(E::Failed))?;
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = E::TimedOut;
        }
        if end == E::Paused && !self.control.acknowledge_pause() {
            end = E::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
