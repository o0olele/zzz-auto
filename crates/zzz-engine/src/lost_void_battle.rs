//! 同次实际队伍、导航遭遇与冻结战斗段的局内交接。
use crate::{
    combat::{
        CombatFrameGate, CombatGuardOptions, CombatResultPolicy, CombatRun, GuardedCombatSegmentEnd,
    },
    lost_void_combat::{FrozenLostVoidPreparation, LostVoidCombatSeed},
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{
    NativeSession,
    combat_vision::CombatRecognizer,
    lost_void_battle::{BattleRead, LostVoidBattleWatcher},
};
use od_core::{
    combat::CombatProgress,
    lost_void_battle::{BattleFlow, BattleGate, BattleReceipt, BattleRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidBattleEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidBattleSnapshot {
    pub flow: BattleFlow,
    pub combat: CombatProgress,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct LostVoidBattleRun {
    snapshot: LostVoidBattleSnapshot,
    segment: CombatRun,
    control: PipelineControl,
    seed: LostVoidCombatSeed,
    scene: PathBuf,
    combat: PathBuf,
    detector: PathBuf,
    recognizer: Option<CombatRecognizer>,
    end: Option<LostVoidBattleEnd>,
}
impl LostVoidBattleRun {
    pub(crate) fn own_timeout(&self) -> bool {
        self.end == Some(LostVoidBattleEnd::TimedOut)
            && self.snapshot.inputs_released
            && self.control.requested().is_none()
            && self.snapshot.active_ms >= u64::from(self.snapshot.flow.request().timeout_ms)
            && !self.snapshot.flow.completed()
    }
    // 生产构造只能经冻结准备的实际执行器交接。
    pub(crate) fn new(
        frozen: &FrozenLostVoidPreparation,
        request: BattleRequest,
        segment: CombatRun,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(
            segment.id == request.roster.run_id,
            "战斗段与实际队伍运行身份不一致"
        );
        let flow = BattleFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: LostVoidBattleSnapshot {
                flow,
                combat: segment.progress.clone(),
                active_ms: 0,
                inputs_released: true,
            },
            segment,
            control,
            seed: frozen.seed().clone(),
            scene: frozen.scene_root().into(),
            combat: frozen.combat_root().into(),
            detector: frozen
                .npc_detector_root()
                .context("局内战斗缺少首次输入前冻结的导航模型")?
                .into(),
            recognizer: None,
            end: None,
        })
    }
    pub fn snapshot(&self) -> LostVoidBattleSnapshot {
        self.snapshot.clone()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn receipt(&self) -> Option<&BattleReceipt> {
        (self.end == Some(LostVoidBattleEnd::Ready)
            && self.snapshot.inputs_released
            && self.control.requested().is_none())
        .then(|| self.snapshot.flow.receipt())
        .flatten()
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidBattleEnd::Paused),
            "局内战斗尚未安全暂停"
        );
        self.resume_after_parent_pause()
    }
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidBattleEnd::Paused | LostVoidBattleEnd::Ready)
            ) && self.snapshot.inputs_released,
            "局内战斗尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        // 子战斗已经恢复同一父控制，不能再对已运行的句柄重复 resume。
        self.segment.resume_segment_after_parent_pause()?;
        self.snapshot.flow = flow;
        self.snapshot.combat = self.segment.progress.clone();
        self.recognizer = None;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidBattleEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidBattleEnd::Paused,
            Interrupt::Stop => LostVoidBattleEnd::Stopped,
        })
    }
    fn verify_frozen(&self, frozen: &FrozenLostVoidPreparation) -> Result<()> {
        ensure!(
            frozen.seed() == &self.seed
                && frozen.scene_root() == self.scene
                && frozen.combat_root() == self.combat
                && frozen.npc_detector_root() == Some(self.detector.as_path()),
            "局内战斗父准备身份或资源路径已变化"
        );
        frozen.verify()
    }
    pub fn execute_with_budget(
        &mut self,
        frozen: &FrozenLostVoidPreparation,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidBattleSnapshot) -> Result<()>,
    ) -> Result<LostVoidBattleEnd> {
        use LostVoidBattleEnd as E;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "局内战斗不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
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
            session.require_lost_void_battle_roots(&self.scene, &self.combat, &self.detector)?;
            let recognizer = self.recognizer.as_mut().unwrap();
            session.require_lost_void_navigation(recognizer)?;
            let ids: Vec<_> = self
                .snapshot
                .flow
                .request()
                .roster
                .members
                .iter()
                .map(|m| m.id.clone())
                .collect();
            recognizer.set_agent_scope(Some(&ids))?;
            let mut watcher = LostVoidBattleWatcher::new(
                recognizer,
                &ids,
                &self.snapshot.flow.request().region,
                self.segment.progress.config.max_frame_age_ms,
                self.snapshot.flow.last_probe_sequence(),
            )?;
            if self.control.requested().is_some() || remaining().is_zero() {
                return Ok(self.interrupted().unwrap_or(E::TimedOut));
            }
            let saved = RefCell::new(&mut self.snapshot);
            let publisher = RefCell::new(&mut publish);
            let cancel = self.control.cancellation();
            let end = self.segment.execute_segment_guarded_with_budget(
                session,
                recognizer,
                game,
                CombatGuardOptions {
                    budget: remaining(),
                    result_policy: CombatResultPolicy::ExternalBoundary,
                },
                |native, recognizer, frame, budget| {
                    let read = native.inspect_lost_void_battle(
                        frame,
                        recognizer,
                        &mut watcher,
                        &cancel,
                        budget,
                    )?;
                    let mut state = saved.borrow_mut();
                    let gate = match read {
                        BattleRead::Release => CombatFrameGate::Refresh,
                        BattleRead::Unavailable => {
                            state.flow.invalidate_observation();
                            CombatFrameGate::Refresh
                        }
                        BattleRead::View(view) => {
                            match state.flow.next(&view).map_err(anyhow::Error::msg)? {
                                BattleGate::Hold
                                    if view.page
                                        == od_core::lost_void_battle::BattlePage::CombatHud =>
                                {
                                    CombatFrameGate::Refresh
                                }
                                BattleGate::Hold => CombatFrameGate::Hold,
                                BattleGate::Proceed => CombatFrameGate::Proceed,
                                BattleGate::Yield => CombatFrameGate::Yield,
                            }
                        }
                    };
                    state.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
                    state.inputs_released = native.inputs_released();
                    publisher.borrow_mut()((**state).clone())?;
                    Ok(gate)
                },
                |progress| {
                    let mut state = saved.borrow_mut();
                    state.inputs_released = progress.inputs_released;
                    state.combat = progress;
                    state.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
                    publisher.borrow_mut()((**state).clone())
                },
            )?;
            match end {
                GuardedCombatSegmentEnd::Yielded => {
                    ensure!(
                        self.snapshot.flow.receipt().is_some(),
                        "战斗守卫缺少本次页面回执"
                    );
                    Ok(E::Ready)
                }
                GuardedCombatSegmentEnd::Interrupted(Interrupt::Pause) => Ok(E::Paused),
                GuardedCombatSegmentEnd::Interrupted(Interrupt::Stop) => Ok(E::Stopped),
                GuardedCombatSegmentEnd::Result(_) => anyhow::bail!("局内战斗不能采用普通结算回执"),
            }
        })();
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        self.snapshot.combat = self.segment.progress.clone();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() || !self.snapshot.inputs_released {
            self.end = Some(E::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "局内战斗收尾无法确认输入释放"
        );
        let mut end = match result {
            Ok(end) => end,
            Err(_) if remaining().is_zero() => E::TimedOut,
            Err(error) => return Err(error),
        };
        self.verify_frozen(frozen).inspect_err(|_| {
            self.end = Some(E::Failed);
        })?;
        if let Err(error) = publish(self.snapshot()) {
            self.end = Some(E::Failed);
            return Err(error);
        }
        self.verify_frozen(frozen).inspect_err(|_| {
            self.end = Some(E::Failed);
        })?;
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
