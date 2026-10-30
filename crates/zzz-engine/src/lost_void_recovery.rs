//! 仅由实际层失败创建的恢复菜单执行器；真实加载及到达后才交接新尝试。
use crate::{
    lost_void_combat::{FrozenLostVoidPreparation, LostVoidCombatSeed},
    lost_void_level::LostVoidLevelRun,
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{
    NativeSession, combat_vision::CombatRecognizer, lost_void_recovery::RecoveryInputEnd,
    lost_void_roster::LostVoidHudReader,
};
use od_core::{
    lost_void::ArtifactSelection,
    lost_void_npc::NpcMemory,
    lost_void_recovery::{LayerFailure, LayerRetryBudget, RetryDecision},
    lost_void_recovery_menu::*,
    lost_void_roster::RosterMember,
    lost_void_team_plan::LostVoidTeamPlan,
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct RecoverySource {
    pub limits: crate::lost_void_level::LostVoidAttemptLimits,
    pub failure: LayerFailure,
    pub memory: NpcMemory,
    pub plan: LostVoidTeamPlan,
    pub members: Vec<RosterMember>,
    pub options: ArtifactSelection,
    pub initial_options: ArtifactSelection,
    pub budget: LayerRetryBudget,
}
pub enum LostVoidRecoveryStart {
    FindExit(Box<LostVoidLevelRun>),
    Menu(Box<LostVoidRecoveryRun>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidRecoveryEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidRecoverySnapshot {
    pub flow: RecoveryFlow,
    pub failure: LayerFailure,
    pub timeout_ms: u32,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct LostVoidRecoveryRun {
    snapshot: LostVoidRecoverySnapshot,
    source: RecoverySource,
    seed: LostVoidCombatSeed,
    scene: PathBuf,
    combat: PathBuf,
    detector: PathBuf,
    control: PipelineControl,
    recognizer: Option<CombatRecognizer>,
    reader: Option<LostVoidHudReader>,
    handoff_consumed: bool,
    end: Option<LostVoidRecoveryEnd>,
    steps: u32,
}
impl LostVoidRecoveryRun {
    pub fn after_level(
        frozen: &FrozenLostVoidPreparation,
        level: &mut LostVoidLevelRun,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<LostVoidRecoveryStart> {
        ensure!(
            level.uses_control(&control) && control.requested().is_none(),
            "恢复与实际失败须共用正常控制"
        );
        ensure!((1000..=300_000).contains(&timeout_ms), "恢复预算无效");
        level.verify_frozen(frozen)?;
        let mut source = level.recovery_source()?;
        let decision = source
            .budget
            .begin(source.failure.clone(), &source.memory, Uuid::new_v4())
            .map_err(anyhow::Error::msg)?;
        let start = match decision {
            RetryDecision::FindExit => LostVoidRecoveryStart::FindExit(Box::new(
                LostVoidLevelRun::after_find_exit(frozen, &source, 1_800_000, control)?,
            )),
            RetryDecision::Restart(_) | RetryDecision::Exit => {
                let flow = RecoveryFlow::new(RecoveryRequest {
                    recovery_id: Uuid::new_v4(),
                    budget: source.budget.clone(),
                    plan: source.plan.clone(),
                    members: source.members.clone(),
                })
                .map_err(anyhow::Error::msg)?;
                LostVoidRecoveryStart::Menu(Box::new(Self {
                    snapshot: LostVoidRecoverySnapshot {
                        flow,
                        failure: source.failure.clone(),
                        timeout_ms,
                        active_ms: 0,
                        inputs_released: true,
                    },
                    source,
                    seed: frozen.seed().clone(),
                    scene: frozen.scene_root().into(),
                    combat: frozen.combat_root().into(),
                    detector: frozen
                        .npc_detector_root()
                        .context("恢复缺少首次输入前冻结的导航资源")?
                        .into(),
                    control,
                    recognizer: None,
                    reader: None,
                    handoff_consumed: false,
                    end: None,
                    steps: 0,
                }))
            }
            RetryDecision::AlreadyHandled => anyhow::bail!("当前失败已经处理"),
        };
        level.reserve_recovery();
        Ok(start)
    }
    pub fn snapshot(&self) -> LostVoidRecoverySnapshot {
        self.snapshot.clone()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn receipt(&self) -> Option<&RecoveryReceipt> {
        (self.end == Some(LostVoidRecoveryEnd::Ready)
            && self.snapshot.inputs_released
            && !self.handoff_consumed
            && self.control.requested().is_none())
        .then(|| self.snapshot.flow.receipt())
        .flatten()
    }
    pub(crate) fn source(&self) -> &RecoverySource {
        &self.source
    }
    pub(crate) fn consume_handoff(&mut self) {
        self.handoff_consumed = true;
    }
    pub(crate) fn verify_frozen(&self, frozen: &FrozenLostVoidPreparation) -> Result<()> {
        ensure!(
            frozen.seed() == &self.seed
                && frozen.scene_root() == self.scene
                && frozen.combat_root() == self.combat
                && frozen.npc_detector_root() == Some(self.detector.as_path()),
            "恢复菜单冻结配置或路径变化"
        );
        frozen.verify()
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidRecoveryEnd::Paused),
            "恢复菜单尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidRecoveryEnd::Paused | LostVoidRecoveryEnd::Ready)
            ) && self.snapshot.inputs_released
                && !self.handoff_consumed,
            "恢复菜单尚未安全暂停"
        );
        self.snapshot.flow.resume().map_err(anyhow::Error::msg)?;
        if let Some(reader) = &mut self.reader {
            reader.reset();
        }
        ensure!(self.control.resume(), "停止不能被继续覆盖");
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidRecoveryEnd> {
        self.control.requested().map(|v| match v {
            Interrupt::Pause => LostVoidRecoveryEnd::Paused,
            Interrupt::Stop => LostVoidRecoveryEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        frozen: &FrozenLostVoidPreparation,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidRecoverySnapshot) -> Result<()>,
    ) -> Result<LostVoidRecoveryEnd> {
        use LostVoidRecoveryEnd as E;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "恢复菜单不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
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
            self.verify_frozen(frozen)?;
            session.check_ether_synthesis_target(game)?;
            session.release_inputs()?;
            if self.recognizer.is_none() {
                session.load_lost_void_scene_bundle(&self.scene)?;
                session.load_challenge_detector(&self.detector)?;
                self.recognizer = Some(session.load_combat_bundle(&self.combat)?);
            }
            if self.reader.is_none() {
                self.reader = Some(LostVoidHudReader::new(self.recognizer.as_ref().unwrap())?);
            }
            session.require_lost_void_battle_roots(&self.scene, &self.combat, &self.detector)?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(E::Ready);
                }
                ensure!(self.steps < 4096, "恢复菜单观察达到上限");
                self.steps += 1;
                let read = session.read_lost_void_recovery(
                    &self.snapshot.flow,
                    self.reader.as_mut().unwrap(),
                    self.recognizer.as_ref().unwrap(),
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "恢复菜单观察后输入未释放");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                let Some(view) = read? else {
                    self.snapshot.flow.invalidate_observation();
                    continue;
                };
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                let action = self.snapshot.flow.next(&view).map_err(anyhow::Error::msg)?;
                publish(self.snapshot())?;
                let Some(action) = action else {
                    std::thread::sleep(Duration::from_millis(30));
                    continue;
                };
                let before = self.snapshot.flow.clone();
                self.snapshot
                    .flow
                    .prepare(action)
                    .map_err(anyhow::Error::msg)?;
                if let Err(error) = publish(self.snapshot()) {
                    self.snapshot.flow = before;
                    return Err(error);
                }
                if self.interrupted().is_some() || remaining().is_zero() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    return Ok(self.interrupted().unwrap_or(E::TimedOut));
                }
                self.verify_frozen(frozen)?;
                if self.interrupted().is_some() || remaining().is_zero() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    return Ok(self.interrupted().unwrap_or(E::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_lost_void_recovery(
                    &self.snapshot.flow,
                    action,
                    self.recognizer.as_ref().unwrap(),
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "恢复菜单输入未释放");
                match input {
                    Ok(RecoveryInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(RecoveryInputEnd::Changed | RecoveryInputEnd::NotPosted) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(RecoveryInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(E::TimedOut);
                    }
                    Ok(RecoveryInputEnd::Unknown) => {
                        self.snapshot
                            .flow
                            .unknown(action)
                            .map_err(anyhow::Error::msg)?;
                        if self.control.requested().is_none() {
                            self.control.pause();
                        }
                        return Ok(self.interrupted().unwrap_or(E::Paused));
                    }
                    Err(error) => {
                        self.snapshot
                            .flow
                            .unknown(action)
                            .map_err(anyhow::Error::msg)?;
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        return Err(error);
                    }
                }
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
        ensure!(self.snapshot.inputs_released, "恢复菜单收尾输入未释放");
        let mut end = result?;
        self.verify_frozen(frozen)
            .inspect_err(|_| self.end = Some(E::Failed))?;
        if let Err(error) = publish(self.snapshot()) {
            self.end = Some(E::Failed);
            return Err(error);
        }
        self.verify_frozen(frozen)
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
