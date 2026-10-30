//! 单次挑战：完整准备、多个层及结算共用控制和总预算；日周记账留给应用父任务。
use crate::{
    lost_void_level::{LevelOutcome, LostVoidLevelEnd, LostVoidLevelRun, LostVoidLevelSnapshot},
    lost_void_preparation::{
        LostVoidPreparationEnd, LostVoidPreparationReceipt, LostVoidPreparationRun,
        LostVoidPreparationSnapshot,
    },
    lost_void_recovery::{
        LostVoidRecoveryEnd, LostVoidRecoveryRun, LostVoidRecoverySnapshot, LostVoidRecoveryStart,
    },
    lost_void_settlement::{
        LostVoidSettlementEnd, LostVoidSettlementRun, LostVoidSettlementSnapshot,
    },
};
use anyhow::{Context, Result, ensure};
use maa_adapter::NativeSession;
use od_core::{
    lost_void::{ArtifactSelection, LostVoidMission, choose_artifacts},
    lost_void_entry::LostVoidEntryReceipt,
    lost_void_settlement::{SettlementOutcome, SettlementReceipt},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengePhase {
    Preparation,
    Level,
    Recovery,
    Settlement,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidChallengeEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidRecordContext {
    pub instance_id: u32,
    pub region: od_core::daily::GameRegion,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "receipt",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ChallengeOutcome {
    Finished(Box<SettlementReceipt>),
    BountyAlreadyComplete(Box<LostVoidEntryReceipt>),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidChallengeReceipt {
    pub run_id: Uuid,
    pub mission: LostVoidMission,
    pub levels: u16,
    pub outcome: ChallengeOutcome,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidChallengeSnapshot {
    pub run_id: Uuid,
    pub phase: ChallengePhase,
    pub levels: u16,
    pub active_ms: u64,
    pub timeout_ms: u32,
    pub inputs_released: bool,
    pub attempt_limits: crate::lost_void_level::LostVoidAttemptLimits,
    pub record_context: Option<LostVoidRecordContext>,
    pub preparation: LostVoidPreparationSnapshot,
    pub level: Option<LostVoidLevelSnapshot>,
    pub recovery: Option<LostVoidRecoverySnapshot>,
    pub settlement: Option<LostVoidSettlementSnapshot>,
    pub receipt: Option<LostVoidChallengeReceipt>,
}
pub struct LostVoidChallengeRun {
    preparation: LostVoidPreparationRun,
    level: Option<LostVoidLevelRun>,
    recovery: Option<LostVoidRecoveryRun>,
    settlement: Option<LostVoidSettlementRun>,
    options: ArtifactSelection,
    attempt_limits: crate::lost_void_level::LostVoidAttemptLimits,
    record_context: Option<LostVoidRecordContext>,
    control: PipelineControl,
    timeout_ms: u32,
    active_ms: u64,
    inputs_released: bool,
    phase: ChallengePhase,
    completion_phase: Option<ChallengePhase>,
    levels: u16,
    child_started: bool,
    end: Option<LostVoidChallengeEnd>,
    receipt: Option<LostVoidChallengeReceipt>,
    steps: u16,
}
enum ChildEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
impl LostVoidChallengeRun {
    pub fn new(
        preparation: LostVoidPreparationRun,
        options: ArtifactSelection,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(
            preparation.uses_control(&control) && control.requested().is_none(),
            "单次挑战与准备须共用正常控制"
        );
        ensure!(
            preparation.pristine() || preparation.receipt().is_some(),
            "单次挑战不能采用运行中或失败的准备执行器"
        );
        ensure!(
            (1000..=7_200_000).contains(&timeout_ms),
            "单次挑战总预算无效"
        );
        ensure!(
            preparation.frozen().npc_detector_root().is_some(),
            "单次挑战须在准备输入前冻结导航资源"
        );
        preparation.frozen().verify()?;
        choose_artifacts(&preparation.frozen().seed().challenge, &[], 0, &options)
            .map_err(anyhow::Error::msg)?;
        let active_ms = preparation.snapshot().active_ms;
        let ready = preparation.receipt().is_some();
        let mut run = Self {
            preparation,
            level: None,
            recovery: None,
            settlement: None,
            options,
            attempt_limits: Default::default(),
            record_context: None,
            control,
            timeout_ms,
            active_ms,
            inputs_released: true,
            phase: ChallengePhase::Preparation,
            completion_phase: None,
            levels: 0,
            child_started: ready,
            end: None,
            receipt: None,
            steps: 0,
        };
        if ready {
            run.consume_preparation()?;
        }
        Ok(run)
    }
    pub fn snapshot(&self) -> LostVoidChallengeSnapshot {
        LostVoidChallengeSnapshot {
            run_id: self.preparation.frozen().seed().run_id,
            phase: self.phase,
            levels: self.levels,
            active_ms: self.active_ms,
            timeout_ms: self.timeout_ms,
            inputs_released: self.inputs_released,
            attempt_limits: self.attempt_limits,
            record_context: self.record_context,
            preparation: self.preparation.snapshot(),
            level: self.level.as_ref().map(|l| l.snapshot()),
            recovery: self.recovery.as_ref().map(|r| r.snapshot()),
            settlement: self.settlement.as_ref().map(|s| s.snapshot()),
            receipt: self.receipt.clone(),
        }
    }
    pub fn uses_control(&self, c: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &c.cancellation())
    }
    pub fn with_attempt_limits(
        mut self,
        limits: crate::lost_void_level::LostVoidAttemptLimits,
    ) -> Result<Self> {
        ensure!(
            self.phase == ChallengePhase::Preparation
                && self.preparation.pristine()
                && self.active_ms == 0
                && !self.child_started,
            "准备开始后不能更换步骤预算"
        );
        limits.validate()?;
        self.attempt_limits = limits;
        Ok(self)
    }
    pub fn receipt(&self) -> Option<&LostVoidChallengeReceipt> {
        (self.end == Some(LostVoidChallengeEnd::Ready)
            && self.inputs_released
            && self.control.requested().is_none())
        .then_some(self.receipt.as_ref())
        .flatten()
    }
    pub fn with_record_context(
        mut self,
        instance_id: u32,
        region: od_core::daily::GameRegion,
    ) -> Result<Self> {
        ensure!(
            (1..=9999).contains(&instance_id)
                && self.preparation.pristine()
                && self.phase == ChallengePhase::Preparation
                && self.active_ms == 0
                && !self.child_started
                && self.end.is_none()
                && self.record_context.is_none(),
            "记账账号须在首次准备输入前冻结一次"
        );
        self.record_context = Some(LostVoidRecordContext {
            instance_id,
            region,
        });
        Ok(self)
    }
    pub fn settlement_record(&self) -> Result<od_core::lost_void_record::LostVoidSettlementRecord> {
        use od_core::{
            daily::GameDay,
            lost_void_record::{LostVoidSettlementRecord, LostVoidUpCompletion},
            lost_void_settlement::CompletionKind,
        };
        let context = self
            .record_context
            .context("本次挑战没有输入前冻结的记账账号")?;
        let ChallengeOutcome::Finished(receipt) = &self
            .receipt()
            .context("挑战尚未安全交回，不能记账")?
            .outcome
        else {
            anyhow::bail!("悬赏已满入口不能提交挑战通关记录");
        };
        let settlement = self.settlement.as_ref().context("实际最终结算执行器缺失")?;
        settlement.verify_frozen(self.preparation.frozen())?;
        ensure!(
            settlement.receipt() == Some(receipt.as_ref()),
            "挑战最终回执与实际结算不一致"
        );
        let at = settlement
            .finished_at()
            .context("实际最终结算缺少完成时刻")?;
        let day = GameDay::at(context.region, at).map_err(anyhow::Error::msg)?;
        let snapshot = settlement.snapshot();
        let up = if self.preparation.frozen().seed().mission == LostVoidMission::TaskForce
            && matches!(
                receipt.outcome,
                SettlementOutcome::Finished {
                    completion: CompletionKind::Completed,
                    ..
                }
            ) {
            self.preparation
                .actual_up_observation()
                .and_then(|observation| {
                    let up = LostVoidUpCompletion {
                        observed_day: GameDay::at(context.region, observation.started_at).ok()?,
                        up_agents: observation.agents.try_into().ok()?,
                    };
                    let end_day = GameDay::at(context.region, observation.observed_at).ok()?;
                    let end = LostVoidUpCompletion {
                        observed_day: end_day,
                        ..up.clone()
                    };
                    let members = snapshot.flow.request().members.as_deref()?;
                    (up.matches_party_in_week(&day, members)
                        && end.matches_party_in_week(&day, members))
                    .then_some(up)
                })
        } else {
            None
        };
        let record =
            LostVoidSettlementRecord::from_settlement(context.instance_id, day, &snapshot.flow, up)
                .map_err(anyhow::Error::msg)?;
        ensure!(
            self.receipt().is_some() && settlement.receipt() == Some(receipt.as_ref()),
            "记账证据交回前控制或实际结算状态变化"
        );
        Ok(record)
    }
    fn complete(&mut self, outcome: ChallengeOutcome) {
        self.receipt = Some(LostVoidChallengeReceipt {
            run_id: self.preparation.frozen().seed().run_id,
            mission: self.preparation.frozen().seed().mission,
            levels: self.levels,
            outcome,
        });
        self.completion_phase = Some(self.phase);
        self.phase = ChallengePhase::Done;
    }
    fn consume_preparation(&mut self) -> Result<()> {
        match self.preparation.receipt().context("准备尚未安全交回")? {
            LostVoidPreparationReceipt::BountyAlreadyComplete { entry } => self.complete(
                ChallengeOutcome::BountyAlreadyComplete(Box::new(entry.clone())),
            ),
            LostVoidPreparationReceipt::Entered { .. } => {
                let level = LostVoidLevelRun::after_preparation(
                    &self.preparation,
                    self.options.clone(),
                    1_800_000,
                    self.control.clone(),
                )?;
                self.level = Some(level.with_attempt_limits(self.attempt_limits)?);
                self.levels = 1;
                self.phase = ChallengePhase::Level;
                self.child_started = false;
            }
        }
        Ok(())
    }
    fn consume(&mut self) -> Result<()> {
        match self.phase {
            ChallengePhase::Preparation => self.consume_preparation()?,
            ChallengePhase::Level => {
                let level = self.level.as_ref().context("单次挑战当前层缺失")?;
                match &level.receipt().context("当前层尚未安全交回")?.outcome {
                    LevelOutcome::NextLevel { .. } => {
                        ensure!(self.levels < 1024, "单次挑战层数达到上限");
                        let next = level.next_level(
                            self.preparation.frozen(),
                            1_800_000,
                            self.control.clone(),
                        )?;
                        self.level = Some(next.with_attempt_limits(self.attempt_limits)?);
                        self.levels += 1;
                        self.child_started = false;
                    }
                    LevelOutcome::AgentDead { .. } => self.start_recovery()?,
                    _ => {
                        let settlement = LostVoidSettlementRun::after_level(
                            self.preparation.frozen(),
                            level,
                            300_000,
                            self.control.clone(),
                        )?;
                        self.settlement = Some(settlement);
                        self.phase = ChallengePhase::Settlement;
                        self.child_started = false;
                    }
                }
            }
            ChallengePhase::Recovery => {
                let recovery = self.recovery.as_mut().context("实际恢复执行器缺失")?;
                match &recovery.receipt().context("恢复尚未安全交回")?.outcome {
                    od_core::lost_void_recovery_menu::RecoveryOutcome::Restarted { .. } => {
                        self.level = Some(LostVoidLevelRun::after_recovery(
                            self.preparation.frozen(),
                            recovery,
                            1_800_000,
                            self.control.clone(),
                        )?);
                        self.phase = ChallengePhase::Level;
                    }
                    od_core::lost_void_recovery_menu::RecoveryOutcome::ObservedResult {
                        ..
                    } => {
                        self.settlement = Some(LostVoidSettlementRun::after_recovery_result(
                            self.preparation.frozen(),
                            recovery,
                            300_000,
                            self.control.clone(),
                        )?);
                        self.phase = ChallengePhase::Settlement;
                    }
                }
                self.child_started = false;
            }
            ChallengePhase::Settlement => {
                let settlement = self.settlement.as_ref().context("单次挑战结算执行器缺失")?;
                let receipt = settlement.receipt().context("结算尚未安全交回")?;
                match &receipt.outcome {
                    SettlementOutcome::Continue { .. } => {
                        ensure!(self.levels < 1024, "单次挑战层数达到上限");
                        let level = LostVoidLevelRun::after_settlement(
                            self.preparation.frozen(),
                            settlement,
                            1_800_000,
                            self.control.clone(),
                        )?;
                        self.options = receipt.options.clone();
                        self.level = Some(level.with_attempt_limits(self.attempt_limits)?);
                        self.levels += 1;
                        self.phase = ChallengePhase::Level;
                        self.child_started = false;
                    }
                    SettlementOutcome::Finished { .. } => {
                        self.complete(ChallengeOutcome::Finished(Box::new(receipt.clone())))
                    }
                }
            }
            ChallengePhase::Done => anyhow::bail!("已完成挑战不能再消费子结果"),
        }
        Ok(())
    }
    fn start_recovery(&mut self) -> Result<()> {
        let start = LostVoidRecoveryRun::after_level(
            self.preparation.frozen(),
            self.level.as_mut().context("实际失败层缺失")?,
            120_000,
            self.control.clone(),
        )?;
        match start {
            LostVoidRecoveryStart::FindExit(level) => {
                self.level = Some(*level);
                self.phase = ChallengePhase::Level;
            }
            LostVoidRecoveryStart::Menu(recovery) => {
                self.recovery = Some(*recovery);
                self.phase = ChallengePhase::Recovery;
            }
        }
        self.child_started = false;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidChallengeEnd> {
        self.control.requested().map(|v| match v {
            Interrupt::Pause => LostVoidChallengeEnd::Paused,
            Interrupt::Stop => LostVoidChallengeEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidChallengeEnd::Paused),
            "单次挑战尚未安全暂停"
        );
        self.resume_after_parent_pause()
    }
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidChallengeEnd::Paused | LostVoidChallengeEnd::Ready)
            ) && self.inputs_released,
            "单次挑战尚未释放输入"
        );
        if self.phase == ChallengePhase::Done {
            self.phase = self.completion_phase.context("挑战终态缺少实际来源")?;
        }
        if self.child_started {
            match self.phase {
                ChallengePhase::Preparation => self.preparation.resume_after_parent_pause()?,
                ChallengePhase::Level => self
                    .level
                    .as_mut()
                    .context("挑战层缺失")?
                    .resume_after_parent_pause()?,
                ChallengePhase::Recovery => self
                    .recovery
                    .as_mut()
                    .context("实际恢复执行器缺失")?
                    .resume_after_parent_pause()?,
                ChallengePhase::Settlement => self
                    .settlement
                    .as_mut()
                    .context("挑战结算缺失")?
                    .resume_after_parent_pause()?,
                ChallengePhase::Done => unreachable!(),
            }
        } else {
            ensure!(self.control.resume(), "停止不能被继续覆盖");
        }
        self.receipt = None;
        self.end = None;
        Ok(())
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidChallengeSnapshot) -> Result<()>,
    ) -> Result<LostVoidChallengeEnd> {
        use LostVoidChallengeEnd as E;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "单次挑战不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.active_ms;
        let budget = Duration::from_millis(u64::from(self.timeout_ms).saturating_sub(initial))
            .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<E> {
            if let Some(end) = self.interrupted() {
                return Ok(end);
            }
            if remaining().is_zero() {
                return Ok(E::TimedOut);
            }
            self.preparation.frozen().verify()?;
            session.check_ether_synthesis_target(game)?;
            session.release_inputs()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                if self.phase == ChallengePhase::Done {
                    return Ok(E::Ready);
                }
                ensure!(self.steps < 4096, "单次挑战交接达到上限");
                self.steps += 1;
                self.child_started = true;
                self.inputs_released = false;
                let mut saved = self.snapshot();
                macro_rules! save {
                    ($field:ident,$snapshot:expr) => {{
                        let child = $snapshot;
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
                let child = match self.phase {
                    ChallengePhase::Preparation => self
                        .preparation
                        .execute_with_budget(session, game, remaining(), |child| {
                            saved.inputs_released = child.inputs_released;
                            saved.preparation = child;
                            saved.active_ms =
                                initial.saturating_add(started.elapsed().as_millis() as u64);
                            publish(saved.clone())
                        })
                        .map(|e| end!(e, LostVoidPreparationEnd)),
                    ChallengePhase::Level => self
                        .level
                        .as_mut()
                        .context("挑战层缺失")?
                        .execute_with_budget(
                            self.preparation.frozen(),
                            session,
                            game,
                            remaining(),
                            |s| save!(level, s),
                        )
                        .map(|e| end!(e, LostVoidLevelEnd)),
                    ChallengePhase::Recovery => self
                        .recovery
                        .as_mut()
                        .context("实际恢复执行器缺失")?
                        .execute_with_budget(
                            self.preparation.frozen(),
                            session,
                            game,
                            remaining(),
                            |s| save!(recovery, s),
                        )
                        .map(|e| end!(e, LostVoidRecoveryEnd)),
                    ChallengePhase::Settlement => self
                        .settlement
                        .as_mut()
                        .context("挑战结算缺失")?
                        .execute_with_budget(
                            self.preparation.frozen(),
                            session,
                            game,
                            remaining(),
                            |s| save!(settlement, s),
                        )
                        .map(|e| end!(e, LostVoidSettlementEnd)),
                    ChallengePhase::Done => unreachable!(),
                };
                self.inputs_released = session.inputs_released();
                self.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                match child? {
                    ChildEnd::Ready => {
                        if let Err(error) = self.consume() {
                            if let Some(end) = self.interrupted() {
                                return Ok(end);
                            }
                            return Err(error);
                        }
                    }
                    ChildEnd::Paused => return Ok(E::Paused),
                    ChildEnd::Stopped => return Ok(E::Stopped),
                    ChildEnd::TimedOut => {
                        if self.phase == ChallengePhase::Level
                            && self
                                .level
                                .as_ref()
                                .is_some_and(|l| l.recovery_source().is_ok())
                        {
                            self.start_recovery()?;
                        } else {
                            return Ok(E::TimedOut);
                        }
                    }
                    ChildEnd::Failed => return Ok(E::Failed),
                }
                publish(self.snapshot())?;
            }
        })();
        self.inputs_released = session.release_inputs().is_ok();
        self.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() || !self.inputs_released {
            self.end = Some(E::Failed);
        }
        ensure!(self.inputs_released, "单次挑战收尾输入未释放");
        let mut end = result?;
        self.preparation
            .frozen()
            .verify()
            .inspect_err(|_| self.end = Some(E::Failed))?;
        if let Err(error) = publish(self.snapshot()) {
            self.end = Some(E::Failed);
            return Err(error);
        }
        self.preparation
            .frozen()
            .verify()
            .inspect_err(|_| self.end = Some(E::Failed))?;
        self.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
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
