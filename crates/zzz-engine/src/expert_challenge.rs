//! 专业挑战室父挑战，目标类型与模拟室分开保存。
use crate::{
    charge_transport::{TrainingTransportEnd, TrainingTransportRun, TrainingTransportSnapshot},
    combat::{CombatRun, CombatSegmentEnd},
    expert_challenge_entry::{ExpertEntryEnd, ExpertEntryRun, ExpertEntrySnapshot},
    expert_challenge_settlement::{
        ExpertSettlementEnd, ExpertSettlementRun, ExpertSettlementSnapshot,
    },
    simulation_battle::BattleTarget,
    simulation_deploy::{DeploymentEnd, DeploymentRun, DeploymentSnapshot, PreparedDeployment},
    simulation_teams::{TeamsEnd, TeamsRun, TeamsSnapshot},
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{
    GameSessionConfig, InferenceDevice, NativeSession, SessionHandoff,
    combat_vision::{CombatBundle, CombatRecognizer},
};
pub use od_core::expert_challenge::ExpertRequest;
use od_core::{
    InputMode,
    app_combat::AppCombatPlan,
    battle_result::NormalBattleResult,
    charge_plan::ChargePlanItem,
    charge_transport::TrainingTransportRequest,
    combat::CombatProgress,
    expert_challenge::ExpertDeployRequest,
    pipeline::{Interrupt, PipelineControl},
    simulation_cards::CardsCharge,
    simulation_teams::TeamsFlow,
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
pub enum ExpertStage {
    Transport,
    Entry,
    Teams,
    Deployment,
    Combat,
    Settlement,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpertEnd {
    Completed,
    InsufficientCharge,
    Paused,
    Stopped,
    TimedOut,
    Failed,
    AwaitingVerification,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertSnapshot {
    pub id: Uuid,
    pub request: ExpertRequest,
    pub stage: ExpertStage,
    pub transport: TrainingTransportSnapshot,
    pub entry: Option<ExpertEntrySnapshot>,
    pub teams: Option<TeamsSnapshot>,
    pub deployment: Option<DeploymentSnapshot<ExpertDeployRequest>>,
    pub combat: Option<CombatProgress>,
    pub settlement: Option<ExpertSettlementSnapshot>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpertProof {
    pub run_id: Uuid,
    pub execution_id: Uuid,
    pub plan: ChargePlanItem,
    pub charge_after: CardsCharge,
}
#[derive(Clone)]
pub(crate) struct ExpertResources {
    root: PathBuf,
    route: PathBuf,
    expert: PathBuf,
    simulation: PathBuf,
    daily: PathBuf,
    restore: PathBuf,
    deploy: PathBuf,
    combat: PathBuf,
    hashes: Vec<(PathBuf, Vec<u8>)>,
}
impl ExpertResources {
    pub(crate) fn load(root: &Path, combat: &Path) -> Result<Self> {
        od_core::config::reject_links(root)?;
        let root = root.canonicalize()?;
        let route = maa_adapter::charge_transport::validate_charge_transport_bundle(
            &root.join("generated/charge-transport"),
        )?;
        let expert = maa_adapter::expert_challenge::validate_expert_challenge_bundle(
            &root.join("generated/expert-challenge"),
        )?;
        let simulation = maa_adapter::simulation::validate_simulation_bundle(
            &root.join("generated/simulation"),
        )?;
        let daily = maa_adapter::validate_basic_daily_bundle(&root.join("generated/basic-daily"))?;
        let restore = maa_adapter::restore_charge::validate_restore_charge_bundle(
            &root.join("generated/restore-charge"),
        )?;
        let deploy = maa_adapter::simulation_deploy::validate_simulation_deploy_bundle(
            &root.join("generated/simulation-deploy"),
        )?;
        let combat = CombatBundle::load(combat)?.root;
        let hashes = [
            route.join("charge_transport-manifest.json"),
            expert.join("expert_challenge-manifest.json"),
            simulation.join("simulation-manifest.json"),
            daily.join("basic-daily-manifest.json"),
            restore.join("restore-charge-manifest.json"),
            deploy.join("simulation_deploy-manifest.json"),
            combat.join("combat-manifest.json"),
        ]
        .into_iter()
        .map(|p| Ok((p.clone(), Sha256::digest(std::fs::read(p)?).to_vec())))
        .collect::<Result<_>>()?;
        Ok(Self {
            root,
            route,
            expert,
            simulation,
            daily,
            restore,
            deploy,
            combat,
            hashes,
        })
    }
    pub(crate) fn verify(&self) -> Result<()> {
        for (p, hash) in &self.hashes {
            od_core::config::reject_links(p)?;
            ensure!(
                Sha256::digest(std::fs::read(p)?).as_slice() == hash,
                "专业挑战室冻结资源已变化"
            );
        }
        maa_adapter::charge_transport::validate_charge_transport_bundle(&self.route)?;
        maa_adapter::expert_challenge::validate_expert_challenge_bundle(&self.expert)?;
        maa_adapter::simulation::validate_simulation_bundle(&self.simulation)?;
        maa_adapter::validate_basic_daily_bundle(&self.daily)?;
        maa_adapter::restore_charge::validate_restore_charge_bundle(&self.restore)?;
        maa_adapter::simulation_deploy::validate_simulation_deploy_bundle(&self.deploy)?;
        CombatBundle::load(&self.combat)?;
        Ok(())
    }
}
enum Child {
    Transport(Box<TrainingTransportRun>),
    Entry(Box<ExpertEntryRun>),
    Teams(Box<TeamsRun>),
    Deployment(Box<DeploymentRun<ExpertDeployRequest>>),
    Combat(Box<CombatRun>),
    Settlement(Box<ExpertSettlementRun>),
}
pub struct ExpertChallengeRun {
    agent_goal_bundle: Option<PathBuf>,
    snapshot: ExpertSnapshot,
    execution_id: Uuid,
    resources: ExpertResources,
    plan: AppCombatPlan,
    prepared: Option<PreparedDeployment>,
    target: BattleTarget,
    control: PipelineControl,
    child: Option<Child>,
    child_started: bool,
    loaded: Option<ExpertStage>,
    recognizer: Option<CombatRecognizer>,
    end: Option<ExpertEnd>,
}
impl ExpertChallengeRun {
    pub(crate) fn after_agent_transport(
        id: Uuid,
        request: ExpertRequest,
        resources: ExpertResources,
        prepared: PreparedDeployment,
        target: BattleTarget,
        control: PipelineControl,
        handoff: crate::charge_transport::AgentRouteHandoff<'_>,
    ) -> Result<Self> {
        let goal_bundle =
            maa_adapter::agent_goal::validate_agent_goal_bundle(&handoff.goal_bundle)?;
        let route = TrainingTransportRun::from_agent_route(
            TrainingTransportRequest {
                plan: request.plan.clone(),
                timeout_ms: request.timeout_ms,
            },
            handoff.route,
            control.clone(),
        )?;
        let entry = ExpertEntryRun::after_transport(request.clone(), &route, control.clone())?;
        let mut run = Self::from_frozen(id, request, resources, prepared, target, control)?;
        run.agent_goal_bundle = Some(goal_bundle);
        run.snapshot.transport = route.snapshot();
        run.snapshot.entry = Some(entry.snapshot());
        run.snapshot.stage = ExpertStage::Entry;
        run.child = Some(Child::Entry(Box::new(entry)));
        run.child_started = false;
        Ok(run)
    }
    pub(crate) fn agent_completion_for_parent(
        &self,
    ) -> Option<(ExpertProof, od_core::agent_goal::AgentGoalStatus)> {
        if self.agent_goal_bundle.is_none()
            || !self.snapshot.inputs_released
            || !matches!(
                self.snapshot.stage,
                ExpertStage::Settlement | ExpertStage::Done
            )
        {
            return None;
        }
        let settlement = self.snapshot.settlement.as_ref()?;
        settlement.flow.validate().ok()?;
        if !settlement.flow.completed() {
            return None;
        }
        let goal = settlement.agent_goal.as_ref()?;
        goal.validate().ok()?;
        Some((
            ExpertProof {
                run_id: self.snapshot.id,
                execution_id: self.execution_id,
                plan: self.snapshot.request.plan.clone(),
                charge_after: settlement.flow.charge()?,
            },
            goal.observed()?,
        ))
    }

    pub fn new(
        id: Uuid,
        request: ExpertRequest,
        root: &Path,
        prepared: PreparedDeployment,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<Self> {
        let resources = ExpertResources::load(root, prepared.resource_root()?)?;
        Self::from_frozen(id, request, resources, prepared, target, control)
    }
    pub(crate) fn from_frozen(
        id: Uuid,
        request: ExpertRequest,
        resources: ExpertResources,
        prepared: PreparedDeployment,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(!id.is_nil(), "专业挑战室缺少运行身份");
        request.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            prepared.plan().config.script
                == request
                    .plan
                    .script(&request.teams)
                    .map_err(anyhow::Error::msg)?,
            "专业挑战室战斗脚本与冻结计划不一致"
        );
        if matches!(target, BattleTarget::Fixture) {
            ensure!(
                prepared.plan().config.mode == InputMode::Foreground
                    && prepared.plan().config.directml_device.is_none(),
                "隔离专业挑战室只复用前台 CPU 控制器"
            );
        }
        if let BattleTarget::Game { window, .. } = &target {
            ensure!(
                window.width == 1920 && window.height == 1080,
                "专业挑战室须冻结 1080p 窗口"
            );
        }
        resources.verify()?;
        ensure!(
            resources.combat == prepared.resource_root()?,
            "区域冻结战斗资源不一致"
        );
        let plan = prepared.plan().clone();
        let transport = TrainingTransportRun::new(
            TrainingTransportRequest {
                plan: request.plan.clone(),
                timeout_ms: request.timeout_ms,
            },
            control.clone(),
        )?;
        Ok(Self {
            agent_goal_bundle: None,
            snapshot: ExpertSnapshot {
                id,
                request,
                stage: ExpertStage::Transport,
                transport: transport.snapshot(),
                entry: None,
                teams: None,
                deployment: None,
                combat: None,
                settlement: None,
                active_ms: 0,
                inputs_released: true,
            },
            execution_id: Uuid::new_v4(),
            resources,
            plan,
            prepared: Some(prepared),
            target,
            control,
            child: Some(Child::Transport(Box::new(transport))),
            child_started: false,
            loaded: None,
            recognizer: None,
            end: None,
        })
    }
    pub fn snapshot(&self) -> ExpertSnapshot {
        self.snapshot.clone()
    }
    pub fn uses_control(&self, c: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &c.cancellation())
    }
    pub fn proof(&self) -> Option<ExpertProof> {
        if self.control.requested().is_some() {
            return None;
        }
        self.proof_for_parent()
    }
    pub(crate) fn proof_for_parent(&self) -> Option<ExpertProof> {
        if self.end != Some(ExpertEnd::Completed)
            || !self.snapshot.inputs_released
            || self.snapshot.stage != ExpertStage::Done
        {
            return None;
        }
        Some(ExpertProof {
            run_id: self.snapshot.id,
            execution_id: self.execution_id,
            plan: self.snapshot.request.plan.clone(),
            charge_after: self.snapshot.settlement.as_ref()?.flow.charge()?,
        })
    }
    pub fn shortage(&self) -> Option<CardsCharge> {
        self.control
            .requested()
            .is_none()
            .then(|| self.shortage_for_parent())
            .flatten()
    }
    pub(crate) fn shortage_for_parent(&self) -> Option<CardsCharge> {
        (self.end == Some(ExpertEnd::InsufficientCharge) && self.snapshot.inputs_released)
            .then(|| self.snapshot.entry.as_ref()?.flow.charge())
            .flatten()
    }
    pub(crate) fn execution_id(&self) -> Uuid {
        self.execution_id
    }
    fn interrupted(&self) -> Option<ExpertEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => ExpertEnd::Paused,
            Interrupt::Stop => ExpertEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(ExpertEnd::Paused) && self.snapshot.inputs_released,
            "专业挑战室尚未安全暂停"
        );
        if !self.child_started {
            ensure!(self.control.resume(), "停止不能继续");
        } else {
            match self.child.as_mut().context("专业挑战室子流程缺失")? {
                Child::Transport(c) => c.resume_after_parent_pause()?,
                Child::Entry(c) => c.resume()?,
                Child::Teams(c) => c.resume()?,
                Child::Deployment(c) => c.resume()?,
                Child::Combat(c) => c.resume_segment_after_parent_pause()?,
                Child::Settlement(c) => {
                    c.resume()?;
                    self.snapshot.stage = ExpertStage::Settlement;
                }
            }
        }
        self.end = None;
        Ok(())
    }
    fn session<'a>(
        &mut self,
        slot: &'a mut Option<NativeSession>,
    ) -> Result<Option<&'a mut NativeSession>> {
        let combat = self.snapshot.stage == ExpertStage::Combat;
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
                let n = slot.as_mut().context("专业挑战室隔离控制器缺失")?;
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
        let n = slot.as_mut().context("专业挑战室控制器交接后缺失")?;
        n.check_combat_input_mode(mode)?;
        if self.control.cancellation().load(Ordering::Acquire) {
            return Ok(None);
        }
        if self.loaded != Some(self.snapshot.stage) {
            self.resources.verify()?;
            match self.snapshot.stage {
                ExpertStage::Transport => n.load_charge_transport_bundle(&self.resources.route)?,
                ExpertStage::Entry => {
                    n.clear_agent_goal_bundle()?;
                    n.load_expert_challenge_bundle(&self.resources.expert)?;
                    n.load_restore_charge_bundle(&self.resources.restore)?;
                }
                ExpertStage::Teams => {
                    n.load_simulation_bundle(&self.resources.simulation)?;
                    n.load_basic_daily_bundle(&self.resources.daily)?;
                }
                ExpertStage::Deployment => {
                    n.load_simulation_deploy_bundle(&self.resources.deploy)?
                }
                ExpertStage::Combat => {
                    self.recognizer = Some(n.load_combat_bundle(&self.resources.combat)?)
                }
                ExpertStage::Settlement => {
                    n.load_expert_challenge_bundle(&self.resources.expert)?;
                    if let Some(path) = &self.agent_goal_bundle {
                        n.load_agent_goal_bundle(path)?;
                    } else {
                        n.clear_agent_goal_bundle()?;
                    }
                }
                ExpertStage::Done => anyhow::bail!("专业挑战室完成后不能交接控制器"),
            }
            self.loaded = Some(self.snapshot.stage);
        }
        Ok(Some(n))
    }
    pub fn execute_with_budget(
        &mut self,
        slot: &mut Option<NativeSession>,
        parent_remaining: Duration,
        mut publish: impl FnMut(ExpertSnapshot) -> Result<()>,
    ) -> Result<ExpertEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(ExpertEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "专业挑战室不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.request.timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<ExpertEnd> {
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
                    return Ok(ExpertEnd::TimedOut);
                }
                if self.snapshot.stage == ExpertStage::Done {
                    return Ok(ExpertEnd::Completed);
                }
                let Some(n) = self.session(slot)? else {
                    return Ok(self.interrupted().unwrap_or(ExpertEnd::Stopped));
                };
                if let Some(e) = self.interrupted() {
                    return Ok(e);
                }
                if remaining().is_zero() {
                    return Ok(ExpertEnd::TimedOut);
                }
                self.child_started = true;
                let snapshot = &mut self.snapshot;
                let mut save = |s: ExpertSnapshot| publish(s);
                match self.child.as_mut().context("专业挑战室当前子流程缺失")? {
                    Child::Transport(c) => {
                        let result = c.execute_with_budget(
                            n,
                            matches!(self.target, BattleTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.transport = s;
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.transport = c.snapshot();
                        snapshot.inputs_released = snapshot.transport.inputs_released;
                        match result? {
                            TrainingTransportEnd::AwaitingVerification => (),
                            TrainingTransportEnd::Paused => return Ok(ExpertEnd::Paused),
                            TrainingTransportEnd::Stopped => return Ok(ExpertEnd::Stopped),
                            TrainingTransportEnd::TimedOut => return Ok(ExpertEnd::TimedOut),
                            TrainingTransportEnd::Failed => anyhow::bail!("专业挑战室传送失败"),
                        }
                        let entry = ExpertEntryRun::after_transport(
                            self.snapshot.request.clone(),
                            c,
                            self.control.clone(),
                        )?;
                        self.snapshot.entry = Some(entry.snapshot());
                        self.child = Some(Child::Entry(Box::new(entry)));
                        self.snapshot.stage = ExpertStage::Entry;
                    }
                    Child::Entry(c) => {
                        let result = c.execute_with_budget(
                            n,
                            matches!(self.target, BattleTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.entry = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.entry = Some(c.snapshot());
                        snapshot.inputs_released = c.snapshot().inputs_released;
                        match result? {
                            ExpertEntryEnd::Ready => {
                                ensure!(c.ready_for_parent(), "专业挑战室详情未正常保存或释放")
                            }
                            ExpertEntryEnd::Insufficient => {
                                ensure!(c.shortage_for_parent(), "专业挑战室缺少实际不足证明");
                                return Ok(ExpertEnd::InsufficientCharge);
                            }
                            ExpertEntryEnd::Paused => return Ok(ExpertEnd::Paused),
                            ExpertEntryEnd::Stopped => return Ok(ExpertEnd::Stopped),
                            ExpertEntryEnd::TimedOut => return Ok(ExpertEnd::TimedOut),
                            ExpertEntryEnd::Failed => anyhow::bail!("专业挑战室详情准备失败"),
                            ExpertEntryEnd::AwaitingVerification => {
                                return Ok(ExpertEnd::AwaitingVerification);
                            }
                        }
                        let c = TeamsRun::new(
                            TeamsFlow::new(
                                self.snapshot
                                    .request
                                    .team_request()
                                    .map_err(anyhow::Error::msg)?,
                            )
                            .map_err(anyhow::Error::msg)?,
                            self.control.clone(),
                        )?;
                        self.snapshot.teams = Some(c.snapshot.clone());
                        self.child = Some(Child::Teams(Box::new(c)));
                        self.snapshot.stage = ExpertStage::Teams;
                    }
                    Child::Teams(c) => {
                        let result = c.execute_with_budget(
                            n,
                            matches!(self.target, BattleTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.teams = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.teams = Some(c.snapshot.clone());
                        snapshot.inputs_released = c.snapshot.inputs_released;
                        match result? {
                            TeamsEnd::Prepared => (),
                            TeamsEnd::Paused => return Ok(ExpertEnd::Paused),
                            TeamsEnd::Stopped => return Ok(ExpertEnd::Stopped),
                            TeamsEnd::TimedOut => return Ok(ExpertEnd::TimedOut),
                            TeamsEnd::Failed => anyhow::bail!("专业挑战室选队失败"),
                        }
                        let entry = self
                            .snapshot
                            .entry
                            .as_ref()
                            .context("专业挑战室详情检查点缺失")?
                            .flow
                            .clone();
                        let team = c.snapshot.flow.result().context("专业挑战室选队结果缺失")?;
                        let d = DeploymentRun::after_teams(
                            self.snapshot.id,
                            ExpertDeployRequest {
                                entry,
                                team,
                                timeout_ms: self.snapshot.request.timeout_ms,
                            },
                            TeamsEnd::Prepared,
                            &c.snapshot,
                            self.prepared.take().context("专业挑战室冻结战斗脚本缺失")?,
                            self.control.clone(),
                        )?;
                        self.snapshot.deployment = Some(d.snapshot.clone());
                        self.child = Some(Child::Deployment(Box::new(d)));
                        self.snapshot.stage = ExpertStage::Deployment;
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
                            DeploymentEnd::Paused => return Ok(ExpertEnd::Paused),
                            DeploymentEnd::Stopped => return Ok(ExpertEnd::Stopped),
                            DeploymentEnd::TimedOut => return Ok(ExpertEnd::TimedOut),
                            DeploymentEnd::Failed => anyhow::bail!("专业挑战室出战失败"),
                        }
                        let Some(Child::Deployment(c)) = self.child.take() else {
                            unreachable!()
                        };
                        let c = c.into_combat_for_parent()?;
                        self.snapshot.combat = Some(c.progress.clone());
                        self.child = Some(Child::Combat(Box::new(c)));
                        self.snapshot.stage = ExpertStage::Combat;
                    }
                    Child::Combat(c) => {
                        let result = c.execute_segment_with_budget(
                            n,
                            self.recognizer.as_mut().context("专业挑战室识别器缺失")?,
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
                        let result = result?;
                        match result {
                            CombatSegmentEnd::Result(NormalBattleResult::Completed) => (),
                            CombatSegmentEnd::Result(NormalBattleResult::Retreat) => {
                                anyhow::bail!("专业挑战室战斗撤退，不能登记成功")
                            }
                            CombatSegmentEnd::Interrupted(Interrupt::Pause) => {
                                return Ok(ExpertEnd::Paused);
                            }
                            CombatSegmentEnd::Interrupted(Interrupt::Stop) => {
                                return Ok(ExpertEnd::Stopped);
                            }
                        }
                        let mut c = ExpertSettlementRun::after_combat(
                            result,
                            self.snapshot.inputs_released,
                            self.snapshot.request.timeout_ms.min(300_000),
                            self.control.clone(),
                        )?;
                        if self.agent_goal_bundle.is_some() {
                            c.enable_agent_goal()?;
                        }
                        self.snapshot.settlement = Some(c.snapshot.clone());
                        self.child = Some(Child::Settlement(Box::new(c)));
                        self.snapshot.stage = ExpertStage::Settlement;
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
                            ExpertSettlementEnd::Collected => (),
                            ExpertSettlementEnd::Paused => return Ok(ExpertEnd::Paused),
                            ExpertSettlementEnd::Stopped => return Ok(ExpertEnd::Stopped),
                            ExpertSettlementEnd::TimedOut => return Ok(ExpertEnd::TimedOut),
                            ExpertSettlementEnd::Failed => anyhow::bail!("专业挑战室结算失败"),
                        }
                        ensure!(
                            c.snapshot.flow.completed() && self.snapshot.inputs_released,
                            "专业挑战室尚未实际返回或释放输入"
                        );
                        self.snapshot.stage = ExpertStage::Done;
                    }
                }
                self.child_started = false;
                if self.snapshot.stage == ExpertStage::Done {
                    self.child_started = true;
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                save(self.snapshot.clone())?;
            }
        })();
        self.snapshot.inputs_released = slot.as_mut().is_none_or(|n| n.release_inputs().is_ok());
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        self.end = Some(ExpertEnd::Failed);
        ensure!(
            self.snapshot.inputs_released,
            "专业挑战室收尾不能确认释放输入"
        );
        let mut end = result?;
        publish(self.snapshot.clone())?;
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(e) = self.interrupted() {
            end = e;
        } else if remaining().is_zero() {
            end = ExpertEnd::TimedOut;
        }
        if end == ExpertEnd::Paused && !self.control.acknowledge_pause() {
            end = ExpertEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
