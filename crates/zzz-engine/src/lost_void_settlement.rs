//! 消费实际层结果的结算执行器；后态、保存及输入释放全部正常才交回。
use crate::{
    lost_void_combat::{FrozenLostVoidPreparation, LostVoidCombatSeed},
    lost_void_event::{LostVoidEventEnd, LostVoidEventRun, LostVoidEventSnapshot},
    lost_void_level::LostVoidLevelRun,
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{
    NativeSession, combat_vision::CombatRecognizer, lost_void_roster::LostVoidHudReader,
    lost_void_settlement::SettlementInputEnd,
};
use od_core::{
    lost_void_event::LostVoidEventRequest,
    lost_void_scene::LostVoidScreen as S,
    lost_void_settlement::*,
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use uuid::Uuid;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidSettlementEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidSettlementSnapshot {
    pub flow: SettlementFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
    pub event_active: bool,
    pub event: Option<LostVoidEventSnapshot>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}
pub struct LostVoidSettlementRun {
    snapshot: LostVoidSettlementSnapshot,
    seed: LostVoidCombatSeed,
    scene: PathBuf,
    combat: PathBuf,
    detector: PathBuf,
    control: PipelineControl,
    recognizer: Option<CombatRecognizer>,
    reader: Option<LostVoidHudReader>,
    event: Option<LostVoidEventRun>,
    event_started: bool,
    end: Option<LostVoidSettlementEnd>,
    steps: u32,
}
impl LostVoidSettlementRun {
    pub fn after_level(
        frozen: &FrozenLostVoidPreparation,
        level: &LostVoidLevelRun,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(level.uses_control(&control), "结算与实际层执行器须共用控制");
        level.verify_frozen(frozen)?;
        let source = level.receipt().context("当前层尚未安全交回结果")?.clone();
        let snapshot = level.snapshot();
        let request = SettlementRequest {
            source,
            settlement_id: Uuid::new_v4(),
            next_level_id: Uuid::new_v4(),
            mission: frozen.seed().mission,
            challenge: frozen.seed().challenge.clone(),
            plan: snapshot.plan,
            expected_count: snapshot.members,
            members: snapshot.actual_members,
            timeout_ms,
        };
        Self::new(frozen, request, control)
    }
    pub fn after_recovery_result(
        frozen: &FrozenLostVoidPreparation,
        recovery: &mut crate::lost_void_recovery::LostVoidRecoveryRun,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(
            recovery.uses_control(&control),
            "恢复结果与结算须共用实际控制"
        );
        recovery.verify_frozen(frozen)?;
        ensure!(
            matches!(
                recovery
                    .receipt()
                    .context("恢复尚未安全交回或已消费")?
                    .outcome,
                od_core::lost_void_recovery_menu::RecoveryOutcome::ObservedResult { .. }
            ),
            "重开到达不能交接最终结算"
        );
        let source = recovery.source();
        let request = SettlementRequest {
            source: od_core::lost_void_level::LostVoidLevelReceipt {
                run_id: source.failure.run_id,
                level_id: source.failure.attempt_id,
                region: source.memory.region().into(),
                options: source.options.clone(),
                outcome: od_core::lost_void_level::LevelOutcome::Aborted {
                    failure: source.failure.clone(),
                },
            },
            settlement_id: Uuid::new_v4(),
            next_level_id: Uuid::new_v4(),
            mission: frozen.seed().mission,
            challenge: frozen.seed().challenge.clone(),
            plan: source.plan.clone(),
            expected_count: source.members.len() as u8,
            members: Some(source.members.clone()),
            timeout_ms,
        };
        let run = Self::new(frozen, request, control)?;
        recovery.consume_handoff();
        Ok(run)
    }
    fn new(
        frozen: &FrozenLostVoidPreparation,
        request: SettlementRequest,
        control: PipelineControl,
    ) -> Result<Self> {
        let flow = SettlementFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: LostVoidSettlementSnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
                event_active: false,
                event: None,
                finished_at: None,
            },
            seed: frozen.seed().clone(),
            scene: frozen.scene_root().into(),
            combat: frozen.combat_root().into(),
            detector: frozen
                .npc_detector_root()
                .context("结算缺少首次输入前冻结的导航资源")?
                .into(),
            control,
            recognizer: None,
            reader: None,
            event: None,
            event_started: false,
            end: None,
            steps: 0,
        })
    }
    pub fn snapshot(&self) -> LostVoidSettlementSnapshot {
        self.snapshot.clone()
    }
    pub fn uses_control(&self, c: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &c.cancellation())
    }
    pub(crate) fn finished_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        let receipt = self.receipt()?;
        matches!(receipt.outcome, SettlementOutcome::Finished { .. })
            .then_some(self.snapshot.finished_at)
            .flatten()
    }
    pub fn receipt(&self) -> Option<&SettlementReceipt> {
        (self.end == Some(LostVoidSettlementEnd::Ready)
            && self.snapshot.inputs_released
            && self.control.requested().is_none())
        .then(|| self.snapshot.flow.receipt())
        .flatten()
    }
    pub(crate) fn verify_frozen(&self, frozen: &FrozenLostVoidPreparation) -> Result<()> {
        ensure!(
            frozen.seed() == &self.seed
                && frozen.scene_root() == self.scene
                && frozen.combat_root() == self.combat
                && frozen.npc_detector_root() == Some(self.detector.as_path()),
            "结算冻结配置或资源路径已变化"
        );
        frozen.verify()
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidSettlementEnd::Paused),
            "结算尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidSettlementEnd::Paused | LostVoidSettlementEnd::Ready)
            ) && self.snapshot.inputs_released,
            "结算尚未安全暂停"
        );
        if self.snapshot.event_active && self.event_started {
            self.event
                .as_mut()
                .context("结算自动事件缺失")?
                .resume_after_parent_pause()?;
        } else {
            self.snapshot.flow.resume().map_err(anyhow::Error::msg)?;
            ensure!(self.control.resume(), "停止不能被继续覆盖");
        }
        if let Some(r) = &mut self.reader {
            r.reset();
        }
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidSettlementEnd> {
        self.control.requested().map(|v| match v {
            Interrupt::Pause => LostVoidSettlementEnd::Paused,
            Interrupt::Stop => LostVoidSettlementEnd::Stopped,
        })
    }
    fn start_event(&mut self) -> Result<()> {
        if let Some(event) = &mut self.event {
            event.continue_observing()?;
        } else {
            let r = self.snapshot.flow.request();
            self.event = Some(LostVoidEventRun::new(
                LostVoidEventRequest {
                    run_id: r.source.run_id,
                    interaction_id: r.settlement_id,
                    challenge: self.seed.challenge.clone(),
                    options: self.snapshot.flow.options(),
                    entry: false,
                    completed_first: vec![],
                    members: r.expected_count,
                    allow_challenge_confirm: false,
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
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidSettlementSnapshot) -> Result<()>,
    ) -> Result<LostVoidSettlementEnd> {
        use LostVoidSettlementEnd as E;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "结算不能直接重跑"
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
                ensure!(self.steps < 4096, "结算观察达到上限");
                self.steps += 1;
                if self.snapshot.event_active {
                    self.event_started = true;
                    let snapshot = &mut self.snapshot;
                    let result = self
                        .event
                        .as_mut()
                        .context("结算自动事件执行器缺失")?
                        .execute_with_budget(session, game, remaining(), |event| {
                            snapshot.inputs_released = event.inputs_released;
                            snapshot.event = Some(event);
                            snapshot.active_ms =
                                initial.saturating_add(started.elapsed().as_millis() as u64);
                            publish(snapshot.clone())
                        });
                    self.snapshot.event = Some(self.event.as_ref().unwrap().snapshot());
                    self.snapshot.inputs_released = session.inputs_released();
                    match result? {
                        LostVoidEventEnd::Ready => {
                            let receipt = self
                                .event
                                .as_ref()
                                .unwrap()
                                .receipt_for_parent()
                                .context("结算自动事件未安全交回")?;
                            ensure!(
                                receipt.returned_to == S::World,
                                "中途结算自动事件未返回世界"
                            );
                            self.snapshot
                                .flow
                                .accept_event(receipt)
                                .map_err(anyhow::Error::msg)?;
                            self.snapshot.event_active = false;
                            self.event_started = false;
                        }
                        LostVoidEventEnd::Paused => return Ok(E::Paused),
                        LostVoidEventEnd::Stopped => return Ok(E::Stopped),
                        LostVoidEventEnd::TimedOut => return Ok(E::TimedOut),
                        LostVoidEventEnd::Failed => return Ok(E::Failed),
                    }
                    publish(self.snapshot())?;
                    continue;
                }
                if self.snapshot.flow.event_screen_ready() {
                    self.start_event()?;
                    publish(self.snapshot())?;
                    continue;
                }
                let read = session.read_lost_void_settlement(
                    &self.snapshot.flow,
                    self.reader.as_mut().unwrap(),
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "结算观察后输入未释放");
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
                let action = self
                    .snapshot
                    .flow
                    .next(&view, self.snapshot.active_ms)
                    .map_err(anyhow::Error::msg)?;
                if self
                    .snapshot
                    .flow
                    .receipt()
                    .is_some_and(|r| matches!(r.outcome, SettlementOutcome::Finished { .. }))
                    && self.snapshot.finished_at.is_none()
                {
                    self.snapshot.finished_at = Some(chrono::Utc::now());
                }
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
                let input = session.act_lost_void_settlement(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "结算输入未释放");
                match input {
                    Ok(SettlementInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(SettlementInputEnd::Changed | SettlementInputEnd::NotPosted) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(SettlementInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(E::TimedOut);
                    }
                    Ok(SettlementInputEnd::Unknown) => {
                        if self.control.requested().is_none() {
                            self.control.pause();
                        }
                        return Ok(self.interrupted().unwrap_or(E::Paused));
                    }
                    Err(error) => {
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
        ensure!(self.snapshot.inputs_released, "结算收尾输入未释放");
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
