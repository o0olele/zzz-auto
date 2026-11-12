//! 周常培养导航、免费单战及原计划结果交接，共用父控制器与总预算。
use crate::{
    agent_transport::{AgentTransportEnd, AgentTransportRun, AgentTransportSnapshot},
    notorious_hunt_challenge::{
        Resources, WeeklyChallengeEnd, WeeklyChallengeSnapshot, WeeklyHuntChallengeRun,
    },
    simulation_battle::BattleTarget,
    simulation_deploy::PreparedDeployment,
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{GameSessionConfig, InferenceDevice, NativeSession, SessionHandoff};
use od_core::{
    InputMode,
    notorious_hunt_agent::WeeklyAgentRequest,
    notorious_hunt_run::{WeeklyOutcome, WeeklySkipReason},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyAgentStage {
    Transport,
    Challenge,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeeklyAgentEnd {
    Completed,
    Unavailable,
    RecoveredFailure,
    Exhausted,
    AwaitingVerification,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyAgentSnapshot {
    pub id: Uuid,
    pub request: WeeklyAgentRequest,
    pub stage: WeeklyAgentStage,
    pub navigation: AgentTransportSnapshot,
    pub challenge: Option<WeeklyChallengeSnapshot>,
    pub outcome: Option<WeeklyOutcome>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct WeeklyAgentRun {
    snapshot: WeeklyAgentSnapshot,
    execution_id: Uuid,
    root: PathBuf,
    navigation_bundle: PathBuf,
    navigation_hash: Vec<u8>,
    resources: Resources,
    prepared: Option<PreparedDeployment>,
    target: BattleTarget,
    control: PipelineControl,
    navigation: AgentTransportRun,
    challenge: Option<WeeklyHuntChallengeRun>,
    child_started: bool,
    navigation_loaded: bool,
    end: Option<WeeklyAgentEnd>,
}
impl WeeklyAgentRun {
    pub fn new(
        id: Uuid,
        request: WeeklyAgentRequest,
        root: &Path,
        prepared: PreparedDeployment,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<Self> {
        request.validate().map_err(anyhow::Error::msg)?;
        ensure!(!id.is_nil(), "周常培养缺少运行身份");
        ensure!(
            prepared.plan().config.script
                == request
                    .plan
                    .script(&request.teams)
                    .map_err(anyhow::Error::msg)?,
            "周常培养脚本与原计划不符"
        );
        if matches!(target, BattleTarget::Fixture) {
            ensure!(
                prepared.plan().config.mode == InputMode::Foreground
                    && prepared.plan().config.directml_device.is_none(),
                "隔离周常培养只允许前台 CPU"
            );
        }
        if let BattleTarget::Game { window, .. } = &target {
            ensure!(
                window.width == 1920 && window.height == 1080,
                "周常培养须冻结 1080p 窗口"
            );
        }
        od_core::config::reject_links(root)?;
        let root = root.canonicalize()?;
        let navigation_bundle = maa_adapter::agent_transport::validate_agent_transport_bundle(
            &root.join("generated/agent-transport"),
        )?;
        let navigation_hash = Sha256::digest(std::fs::read(
            navigation_bundle.join("agent_transport-manifest.json"),
        )?)
        .to_vec();
        let mut resources = Resources::load(&root, prepared.resource_root()?)?;
        resources.enable_agent_goal(&root)?;
        let navigation = AgentTransportRun::new(request.navigation(), control.clone())?;
        Ok(Self {
            snapshot: WeeklyAgentSnapshot {
                id,
                request,
                stage: WeeklyAgentStage::Transport,
                navigation: navigation.snapshot(),
                challenge: None,
                outcome: None,
                active_ms: 0,
                inputs_released: true,
            },
            execution_id: Uuid::new_v4(),
            root,
            navigation_bundle,
            navigation_hash,
            resources,
            prepared: Some(prepared),
            target,
            control,
            navigation,
            challenge: None,
            child_started: false,
            navigation_loaded: false,
            end: None,
        })
    }
    pub fn snapshot(&self) -> WeeklyAgentSnapshot {
        self.snapshot.clone()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub(crate) fn outcome_for_parent(&self) -> Option<WeeklyOutcome> {
        if self.snapshot.stage != WeeklyAgentStage::Done
            || !self.snapshot.inputs_released
            || self.end != Some(self.completed_end())
        {
            return None;
        }
        self.snapshot.outcome.clone()
    }
    fn completed_end(&self) -> WeeklyAgentEnd {
        match self.snapshot.outcome {
            Some(WeeklyOutcome::AgentSucceeded { .. }) => WeeklyAgentEnd::Completed,
            Some(WeeklyOutcome::Skipped {
                reason: WeeklySkipReason::AgentUnavailable,
                ..
            }) => WeeklyAgentEnd::Unavailable,
            Some(WeeklyOutcome::Skipped {
                reason: WeeklySkipReason::ChallengeFailed,
                ..
            }) => WeeklyAgentEnd::RecoveredFailure,
            Some(WeeklyOutcome::Exhausted { .. }) => WeeklyAgentEnd::Exhausted,
            Some(WeeklyOutcome::AwaitingVerification { .. }) => {
                WeeklyAgentEnd::AwaitingVerification
            }
            _ => WeeklyAgentEnd::Failed,
        }
    }
    fn interrupt(&self) -> Option<WeeklyAgentEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => WeeklyAgentEnd::Paused,
            Interrupt::Stop => WeeklyAgentEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(WeeklyAgentEnd::Paused)
                && self.snapshot.inputs_released
                && self.control.requested() == Some(Interrupt::Pause),
            "周常培养尚未安全暂停"
        );
        if self.snapshot.stage == WeeklyAgentStage::Done || !self.child_started {
            ensure!(self.control.resume(), "停止不能继续");
        } else {
            match self.snapshot.stage {
                WeeklyAgentStage::Transport => self.navigation.resume_after_parent_pause()?,
                WeeklyAgentStage::Challenge => self
                    .challenge
                    .as_mut()
                    .context("周常培养挑战缺失")?
                    .resume()?,
                WeeklyAgentStage::Done => unreachable!(),
            }
        }
        self.end = None;
        Ok(())
    }
    fn navigation_session(&mut self, slot: &mut Option<NativeSession>) -> Result<bool> {
        let path = self.navigation_bundle.join("agent_transport-manifest.json");
        od_core::config::reject_links(&path)?;
        ensure!(
            Sha256::digest(std::fs::read(path)?).as_slice() == self.navigation_hash,
            "周常培养导航资源已变化"
        );
        maa_adapter::agent_transport::validate_agent_transport_bundle(&self.navigation_bundle)?;
        match &self.target {
            BattleTarget::Fixture => {
                let n = slot.as_mut().context("周常培养隔离控制器缺失")?;
                n.quiesce()?;
                n.check_ether_synthesis_target(false)?;
            }
            BattleTarget::Game { window, capture } => match NativeSession::handoff_game(
                slot,
                GameSessionConfig {
                    window,
                    resources: &self.root,
                    capture: *capture,
                    input: InputMode::Foreground,
                    device: InferenceDevice::Cpu,
                },
                &self.control.cancellation(),
            )? {
                SessionHandoff::Cancelled => return Ok(false),
                SessionHandoff::Created => self.navigation_loaded = false,
                SessionHandoff::Reused => (),
            },
        }
        let n = slot.as_mut().context("周常培养控制器交接后缺失")?;
        n.check_combat_input_mode(InputMode::Foreground)?;
        if self.control.requested().is_some() {
            return Ok(false);
        }
        if !self.navigation_loaded {
            n.load_agent_transport_bundle(&self.navigation_bundle)?;
            self.navigation_loaded = true;
        }
        Ok(true)
    }
    pub fn execute_with_budget(
        &mut self,
        slot: &mut Option<NativeSession>,
        parent_remaining: Duration,
        mut publish: impl FnMut(WeeklyAgentSnapshot) -> Result<()>,
    ) -> Result<WeeklyAgentEnd> {
        use WeeklyAgentEnd as E;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "周常培养不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.request.timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<E> {
            self.resources.verify()?;
            loop {
                if let Some(e) = self.interrupt() {
                    return Ok(e);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                if self.snapshot.stage == WeeklyAgentStage::Done {
                    return Ok(self.completed_end());
                }
                if self.snapshot.stage == WeeklyAgentStage::Transport {
                    if !self.navigation_session(slot)? {
                        return Ok(self.interrupt().unwrap_or(E::Stopped));
                    }
                    if remaining().is_zero() {
                        return Ok(E::TimedOut);
                    }
                    self.child_started = true;
                    let s = &mut self.snapshot;
                    let result = self.navigation.execute_with_budget(
                        slot.as_mut().context("周常培养导航会话缺失")?,
                        matches!(self.target, BattleTarget::Game { .. }),
                        remaining(),
                        |n| {
                            s.inputs_released = n.inputs_released;
                            s.navigation = n;
                            s.active_ms =
                                initial.saturating_add(started.elapsed().as_millis() as u64);
                            publish(s.clone())
                        },
                    );
                    self.snapshot.navigation = self.navigation.snapshot();
                    self.snapshot.inputs_released = self.snapshot.navigation.inputs_released;
                    match result? {
                        AgentTransportEnd::Unavailable => {
                            ensure!(
                                self.snapshot.navigation.flow.unavailable()
                                    && self.snapshot.inputs_released,
                                "周常培养未找到结果尚未保存或释放"
                            );
                            self.snapshot.outcome = Some(WeeklyOutcome::Skipped {
                                execution_id: self.execution_id,
                                reason: WeeklySkipReason::AgentUnavailable,
                            });
                            self.snapshot.stage = WeeklyAgentStage::Done;
                        }
                        AgentTransportEnd::AwaitingVerification => {
                            let request = self
                                .snapshot
                                .request
                                .resolve(
                                    self.navigation
                                        .receipt_for_parent()
                                        .context("周常培养尚无本次已保存导航结果")?,
                                )
                                .map_err(anyhow::Error::msg)?;
                            if let Some(e) = self.interrupt() {
                                return Ok(e);
                            }
                            if remaining().is_zero() {
                                return Ok(E::TimedOut);
                            }
                            let child = WeeklyHuntChallengeRun::from_frozen(
                                self.snapshot.id,
                                request,
                                self.resources.clone(),
                                self.prepared.take().context("周常培养冻结脚本缺失")?,
                                self.target.clone(),
                                self.control.clone(),
                                Some(&self.navigation),
                            )?;
                            self.snapshot.challenge = Some(child.snapshot());
                            self.challenge = Some(child);
                            self.snapshot.stage = WeeklyAgentStage::Challenge;
                            self.child_started = false;
                        }
                        AgentTransportEnd::Paused => return Ok(E::Paused),
                        AgentTransportEnd::Stopped => return Ok(E::Stopped),
                        AgentTransportEnd::TimedOut => return Ok(E::TimedOut),
                        AgentTransportEnd::Failed => anyhow::bail!("周常培养导航失败"),
                    }
                } else {
                    self.child_started = true;
                    let child = self.challenge.as_mut().context("周常培养挑战缺失")?;
                    let s = &mut self.snapshot;
                    let result = child.execute_with_budget(slot, remaining(), |c| {
                        s.inputs_released = c.inputs_released;
                        s.challenge = Some(c);
                        s.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
                        publish(s.clone())
                    });
                    self.snapshot.challenge = Some(child.snapshot());
                    self.snapshot.inputs_released = child.snapshot().inputs_released;
                    let end = result?;
                    let outcome = if end == WeeklyChallengeEnd::Completed {
                        let (proof, goal) = child
                            .agent_completion_for_parent()
                            .context("周常培养缺少本次完成和达成读数")?;
                        WeeklyOutcome::AgentSucceeded {
                            execution_id: self.execution_id,
                            rewards_before: proof.rewards_before,
                            rewards_after: proof.rewards_after,
                            goal,
                        }
                    } else {
                        match end {
                            WeeklyChallengeEnd::RecoveredFailure
                            | WeeklyChallengeEnd::Exhausted
                            | WeeklyChallengeEnd::AwaitingVerification => child
                                .outcome_for_parent()
                                .context("周常培养缺少本次收尾结果")?,
                            WeeklyChallengeEnd::Paused => return Ok(E::Paused),
                            WeeklyChallengeEnd::Stopped => return Ok(E::Stopped),
                            WeeklyChallengeEnd::TimedOut => return Ok(E::TimedOut),
                            WeeklyChallengeEnd::Failed => anyhow::bail!("周常培养挑战失败"),
                            WeeklyChallengeEnd::Completed => unreachable!(),
                        }
                    };
                    self.snapshot.outcome = Some(outcome);
                    self.snapshot.stage = WeeklyAgentStage::Done;
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.snapshot.inputs_released = slot.as_mut().is_none_or(|n| n.release_inputs().is_ok());
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        self.end = Some(E::Failed);
        ensure!(self.snapshot.inputs_released, "周常培养收尾未释放输入");
        let mut end = result?;
        publish(self.snapshot.clone())?;
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(e) = self.interrupt() {
            end = e;
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
