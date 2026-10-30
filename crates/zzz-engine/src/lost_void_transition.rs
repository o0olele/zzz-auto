//! 绑定实际出口的跨层等待，事件子流程同控制、同预算，加载历史覆盖子截图。
use crate::{
    lost_void_combat::{FrozenLostVoidPreparation, LostVoidCombatSeed},
    lost_void_event::{LostVoidEventEnd, LostVoidEventRun, LostVoidEventSnapshot},
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{
    NativeSession, combat_vision::CombatRecognizer, lost_void_roster::LostVoidHudReader,
};
use od_core::{
    lost_void_event::LostVoidEventRequest,
    lost_void_loading::LoadingObservation,
    lost_void_npc::NpcMemory,
    lost_void_scene::LostVoidScreen as S,
    lost_void_transition::{
        TransitionFlow, TransitionOutcome, TransitionReceipt, TransitionRequest, TransitionView,
    },
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidTransitionEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidTransitionSnapshot {
    pub flow: TransitionFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
    pub event_active: bool,
    pub event: Option<LostVoidEventSnapshot>,
}
pub struct LostVoidTransitionRun {
    snapshot: LostVoidTransitionSnapshot,
    seed: LostVoidCombatSeed,
    scene: PathBuf,
    combat: PathBuf,
    detector: PathBuf,
    memory: NpcMemory,
    control: PipelineControl,
    reader: Option<LostVoidHudReader>,
    event: Option<LostVoidEventRun>,
    event_started: bool,
    end: Option<LostVoidTransitionEnd>,
    steps: u32,
}
impl LostVoidTransitionRun {
    pub(crate) fn new(
        frozen: &FrozenLostVoidPreparation,
        request: TransitionRequest,
        memory: NpcMemory,
        loading: Option<LoadingObservation>,
        control: PipelineControl,
    ) -> Result<Self> {
        memory.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            memory.run_id() == request.exit.run_id && memory.level_id() == request.exit.level_id,
            "跨层记忆与出口不在同层"
        );
        let flow = TransitionFlow::new(request, loading).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: LostVoidTransitionSnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
                event_active: false,
                event: None,
            },
            seed: frozen.seed().clone(),
            scene: frozen.scene_root().into(),
            combat: frozen.combat_root().into(),
            detector: frozen
                .npc_detector_root()
                .context("跨层缺少首次输入前冻结的导航模型")?
                .into(),
            memory,
            control,
            reader: None,
            event: None,
            event_started: false,
            end: None,
            steps: 0,
        })
    }
    pub fn snapshot(&self) -> LostVoidTransitionSnapshot {
        self.snapshot.clone()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn receipt(&self) -> Option<&TransitionReceipt> {
        (self.end == Some(LostVoidTransitionEnd::Ready)
            && self.snapshot.inputs_released
            && self.control.requested().is_none())
        .then(|| self.snapshot.flow.receipt())
        .flatten()
    }
    /// 仅原层 NPC 抢到门交互时提供实际事件执行器，供已验证的返回移动消费。
    pub fn same_level_event(&self) -> Option<&LostVoidEventRun> {
        matches!(
            self.receipt().map(|r| &r.outcome),
            Some(TransitionOutcome::SameLevelEvent)
        )
        .then(|| self.event.as_ref())
        .flatten()
    }
    pub fn artifact_options(&self) -> Result<od_core::lost_void::ArtifactSelection> {
        self.receipt().context("跨层尚未安全交回")?;
        let mut options = self.snapshot.flow.request().roster.options.clone();
        if let Some(receipt) = self.event.as_ref().and_then(|e| e.receipt()) {
            for rule in &receipt.append_priority {
                if !options.battle_priority.contains(rule) {
                    options.battle_priority.push(rule.clone());
                }
            }
        }
        od_core::lost_void::choose_artifacts(&self.seed.challenge, &[], 0, &options)
            .map_err(anyhow::Error::msg)?;
        Ok(options)
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidTransitionEnd::Paused),
            "跨层尚未安全暂停"
        );
        self.resume_after_parent_pause()
    }
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidTransitionEnd::Paused | LostVoidTransitionEnd::Ready)
            ) && self.snapshot.inputs_released,
            "跨层尚未安全暂停"
        );
        if self.snapshot.event_active && self.event_started {
            self.event
                .as_mut()
                .context("跨层事件暂停状态缺失")?
                .resume_after_parent_pause()?;
        } else {
            self.snapshot.flow.resume().map_err(anyhow::Error::msg)?;
            ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        }
        self.reader = None;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidTransitionEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidTransitionEnd::Paused,
            Interrupt::Stop => LostVoidTransitionEnd::Stopped,
        })
    }
    pub(crate) fn verify_frozen(&self, frozen: &FrozenLostVoidPreparation) -> Result<()> {
        ensure!(
            frozen.seed() == &self.seed
                && frozen.scene_root() == self.scene
                && frozen.combat_root() == self.combat
                && frozen.npc_detector_root() == Some(self.detector.as_path()),
            "跨层冻结身份或资源路径已变化"
        );
        frozen.verify()
    }
    fn start_event(&mut self) -> Result<()> {
        if let Some(event) = self.event.as_mut() {
            event.continue_observing()?;
        } else {
            let request = self.snapshot.flow.request();
            self.event = Some(LostVoidEventRun::new(
                LostVoidEventRequest {
                    run_id: request.exit.run_id,
                    interaction_id: request.exit.interaction_id,
                    challenge: self.seed.challenge.clone(),
                    options: request.roster.options.clone(),
                    entry: self.memory.region() == "入口",
                    completed_first: self.memory.completed_first().to_vec(),
                    members: request.roster.members.len() as u8,
                    allow_challenge_confirm: request
                        .region()
                        .map_err(anyhow::Error::msg)?
                        .starts_with("挑战-"),
                    allow_gear_notice: false,
                    timeout_ms: 300_000,
                },
                self.control.clone(),
            )?);
        }
        self.snapshot.event_active = true;
        self.event_started = false;
        Ok(())
    }
    pub fn execute_with_budget(
        &mut self,
        frozen: &FrozenLostVoidPreparation,
        session: &mut NativeSession,
        recognizer: &mut CombatRecognizer,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidTransitionSnapshot) -> Result<()>,
    ) -> Result<LostVoidTransitionEnd> {
        use LostVoidTransitionEnd as E;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "跨层不能直接重跑"
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
            session.require_lost_void_battle_roots(&self.scene, &self.combat, &self.detector)?;
            ensure!(
                recognizer.bundle.root == self.combat,
                "跨层队伍识别器与冻结资源不符"
            );
            if self.reader.is_none() {
                self.reader = Some(LostVoidHudReader::new(recognizer)?);
            }
            session.require_lost_void_roster(self.reader.as_ref().unwrap())?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                ensure!(self.steps < 4096, "跨层观察达到上限");
                self.steps += 1;
                if self.snapshot.event_active {
                    self.event_started = true;
                    let trace = if self.snapshot.flow.departure().is_none() {
                        Some(session.trace_lost_void_exit(
                            *self.snapshot.flow.request().exit.interaction_id.as_bytes(),
                        )?)
                    } else {
                        None
                    };
                    let snapshot = &mut self.snapshot;
                    let result = self
                        .event
                        .as_mut()
                        .context("跨层事件执行器缺失")?
                        .execute_with_budget(session, game, remaining(), |event| {
                            if let Some(trace) = &trace
                                && let Some(loading) = trace.observation()?
                            {
                                snapshot
                                    .flow
                                    .record_loading(loading)
                                    .map_err(anyhow::Error::msg)?;
                            }
                            snapshot.inputs_released = event.inputs_released;
                            snapshot.event = Some(event);
                            snapshot.active_ms =
                                initial.saturating_add(started.elapsed().as_millis() as u64);
                            publish(snapshot.clone())
                        });
                    if let Some(trace) = trace
                        && let Some(loading) = trace.finish()?
                    {
                        self.snapshot
                            .flow
                            .record_loading(loading)
                            .map_err(anyhow::Error::msg)?;
                    }
                    self.snapshot.event = Some(self.event.as_ref().unwrap().snapshot());
                    self.snapshot.inputs_released =
                        self.event.as_ref().unwrap().snapshot().inputs_released;
                    match result? {
                        LostVoidEventEnd::Ready => {
                            let event = self.event.as_ref().unwrap();
                            let receipt =
                                event.receipt_for_parent().context("跨层事件尚未安全交回")?;
                            if matches!(receipt.returned_to, S::BattleResult | S::BattleFail) {
                                self.snapshot.flow.resume().map_err(anyhow::Error::msg)?;
                            } else {
                                let flow = event.snapshot().flow;
                                self.snapshot
                                    .flow
                                    .accept_event(
                                        receipt,
                                        flow.departure().cloned(),
                                        flow.confirmed_challenge().cloned(),
                                    )
                                    .map_err(anyhow::Error::msg)?;
                            }
                            self.snapshot.event_active = false;
                            self.event_started = false;
                        }
                        LostVoidEventEnd::Paused => return Ok(E::Paused),
                        LostVoidEventEnd::Stopped => return Ok(E::Stopped),
                        LostVoidEventEnd::TimedOut => return Ok(E::TimedOut),
                        LostVoidEventEnd::Failed => return Ok(E::Failed),
                    }
                    self.snapshot.active_ms =
                        initial.saturating_add(started.elapsed().as_millis() as u64);
                    publish(self.snapshot.clone())?;
                    continue;
                }
                if let Some(receipt) = self.snapshot.flow.receipt() {
                    if matches!(receipt.outcome, TransitionOutcome::Event { .. }) {
                        self.start_event()?;
                        publish(self.snapshot())?;
                        continue;
                    }
                    return Ok(E::Ready);
                }
                let cancel = self.control.cancellation();
                let Some(frame) = session.combat_frame(&cancel)? else {
                    return Ok(self.interrupted().unwrap_or(E::TimedOut));
                };
                let loading = session.inspect_lost_void_loading(
                    frame.recognition_image(),
                    &cancel,
                    remaining(),
                )?;
                let view = if let Some(loading) = loading {
                    Some(TransitionView {
                        screen: S::Unknown,
                        loading: Some(loading),
                        members: None,
                    })
                } else {
                    session
                        .inspect_lost_void_roster(
                            &frame,
                            self.reader.as_mut().unwrap(),
                            &cancel,
                            remaining(),
                        )?
                        .map(|v| TransitionView {
                            screen: v.screen,
                            loading: None,
                            members: v.members.filter(|m| {
                                m.len() == self.snapshot.flow.request().roster.members.len()
                            }),
                        })
                };
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "跨层读帧后无法释放输入");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                match view {
                    Some(view) if frame.age_ms() <= 1500 => {
                        self.snapshot.flow.next(&view).map_err(anyhow::Error::msg)?
                    }
                    _ => self.snapshot.flow.invalidate_observation(),
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot())?;
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() || !self.snapshot.inputs_released {
            self.end = Some(E::Failed);
        }
        ensure!(self.snapshot.inputs_released, "跨层收尾无法确认输入释放");
        let mut end = result?;
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
