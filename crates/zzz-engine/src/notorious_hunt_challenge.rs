//! 周常单战：普通入口、免费准备、出战、战前移动、冻结战斗与实际余量结算。
use crate::{
    combat::{CombatRun, CombatSegmentEnd, CombatTimeout},
    notorious_hunt_preparation::{
        Resources as PreparationResources, WeeklyHuntPreparationRun, WeeklyPreparationEnd,
        WeeklyPreparationSnapshot,
    },
    notorious_hunt_recovery::{WeeklyRecoveryEnd, WeeklyRecoveryRun, WeeklyRecoverySnapshot},
    notorious_hunt_settlement::{
        WeeklySettlementEnd, WeeklySettlementRun, WeeklySettlementSnapshot,
    },
    notorious_hunt_timeout::{WeeklyTimeoutEnd, WeeklyTimeoutRun, WeeklyTimeoutSnapshot},
    simulation_battle::BattleTarget,
    simulation_deploy::{DeploymentEnd, DeploymentRun, DeploymentSnapshot, PreparedDeployment},
    simulation_teams::TeamsEnd,
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{
    GameSessionConfig, InferenceDevice, NativeSession, SessionHandoff,
    combat_vision::{CombatBundle, CombatRecognizer},
};
use od_core::{
    InputMode,
    app_combat::AppCombatPlan,
    battle_result::NormalBattleResult,
    charge_plan::ChargePlanItem,
    combat::CombatProgress,
    notorious_hunt_challenge::WeeklyDeployRequest,
    notorious_hunt_preparation::WeeklyPreparationRequest,
    notorious_hunt_timeout::{WeeklyTimeoutCause, WeeklyTimeoutFlow, WeeklyTimeoutRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use uuid::Uuid;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyChallengeStage {
    Preparation,
    Deployment,
    Move,
    Combat,
    Settlement,
    Recovery,
    TimeoutExit,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeeklyChallengeEnd {
    Completed,
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
pub struct WeeklyChallengeSnapshot {
    pub id: Uuid,
    pub request: WeeklyPreparationRequest,
    pub stage: WeeklyChallengeStage,
    pub preparation: WeeklyPreparationSnapshot,
    pub deployment: Option<DeploymentSnapshot<WeeklyDeployRequest>>,
    pub combat: Option<CombatProgress>,
    pub movement: Option<od_core::challenge_move::ChallengeMove>,
    pub movement_ms: u64,
    pub settlement: Option<WeeklySettlementSnapshot>,
    pub recovery: Option<WeeklyRecoverySnapshot>,
    pub timeout_exit: Option<WeeklyTimeoutSnapshot>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeeklyChallengeProof {
    pub run_id: Uuid,
    pub execution_id: Uuid,
    pub plan: ChargePlanItem,
    pub rewards_before: u8,
    pub rewards_after: u8,
}
#[derive(Clone)]
pub(crate) struct Resources {
    root: PathBuf,
    preparation: PreparationResources,
    deploy: PathBuf,
    combat: PathBuf,
    agent_goal: Option<PathBuf>,
    hashes: Vec<(PathBuf, Vec<u8>)>,
}
impl Resources {
    pub(crate) fn load(root: &Path, combat: &Path) -> Result<Self> {
        od_core::config::reject_links(root)?;
        let root = root.canonicalize()?;
        let preparation = PreparationResources::load(&root)?;
        maa_adapter::yolo::validate_challenge_detector(&preparation.daily)?;
        let deploy = maa_adapter::simulation_deploy::validate_simulation_deploy_bundle(
            &root.join("generated/simulation-deploy"),
        )?;
        let combat = CombatBundle::load(combat)?.root;
        let hashes = [
            deploy.join("simulation_deploy-manifest.json"),
            combat.join("combat-manifest.json"),
        ]
        .into_iter()
        .map(|p| Ok((p.clone(), Sha256::digest(std::fs::read(p)?).to_vec())))
        .collect::<Result<_>>()?;
        Ok(Self {
            root,
            preparation,
            deploy,
            combat,
            agent_goal: None,
            hashes,
        })
    }
    pub(crate) fn enable_agent_goal(&mut self, root: &Path) -> Result<()> {
        ensure!(self.agent_goal.is_none(), "周常培养资源不能重复冻结");
        let path = maa_adapter::agent_goal::validate_agent_goal_bundle(
            &root.join("generated/agent-goal"),
        )?;
        let manifest = path.join("agent_goal-manifest.json");
        self.hashes.push((
            manifest.clone(),
            Sha256::digest(std::fs::read(manifest)?).to_vec(),
        ));
        self.agent_goal = Some(path);
        Ok(())
    }
    pub(crate) fn verify(&self) -> Result<()> {
        for (p, h) in &self.hashes {
            od_core::config::reject_links(p)?;
            ensure!(
                Sha256::digest(std::fs::read(p)?).as_slice() == h,
                "周常挑战冻结资源已变化"
            );
        }
        self.preparation.verify()?;
        maa_adapter::yolo::validate_challenge_detector(&self.preparation.daily)?;
        maa_adapter::simulation_deploy::validate_simulation_deploy_bundle(&self.deploy)?;
        CombatBundle::load(&self.combat)?;
        if let Some(path) = &self.agent_goal {
            maa_adapter::agent_goal::validate_agent_goal_bundle(path)?;
        }
        Ok(())
    }
}
enum Child {
    Preparation(Box<WeeklyHuntPreparationRun>),
    Deployment(Box<DeploymentRun<WeeklyDeployRequest>>),
    Combat(Box<CombatRun>),
    Settlement(Box<WeeklySettlementRun>),
    Recovery(Box<WeeklyRecoveryRun>),
    TimeoutExit(Box<WeeklyTimeoutRun>),
}
pub struct WeeklyHuntChallengeRun {
    snapshot: WeeklyChallengeSnapshot,
    execution_id: Uuid,
    resources: Resources,
    plan: AppCombatPlan,
    prepared: Option<PreparedDeployment>,
    target: BattleTarget,
    control: PipelineControl,
    child: Option<Child>,
    child_started: bool,
    loaded: Option<WeeklyChallengeStage>,
    recognizer: Option<CombatRecognizer>,
    end: Option<WeeklyChallengeEnd>,
}
impl WeeklyHuntChallengeRun {
    pub fn new(
        id: Uuid,
        request: WeeklyPreparationRequest,
        root: &Path,
        prepared: PreparedDeployment,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<Self> {
        let resources = Resources::load(root, prepared.resource_root()?)?;
        Self::from_frozen(id, request, resources, prepared, target, control, None)
    }
    pub(crate) fn from_frozen(
        id: Uuid,
        request: WeeklyPreparationRequest,
        resources: Resources,
        prepared: PreparedDeployment,
        target: BattleTarget,
        control: PipelineControl,
        agent: Option<&crate::agent_transport::AgentTransportRun>,
    ) -> Result<Self> {
        ensure!(!id.is_nil(), "周常挑战缺少运行身份");
        request.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            prepared.plan().config.script
                == request
                    .plan
                    .script(&request.teams)
                    .map_err(anyhow::Error::msg)?,
            "周常战斗脚本与冻结计划不一致"
        );
        if matches!(target, BattleTarget::Fixture) {
            ensure!(
                prepared.plan().config.mode == InputMode::Foreground
                    && prepared.plan().config.directml_device.is_none(),
                "隔离周常挑战只复用前台 CPU 控制器"
            );
        }
        if let BattleTarget::Game { window, .. } = &target {
            ensure!(
                window.width == 1920 && window.height == 1080,
                "周常挑战须冻结 1080p 窗口"
            );
        }
        resources.verify()?;
        ensure!(
            resources.combat == prepared.resource_root()?
                && resources.agent_goal.is_some() == agent.is_some(),
            "周常冻结资源与本次战斗或培养导航不一致"
        );
        let plan = prepared.plan().clone();
        let child = if let Some(agent) = agent {
            WeeklyHuntPreparationRun::after_agent_transport(
                id,
                request.clone(),
                &resources.root,
                control.clone(),
                agent,
            )?
        } else {
            WeeklyHuntPreparationRun::new(id, request.clone(), &resources.root, control.clone())?
        };
        Ok(Self {
            snapshot: WeeklyChallengeSnapshot {
                id,
                request,
                stage: WeeklyChallengeStage::Preparation,
                preparation: child.snapshot(),
                deployment: None,
                combat: None,
                movement: None,
                movement_ms: 0,
                settlement: None,
                recovery: None,
                timeout_exit: None,
                active_ms: 0,
                inputs_released: true,
            },
            execution_id: Uuid::new_v4(),
            resources,
            plan,
            prepared: Some(prepared),
            target,
            control,
            child: Some(Child::Preparation(Box::new(child))),
            child_started: false,
            loaded: None,
            recognizer: None,
            end: None,
        })
    }
    pub fn snapshot(&self) -> WeeklyChallengeSnapshot {
        self.snapshot.clone()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub(crate) fn agent_completion_for_parent(
        &self,
    ) -> Option<(WeeklyChallengeProof, od_core::agent_goal::AgentGoalStatus)> {
        self.resources.agent_goal.as_ref()?;
        let proof = self.proof_for_parent()?;
        let goal = self.snapshot.settlement.as_ref()?.agent_goal.as_ref()?;
        goal.validate().ok()?;
        Some((proof, goal.observed()?))
    }
    /// 仅从这个活执行器的正常终态提取；反序列化快照不能恢复执行证明。
    pub(crate) fn outcome_for_parent(&self) -> Option<od_core::notorious_hunt_run::WeeklyOutcome> {
        use od_core::notorious_hunt_run::WeeklyOutcome;
        if !self.snapshot.inputs_released {
            return None;
        }
        if let Some(proof) = self.proof_for_parent() {
            return Some(WeeklyOutcome::Succeeded {
                execution_id: proof.execution_id,
                rewards_before: proof.rewards_before,
                rewards_after: proof.rewards_after,
            });
        }
        match self.end? {
            WeeklyChallengeEnd::RecoveredFailure
                if self.snapshot.stage == WeeklyChallengeStage::Done
                    && self.snapshot.recovery.as_ref().is_some_and(|s| {
                        s.inputs_released && s.flow.validate().is_ok() && s.flow.completed()
                    }) =>
            {
                Some(WeeklyOutcome::Skipped {
                    execution_id: self.execution_id,
                    reason: od_core::notorious_hunt_run::WeeklySkipReason::ChallengeFailed,
                })
            }
            WeeklyChallengeEnd::Exhausted => Some(WeeklyOutcome::Exhausted {
                execution_id: self.execution_id,
            }),
            WeeklyChallengeEnd::AwaitingVerification => Some(WeeklyOutcome::AwaitingVerification {
                execution_id: self.execution_id,
            }),
            _ => None,
        }
    }
    pub fn proof(&self) -> Option<WeeklyChallengeProof> {
        if self.control.requested().is_some() {
            return None;
        }
        self.proof_for_parent()
    }
    pub(crate) fn proof_for_parent(&self) -> Option<WeeklyChallengeProof> {
        if self.end != Some(WeeklyChallengeEnd::Completed)
            || !self.snapshot.inputs_released
            || self.snapshot.stage != WeeklyChallengeStage::Done
        {
            return None;
        }
        let flow = &self.snapshot.settlement.as_ref()?.flow;
        flow.validate().ok()?;
        Some(WeeklyChallengeProof {
            run_id: self.snapshot.id,
            execution_id: self.execution_id,
            plan: self.snapshot.request.plan.clone(),
            rewards_before: flow.rewards_before(),
            rewards_after: flow.rewards_after()?,
        })
    }
    fn interrupted(&self) -> Option<WeeklyChallengeEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => WeeklyChallengeEnd::Paused,
            Interrupt::Stop => WeeklyChallengeEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(WeeklyChallengeEnd::Paused)
                && self.snapshot.inputs_released
                && self.control.requested() == Some(Interrupt::Pause),
            "周常挑战尚未安全暂停"
        );
        if !self.child_started {
            ensure!(self.control.resume(), "停止不能继续");
        } else {
            match self.child.as_mut().context("周常挑战子流程缺失")? {
                Child::Preparation(c) => c.resume_after_parent_pause()?,
                Child::Deployment(c) => c.resume()?,
                Child::Combat(_) if self.snapshot.stage == WeeklyChallengeStage::Move => {
                    ensure!(self.control.resume(), "停止不能继续");
                }
                Child::Combat(c) => c.resume_segment_after_parent_pause()?,
                Child::Settlement(c) => {
                    c.resume()?;
                    self.snapshot.stage = WeeklyChallengeStage::Settlement;
                }
                Child::Recovery(c) => {
                    c.resume()?;
                    self.snapshot.stage = WeeklyChallengeStage::Recovery;
                }
                Child::TimeoutExit(c) => c.resume()?,
            }
        }
        self.end = None;
        Ok(())
    }
    fn session<'a>(
        &mut self,
        slot: &'a mut Option<NativeSession>,
    ) -> Result<Option<&'a mut NativeSession>> {
        let combat = self.snapshot.stage == WeeklyChallengeStage::Combat;
        let mode = if combat {
            self.plan.config.mode
        } else {
            InputMode::Foreground
        };
        let device = if combat {
            self.plan
                .config
                .directml_device
                .map_or(InferenceDevice::Cpu, InferenceDevice::DirectMl)
        } else {
            InferenceDevice::Cpu
        };
        if self.loaded != Some(self.snapshot.stage) {
            self.recognizer = None;
        }
        match &self.target {
            BattleTarget::Fixture => {
                let n = slot.as_mut().context("周常挑战隔离控制器缺失")?;
                n.quiesce()?;
                n.check_ether_synthesis_target(false)?;
            }
            BattleTarget::Game { window, capture } => match NativeSession::handoff_game(
                slot,
                GameSessionConfig {
                    window,
                    resources: &self.resources.root,
                    capture: *capture,
                    input: mode,
                    device,
                },
                &self.control.cancellation(),
            )? {
                SessionHandoff::Cancelled => return Ok(None),
                SessionHandoff::Created => {
                    self.loaded = None;
                    self.recognizer = None;
                }
                SessionHandoff::Reused => (),
            },
        }
        let n = slot.as_mut().context("周常挑战控制器交接后缺失")?;
        n.check_combat_input_mode(mode)?;
        if self.control.cancellation().load(Ordering::Acquire) {
            return Ok(None);
        }
        if self.loaded != Some(self.snapshot.stage) {
            self.resources.verify()?;
            match self.snapshot.stage {
                WeeklyChallengeStage::Preparation => (),
                WeeklyChallengeStage::Deployment => {
                    n.load_simulation_deploy_bundle(&self.resources.deploy)?
                }
                WeeklyChallengeStage::Move | WeeklyChallengeStage::Combat => {
                    if self.snapshot.stage == WeeklyChallengeStage::Move {
                        n.load_challenge_detector(&self.resources.preparation.daily)?;
                    }
                    self.recognizer = Some(n.load_combat_bundle(&self.resources.combat)?)
                }
                WeeklyChallengeStage::Settlement | WeeklyChallengeStage::Recovery => {
                    n.load_deep_hunt_bundle(&self.resources.preparation.hunt)?;
                    if self.snapshot.stage == WeeklyChallengeStage::Settlement
                        && let Some(path) = &self.resources.agent_goal
                    {
                        n.load_agent_goal_bundle(path)?;
                    } else {
                        n.clear_agent_goal_bundle()?;
                    }
                }
                WeeklyChallengeStage::TimeoutExit => {
                    n.load_deep_hunt_bundle(&self.resources.preparation.hunt)?;
                    n.clear_agent_goal_bundle()?;
                    self.recognizer = Some(n.load_combat_bundle(&self.resources.combat)?);
                }
                WeeklyChallengeStage::Done => anyhow::bail!("周常挑战完成后不能交接控制器"),
            }
            self.loaded = Some(self.snapshot.stage);
        }
        Ok(Some(n))
    }
    pub fn execute_with_budget(
        &mut self,
        slot: &mut Option<NativeSession>,
        parent_remaining: Duration,
        mut publish: impl FnMut(WeeklyChallengeSnapshot) -> Result<()>,
    ) -> Result<WeeklyChallengeEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(WeeklyChallengeEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "周常挑战不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.request.timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<WeeklyChallengeEnd> {
            if let Some(n) = slot.as_mut() {
                n.quiesce()?;
            }
            self.loaded = None;
            self.recognizer = None;
            self.resources.verify()?;
            loop {
                if let Some(e) = self.interrupted() {
                    return Ok(e);
                }
                if remaining().is_zero() {
                    return Ok(WeeklyChallengeEnd::TimedOut);
                }
                if self.snapshot.stage == WeeklyChallengeStage::Done {
                    return Ok(if self.snapshot.recovery.is_some() {
                        WeeklyChallengeEnd::RecoveredFailure
                    } else {
                        WeeklyChallengeEnd::Completed
                    });
                }
                let Some(n) = self.session(slot)? else {
                    return Ok(self.interrupted().unwrap_or(WeeklyChallengeEnd::Stopped));
                };
                if let Some(e) = self.interrupted() {
                    return Ok(e);
                }
                if remaining().is_zero() {
                    return Ok(WeeklyChallengeEnd::TimedOut);
                }
                self.child_started = true;
                let snapshot = &mut self.snapshot;
                let mut save = |s: WeeklyChallengeSnapshot| publish(s);
                match self.child.as_mut().context("周常挑战当前子流程缺失")? {
                    Child::Preparation(c) => {
                        let result = c.execute_with_budget(
                            n,
                            matches!(self.target, BattleTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.preparation = s;
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.preparation = c.snapshot();
                        snapshot.inputs_released = snapshot.preparation.inputs_released;
                        match result? {
                            WeeklyPreparationEnd::Prepared => (),
                            WeeklyPreparationEnd::Exhausted => {
                                return Ok(WeeklyChallengeEnd::Exhausted);
                            }
                            WeeklyPreparationEnd::AwaitingVerification => {
                                return Ok(WeeklyChallengeEnd::AwaitingVerification);
                            }
                            WeeklyPreparationEnd::Paused => return Ok(WeeklyChallengeEnd::Paused),
                            WeeklyPreparationEnd::Stopped => {
                                return Ok(WeeklyChallengeEnd::Stopped);
                            }
                            WeeklyPreparationEnd::TimedOut => {
                                return Ok(WeeklyChallengeEnd::TimedOut);
                            }
                            WeeklyPreparationEnd::Failed => anyhow::bail!("周常准备失败"),
                        }
                        let receipt = c.receipt_for_parent().context("周常准备未正常保存或释放")?;
                        let request = WeeklyDeployRequest {
                            preparation: snapshot
                                .preparation
                                .preparation
                                .clone()
                                .context("周常准备检查点缺失")?,
                            team: receipt.teams,
                            timeout_ms: snapshot.request.timeout_ms,
                        };
                        let d = DeploymentRun::after_teams(
                            snapshot.id,
                            request,
                            TeamsEnd::Prepared,
                            snapshot
                                .preparation
                                .teams
                                .as_ref()
                                .context("周常选队检查点缺失")?,
                            self.prepared.take().context("周常冻结战斗脚本缺失")?,
                            self.control.clone(),
                        )?;
                        snapshot.deployment = Some(d.snapshot.clone());
                        self.child = Some(Child::Deployment(Box::new(d)));
                        self.snapshot.stage = WeeklyChallengeStage::Deployment;
                    }
                    Child::Deployment(c) => {
                        let result = c.execute_with_budget(
                            n,
                            matches!(self.target, BattleTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.deployment = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.deployment = Some(c.snapshot.clone());
                        snapshot.inputs_released = c.snapshot.inputs_released;
                        match result? {
                            DeploymentEnd::Entered => (),
                            DeploymentEnd::Paused => return Ok(WeeklyChallengeEnd::Paused),
                            DeploymentEnd::Stopped => return Ok(WeeklyChallengeEnd::Stopped),
                            DeploymentEnd::TimedOut => return Ok(WeeklyChallengeEnd::TimedOut),
                            DeploymentEnd::Failed => anyhow::bail!("周常挑战出战失败"),
                        }
                        let Some(Child::Deployment(c)) = self.child.take() else {
                            unreachable!()
                        };
                        let c = c.into_combat_for_parent()?;
                        self.snapshot.combat = Some(c.progress.clone());
                        self.child = Some(Child::Combat(Box::new(c)));
                        self.snapshot.movement = Some(
                            od_core::challenge_move::ChallengeMove::new(
                                false,
                                self.snapshot.request.plan.notorious_hunt_buff_num,
                            )
                            .map_err(anyhow::Error::msg)?,
                        );
                        self.snapshot.stage = WeeklyChallengeStage::Move;
                    }
                    Child::Combat(c) => {
                        if snapshot.stage == WeeklyChallengeStage::Move {
                            let mut movement =
                                snapshot.movement.clone().context("周常挑战战前移动缺失")?;
                            let move_started = Instant::now();
                            let move_initial = snapshot.movement_ms;
                            let move_budget =
                                Duration::from_millis(120_000_u64.saturating_sub(move_initial))
                                    .min(remaining());
                            snapshot.inputs_released = false;
                            let end = n.execute_challenge_move(
                                self.recognizer.as_ref().context("周常挑战移动识别器缺失")?,
                                &mut movement,
                                maa_adapter::challenge_move::MoveOptions {
                                    game: matches!(self.target, BattleTarget::Game { .. }),
                                    timeout: move_budget,
                                    turn_sensitivity_milli: self.plan.config.turn_sensitivity_milli,
                                    max_frame_age_ms: 1000,
                                },
                                self.control.cancellation(),
                                |m| {
                                    snapshot.movement = Some(m);
                                    snapshot.movement_ms = move_initial
                                        .saturating_add(move_started.elapsed().as_millis() as u64);
                                    snapshot.active_ms = initial
                                        .saturating_add(started.elapsed().as_millis() as u64);
                                    save(snapshot.clone())
                                },
                            );
                            snapshot.movement = Some(movement);
                            snapshot.movement_ms = move_initial
                                .saturating_add(move_started.elapsed().as_millis() as u64);
                            snapshot.inputs_released = n.release_inputs().is_ok();
                            ensure!(snapshot.inputs_released, "周常挑战战前移动输入未释放");
                            match end? {
                                maa_adapter::challenge_move::MoveEnd::Completed => (),
                                maa_adapter::challenge_move::MoveEnd::TimedOut
                                    if snapshot.movement_ms >= 120_000
                                        && !remaining().is_zero()
                                        && self.control.requested().is_none() =>
                                {
                                    let c = timeout_child(
                                        snapshot,
                                        WeeklyTimeoutCause::Movement,
                                        self.control.clone(),
                                    )?;
                                    snapshot.timeout_exit = Some(c.snapshot.clone());
                                    snapshot.stage = WeeklyChallengeStage::TimeoutExit;
                                    self.child = Some(Child::TimeoutExit(Box::new(c)));
                                    self.child_started = false;
                                    save(snapshot.clone())?;
                                    continue;
                                }
                                maa_adapter::challenge_move::MoveEnd::TimedOut => {
                                    return Ok(WeeklyChallengeEnd::TimedOut);
                                }
                                maa_adapter::challenge_move::MoveEnd::Cancelled => {
                                    return Ok(match self.control.requested() {
                                        Some(Interrupt::Pause) => WeeklyChallengeEnd::Paused,
                                        _ => WeeklyChallengeEnd::Stopped,
                                    });
                                }
                            }
                            snapshot.stage = WeeklyChallengeStage::Combat;
                            save(snapshot.clone())?;
                            continue;
                        }
                        let result = c.execute_segment_with_budget(
                            n,
                            self.recognizer.as_mut().context("周常挑战识别器缺失")?,
                            matches!(self.target, BattleTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.combat = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.combat = Some(c.progress.clone());
                        snapshot.inputs_released = c.progress.inputs_released;
                        let result = match result {
                            Err(error)
                                if error.downcast_ref::<CombatTimeout>()
                                    == Some(&CombatTimeout::BattleLimit)
                                    && snapshot.inputs_released
                                    && !remaining().is_zero()
                                    && self.control.requested().is_none() =>
                            {
                                let c = timeout_child(
                                    snapshot,
                                    WeeklyTimeoutCause::Combat,
                                    self.control.clone(),
                                )?;
                                snapshot.timeout_exit = Some(c.snapshot.clone());
                                snapshot.stage = WeeklyChallengeStage::TimeoutExit;
                                self.child = Some(Child::TimeoutExit(Box::new(c)));
                                self.child_started = false;
                                save(snapshot.clone())?;
                                continue;
                            }
                            other => other?,
                        };
                        match result {
                            CombatSegmentEnd::Result(_) => (),
                            CombatSegmentEnd::Interrupted(Interrupt::Pause) => {
                                return Ok(WeeklyChallengeEnd::Paused);
                            }
                            CombatSegmentEnd::Interrupted(Interrupt::Stop) => {
                                return Ok(WeeklyChallengeEnd::Stopped);
                            }
                        }
                        if result == CombatSegmentEnd::Result(NormalBattleResult::Retreat) {
                            let c = WeeklyRecoveryRun::after_combat(
                                result,
                                self.snapshot.inputs_released,
                                self.snapshot.request.timeout_ms.min(300_000),
                                self.snapshot
                                    .request
                                    .plan
                                    .execution_mission_type()
                                    .to_owned(),
                                self.snapshot
                                    .preparation
                                    .preparation
                                    .as_ref()
                                    .context("周常失败收尾缺少免费余量前置")?
                                    .rewards_left(),
                                self.control.clone(),
                            )?;
                            self.snapshot.recovery = Some(c.snapshot.clone());
                            self.child = Some(Child::Recovery(Box::new(c)));
                            self.snapshot.stage = WeeklyChallengeStage::Recovery;
                            self.child_started = false;
                            save(self.snapshot.clone())?;
                            continue;
                        }
                        let mut c = WeeklySettlementRun::after_combat(
                            result,
                            self.snapshot.inputs_released,
                            self.snapshot.request.timeout_ms.min(300_000),
                            self.snapshot
                                .request
                                .plan
                                .execution_mission_type()
                                .to_owned(),
                            self.snapshot
                                .preparation
                                .preparation
                                .as_ref()
                                .context("周常免费余量前置缺失")?
                                .rewards_left(),
                            self.control.clone(),
                        )?;
                        if self.resources.agent_goal.is_some() {
                            c.enable_agent_goal()?;
                        }
                        self.snapshot.settlement = Some(c.snapshot.clone());
                        self.child = Some(Child::Settlement(Box::new(c)));
                        self.snapshot.stage = WeeklyChallengeStage::Settlement;
                    }
                    Child::TimeoutExit(c) => {
                        let result = c.execute_with_budget(
                            n,
                            self.recognizer.as_ref().context("周常超时退出识别器缺失")?,
                            matches!(self.target, BattleTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.timeout_exit = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.timeout_exit = Some(c.snapshot.clone());
                        snapshot.inputs_released = c.snapshot.inputs_released;
                        match result? {
                            WeeklyTimeoutEnd::Finished => (),
                            WeeklyTimeoutEnd::Paused => return Ok(WeeklyChallengeEnd::Paused),
                            WeeklyTimeoutEnd::Stopped => return Ok(WeeklyChallengeEnd::Stopped),
                            WeeklyTimeoutEnd::TimedOut => return Ok(WeeklyChallengeEnd::TimedOut),
                            WeeklyTimeoutEnd::Failed => anyhow::bail!("周常超时退出失败"),
                        }
                        let recovery =
                            WeeklyRecoveryRun::after_timeout_exit(c, self.control.clone())?;
                        snapshot.recovery = Some(recovery.snapshot.clone());
                        snapshot.stage = WeeklyChallengeStage::Recovery;
                        self.child = Some(Child::Recovery(Box::new(recovery)));
                    }
                    Child::Recovery(c) => {
                        let result = c.execute_with_budget(
                            n,
                            matches!(self.target, BattleTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.recovery = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.recovery = Some(c.snapshot.clone());
                        snapshot.inputs_released = c.snapshot.inputs_released;
                        match result? {
                            WeeklyRecoveryEnd::Returned => (),
                            WeeklyRecoveryEnd::Paused => return Ok(WeeklyChallengeEnd::Paused),
                            WeeklyRecoveryEnd::Stopped => return Ok(WeeklyChallengeEnd::Stopped),
                            WeeklyRecoveryEnd::TimedOut => return Ok(WeeklyChallengeEnd::TimedOut),
                            WeeklyRecoveryEnd::Failed => anyhow::bail!("周常挑战失败收尾未完成"),
                        }
                        ensure!(
                            c.snapshot.flow.completed() && snapshot.inputs_released,
                            "周常失败收尾尚未返回原详情或释放输入"
                        );
                        snapshot.stage = WeeklyChallengeStage::Done;
                    }
                    Child::Settlement(c) => {
                        let result = c.execute_with_budget(
                            n,
                            matches!(self.target, BattleTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.settlement = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.settlement = Some(c.snapshot.clone());
                        snapshot.inputs_released = c.snapshot.inputs_released;
                        match result? {
                            WeeklySettlementEnd::Collected => (),
                            WeeklySettlementEnd::Paused => return Ok(WeeklyChallengeEnd::Paused),
                            WeeklySettlementEnd::Stopped => return Ok(WeeklyChallengeEnd::Stopped),
                            WeeklySettlementEnd::TimedOut => {
                                return Ok(WeeklyChallengeEnd::TimedOut);
                            }
                            WeeklySettlementEnd::Failed => anyhow::bail!("周常挑战结算失败"),
                        }
                        ensure!(
                            c.snapshot.flow.completed() && self.snapshot.inputs_released,
                            "周常挑战尚未实际返回或释放输入"
                        );
                        self.snapshot.stage = WeeklyChallengeStage::Done;
                    }
                }
                self.child_started = false;
                if self.snapshot.stage == WeeklyChallengeStage::Done {
                    self.child_started = true;
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                save(self.snapshot.clone())?;
            }
        })();
        self.snapshot.inputs_released = slot.as_mut().is_none_or(|n| n.release_inputs().is_ok());
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        self.end = Some(WeeklyChallengeEnd::Failed);
        ensure!(
            self.snapshot.inputs_released,
            "周常挑战收尾不能确认释放输入"
        );
        let mut end = result?;
        publish(self.snapshot.clone())?;
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(e) = self.interrupted() {
            end = e;
        } else if remaining().is_zero() {
            end = WeeklyChallengeEnd::TimedOut;
        }
        if end == WeeklyChallengeEnd::Paused && !self.control.acknowledge_pause() {
            end = WeeklyChallengeEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}

fn timeout_child(
    snapshot: &WeeklyChallengeSnapshot,
    cause: WeeklyTimeoutCause,
    control: PipelineControl,
) -> Result<WeeklyTimeoutRun> {
    ensure!(
        snapshot.inputs_released
            && snapshot
                .deployment
                .as_ref()
                .is_some_and(|s| s.flow.completed()),
        "周常超时退出缺少本次出战或输入释放证明"
    );
    let flow = WeeklyTimeoutFlow::new(WeeklyTimeoutRequest {
        cause,
        mission_type: snapshot.request.plan.execution_mission_type().to_owned(),
        rewards_before: snapshot
            .preparation
            .preparation
            .as_ref()
            .context("周常超时退出缺少本次免费余量")?
            .rewards_left(),
        timeout_ms: 60_000,
    })
    .map_err(anyhow::Error::msg)?;
    WeeklyTimeoutRun::new(flow, control)
}
