//! 区域巡防父挑战，目标类型与模拟室分开保存。
use crate::{
    area_patrol_entry::{PatrolEntryEnd, PatrolEntryRun, PatrolEntrySnapshot},
    area_patrol_settlement::{PatrolSettlementEnd, PatrolSettlementRun, PatrolSettlementSnapshot},
    charge_transport::{TrainingTransportEnd, TrainingTransportRun, TrainingTransportSnapshot},
    combat::{CombatRun, CombatSegmentEnd},
    simulation_battle::BattleTarget,
    simulation_deploy::{DeploymentEnd, DeploymentRun, DeploymentSnapshot, PreparedDeployment},
    simulation_teams::{TeamsEnd, TeamsRun, TeamsSnapshot},
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{
    GameSessionConfig, InferenceDevice, NativeSession, SessionHandoff,
    combat_vision::{CombatBundle, CombatRecognizer},
};
pub use od_core::area_patrol::PatrolRequest;
use od_core::{
    InputMode,
    app_combat::AppCombatPlan,
    area_patrol::PatrolDeployRequest,
    battle_result::NormalBattleResult,
    charge_plan::ChargePlanItem,
    charge_transport::TrainingTransportRequest,
    combat::CombatProgress,
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
pub enum PatrolStage {
    Transport,
    Entry,
    Teams,
    Deployment,
    Combat,
    Settlement,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatrolEnd {
    Completed,
    InsufficientCharge,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatrolSnapshot {
    pub id: Uuid,
    pub request: PatrolRequest,
    pub stage: PatrolStage,
    pub transport: TrainingTransportSnapshot,
    pub entry: Option<PatrolEntrySnapshot>,
    pub teams: Option<TeamsSnapshot>,
    pub deployment: Option<DeploymentSnapshot<PatrolDeployRequest>>,
    pub combat: Option<CombatProgress>,
    pub settlement: Option<PatrolSettlementSnapshot>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatrolProof {
    pub run_id: Uuid,
    pub execution_id: Uuid,
    pub plan: ChargePlanItem,
    pub charge_after: CardsCharge,
}
#[derive(Clone)]
pub(crate) struct PatrolResources {
    root: PathBuf,
    route: PathBuf,
    patrol: PathBuf,
    simulation: PathBuf,
    daily: PathBuf,
    restore: PathBuf,
    deploy: PathBuf,
    combat: PathBuf,
    hashes: Vec<(PathBuf, Vec<u8>)>,
}
impl PatrolResources {
    pub(crate) fn load(root: &Path, combat: &Path) -> Result<Self> {
        od_core::config::reject_links(root)?;
        let root = root.canonicalize()?;
        let route = maa_adapter::charge_transport::validate_charge_transport_bundle(
            &root.join("generated/charge-transport"),
        )?;
        let patrol = maa_adapter::area_patrol::validate_area_patrol_bundle(
            &root.join("generated/area-patrol"),
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
            patrol.join("area_patrol-manifest.json"),
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
            patrol,
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
                "区域巡防冻结资源已变化"
            );
        }
        maa_adapter::charge_transport::validate_charge_transport_bundle(&self.route)?;
        maa_adapter::area_patrol::validate_area_patrol_bundle(&self.patrol)?;
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
    Entry(Box<PatrolEntryRun>),
    Teams(Box<TeamsRun>),
    Deployment(Box<DeploymentRun<PatrolDeployRequest>>),
    Combat(Box<CombatRun>),
    Settlement(Box<PatrolSettlementRun>),
}
pub struct AreaPatrolRun {
    agent_goal_bundle: Option<PathBuf>,
    snapshot: PatrolSnapshot,
    execution_id: Uuid,
    resources: PatrolResources,
    plan: AppCombatPlan,
    prepared: Option<PreparedDeployment>,
    target: BattleTarget,
    control: PipelineControl,
    child: Option<Child>,
    child_started: bool,
    loaded: Option<PatrolStage>,
    recognizer: Option<CombatRecognizer>,
    end: Option<PatrolEnd>,
}
impl AreaPatrolRun {
    pub(crate) fn after_agent_transport(
        id: Uuid,
        request: PatrolRequest,
        resources: PatrolResources,
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
        let entry = PatrolEntryRun::after_transport(request.clone(), &route, control.clone())?;
        let mut run = Self::from_frozen(id, request, resources, prepared, target, control)?;
        run.agent_goal_bundle = Some(goal_bundle);
        run.snapshot.transport = route.snapshot();
        run.snapshot.entry = Some(entry.snapshot());
        run.snapshot.stage = PatrolStage::Entry;
        run.child = Some(Child::Entry(Box::new(entry)));
        run.child_started = false;
        Ok(run)
    }
    pub(crate) fn agent_completion_for_parent(
        &self,
    ) -> Option<(PatrolProof, od_core::agent_goal::AgentGoalStatus)> {
        if self.agent_goal_bundle.is_none()
            || !self.snapshot.inputs_released
            || !matches!(
                self.snapshot.stage,
                PatrolStage::Settlement | PatrolStage::Done
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
            PatrolProof {
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
        request: PatrolRequest,
        root: &Path,
        prepared: PreparedDeployment,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<Self> {
        let resources = PatrolResources::load(root, prepared.resource_root()?)?;
        Self::from_frozen(id, request, resources, prepared, target, control)
    }
    pub(crate) fn from_frozen(
        id: Uuid,
        request: PatrolRequest,
        resources: PatrolResources,
        prepared: PreparedDeployment,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(!id.is_nil(), "区域巡防缺少运行身份");
        request.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            prepared.plan().config.script
                == request
                    .plan
                    .script(&request.teams)
                    .map_err(anyhow::Error::msg)?,
            "区域巡防战斗脚本与冻结计划不一致"
        );
        if matches!(target, BattleTarget::Fixture) {
            ensure!(
                prepared.plan().config.mode == InputMode::Foreground
                    && prepared.plan().config.directml_device.is_none(),
                "隔离区域巡防只复用前台 CPU 控制器"
            );
        }
        if let BattleTarget::Game { window, .. } = &target {
            ensure!(
                window.width == 1920 && window.height == 1080,
                "区域巡防须冻结 1080p 窗口"
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
            snapshot: PatrolSnapshot {
                id,
                request,
                stage: PatrolStage::Transport,
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
    pub fn snapshot(&self) -> PatrolSnapshot {
        self.snapshot.clone()
    }
    pub fn uses_control(&self, c: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &c.cancellation())
    }
    pub fn proof(&self) -> Option<PatrolProof> {
        if self.control.requested().is_some() {
            return None;
        }
        self.proof_for_parent()
    }
    pub(crate) fn proof_for_parent(&self) -> Option<PatrolProof> {
        if self.end != Some(PatrolEnd::Completed)
            || !self.snapshot.inputs_released
            || self.snapshot.stage != PatrolStage::Done
        {
            return None;
        }
        Some(PatrolProof {
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
        (self.end == Some(PatrolEnd::InsufficientCharge) && self.snapshot.inputs_released)
            .then(|| self.snapshot.entry.as_ref()?.flow.charge())
            .flatten()
    }
    pub(crate) fn execution_id(&self) -> Uuid {
        self.execution_id
    }
    fn interrupted(&self) -> Option<PatrolEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => PatrolEnd::Paused,
            Interrupt::Stop => PatrolEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(PatrolEnd::Paused) && self.snapshot.inputs_released,
            "区域巡防尚未安全暂停"
        );
        if !self.child_started {
            ensure!(self.control.resume(), "停止不能继续");
        } else {
            match self.child.as_mut().context("区域巡防子流程缺失")? {
                Child::Transport(c) => c.resume_after_parent_pause()?,
                Child::Entry(c) => c.resume()?,
                Child::Teams(c) => c.resume()?,
                Child::Deployment(c) => c.resume()?,
                Child::Combat(c) => c.resume_segment_after_parent_pause()?,
                Child::Settlement(c) => {
                    c.resume()?;
                    self.snapshot.stage = PatrolStage::Settlement;
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
        let combat = self.snapshot.stage == PatrolStage::Combat;
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
                let n = slot.as_mut().context("区域巡防隔离控制器缺失")?;
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
        let n = slot.as_mut().context("区域巡防控制器交接后缺失")?;
        n.check_combat_input_mode(mode)?;
        if self.control.cancellation().load(Ordering::Acquire) {
            return Ok(None);
        }
        if self.loaded != Some(self.snapshot.stage) {
            self.resources.verify()?;
            match self.snapshot.stage {
                PatrolStage::Transport => n.load_charge_transport_bundle(&self.resources.route)?,
                PatrolStage::Entry => {
                    n.clear_agent_goal_bundle()?;
                    n.load_area_patrol_bundle(&self.resources.patrol)?;
                    n.load_restore_charge_bundle(&self.resources.restore)?;
                }
                PatrolStage::Teams => {
                    n.load_simulation_bundle(&self.resources.simulation)?;
                    n.load_basic_daily_bundle(&self.resources.daily)?;
                }
                PatrolStage::Deployment => {
                    n.load_simulation_deploy_bundle(&self.resources.deploy)?
                }
                PatrolStage::Combat => {
                    self.recognizer = Some(n.load_combat_bundle(&self.resources.combat)?)
                }
                PatrolStage::Settlement => {
                    n.load_area_patrol_bundle(&self.resources.patrol)?;
                    if let Some(path) = &self.agent_goal_bundle {
                        n.load_agent_goal_bundle(path)?;
                    } else {
                        n.clear_agent_goal_bundle()?;
                    }
                }
                PatrolStage::Done => anyhow::bail!("区域巡防完成后不能交接控制器"),
            }
            self.loaded = Some(self.snapshot.stage);
        }
        Ok(Some(n))
    }
    pub fn execute_with_budget(
        &mut self,
        slot: &mut Option<NativeSession>,
        parent_remaining: Duration,
        mut publish: impl FnMut(PatrolSnapshot) -> Result<()>,
    ) -> Result<PatrolEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(PatrolEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "区域巡防不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.request.timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<PatrolEnd> {
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
                    return Ok(PatrolEnd::TimedOut);
                }
                if self.snapshot.stage == PatrolStage::Done {
                    return Ok(PatrolEnd::Completed);
                }
                let Some(n) = self.session(slot)? else {
                    return Ok(self.interrupted().unwrap_or(PatrolEnd::Stopped));
                };
                if let Some(e) = self.interrupted() {
                    return Ok(e);
                }
                if remaining().is_zero() {
                    return Ok(PatrolEnd::TimedOut);
                }
                self.child_started = true;
                let snapshot = &mut self.snapshot;
                let mut save = |s: PatrolSnapshot| publish(s);
                match self.child.as_mut().context("区域巡防当前子流程缺失")? {
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
                            TrainingTransportEnd::Paused => return Ok(PatrolEnd::Paused),
                            TrainingTransportEnd::Stopped => return Ok(PatrolEnd::Stopped),
                            TrainingTransportEnd::TimedOut => return Ok(PatrolEnd::TimedOut),
                            TrainingTransportEnd::Failed => anyhow::bail!("区域巡防传送失败"),
                        }
                        let entry = PatrolEntryRun::after_transport(
                            self.snapshot.request.clone(),
                            c,
                            self.control.clone(),
                        )?;
                        self.snapshot.entry = Some(entry.snapshot());
                        self.child = Some(Child::Entry(Box::new(entry)));
                        self.snapshot.stage = PatrolStage::Entry;
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
                            PatrolEntryEnd::Ready => {
                                ensure!(c.ready_for_parent(), "区域巡防详情未正常保存或释放")
                            }
                            PatrolEntryEnd::Insufficient => {
                                ensure!(c.shortage_for_parent(), "区域巡防缺少实际不足证明");
                                return Ok(PatrolEnd::InsufficientCharge);
                            }
                            PatrolEntryEnd::Paused => return Ok(PatrolEnd::Paused),
                            PatrolEntryEnd::Stopped => return Ok(PatrolEnd::Stopped),
                            PatrolEntryEnd::TimedOut => return Ok(PatrolEnd::TimedOut),
                            PatrolEntryEnd::Failed => anyhow::bail!("区域巡防详情准备失败"),
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
                        self.snapshot.stage = PatrolStage::Teams;
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
                            TeamsEnd::Paused => return Ok(PatrolEnd::Paused),
                            TeamsEnd::Stopped => return Ok(PatrolEnd::Stopped),
                            TeamsEnd::TimedOut => return Ok(PatrolEnd::TimedOut),
                            TeamsEnd::Failed => anyhow::bail!("区域巡防选队失败"),
                        }
                        let entry = self
                            .snapshot
                            .entry
                            .as_ref()
                            .context("区域巡防详情检查点缺失")?
                            .flow
                            .clone();
                        let team = c.snapshot.flow.result().context("区域巡防选队结果缺失")?;
                        let d = DeploymentRun::after_teams(
                            self.snapshot.id,
                            PatrolDeployRequest {
                                entry,
                                team,
                                timeout_ms: self.snapshot.request.timeout_ms,
                            },
                            TeamsEnd::Prepared,
                            &c.snapshot,
                            self.prepared.take().context("区域巡防冻结战斗脚本缺失")?,
                            self.control.clone(),
                        )?;
                        self.snapshot.deployment = Some(d.snapshot.clone());
                        self.child = Some(Child::Deployment(Box::new(d)));
                        self.snapshot.stage = PatrolStage::Deployment;
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
                            DeploymentEnd::Paused => return Ok(PatrolEnd::Paused),
                            DeploymentEnd::Stopped => return Ok(PatrolEnd::Stopped),
                            DeploymentEnd::TimedOut => return Ok(PatrolEnd::TimedOut),
                            DeploymentEnd::Failed => anyhow::bail!("区域巡防出战失败"),
                        }
                        let Some(Child::Deployment(c)) = self.child.take() else {
                            unreachable!()
                        };
                        let c = c.into_combat_for_parent()?;
                        self.snapshot.combat = Some(c.progress.clone());
                        self.child = Some(Child::Combat(Box::new(c)));
                        self.snapshot.stage = PatrolStage::Combat;
                    }
                    Child::Combat(c) => {
                        let result = c.execute_segment_with_budget(
                            n,
                            self.recognizer.as_mut().context("区域巡防识别器缺失")?,
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
                                anyhow::bail!("区域巡防战斗撤退，不能登记成功")
                            }
                            CombatSegmentEnd::Interrupted(Interrupt::Pause) => {
                                return Ok(PatrolEnd::Paused);
                            }
                            CombatSegmentEnd::Interrupted(Interrupt::Stop) => {
                                return Ok(PatrolEnd::Stopped);
                            }
                        }
                        let mut c = PatrolSettlementRun::after_combat(
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
                        self.snapshot.stage = PatrolStage::Settlement;
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
                            PatrolSettlementEnd::Collected => (),
                            PatrolSettlementEnd::Paused => return Ok(PatrolEnd::Paused),
                            PatrolSettlementEnd::Stopped => return Ok(PatrolEnd::Stopped),
                            PatrolSettlementEnd::TimedOut => return Ok(PatrolEnd::TimedOut),
                            PatrolSettlementEnd::Failed => anyhow::bail!("区域巡防结算失败"),
                        }
                        ensure!(
                            c.snapshot.flow.completed() && self.snapshot.inputs_released,
                            "区域巡防尚未实际返回或释放输入"
                        );
                        self.snapshot.stage = PatrolStage::Done;
                    }
                }
                self.child_started = false;
                if self.snapshot.stage == PatrolStage::Done {
                    self.child_started = true;
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                save(self.snapshot.clone())?;
            }
        })();
        self.snapshot.inputs_released = slot.as_mut().is_none_or(|n| n.release_inputs().is_ok());
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        self.end = Some(PatrolEnd::Failed);
        ensure!(
            self.snapshot.inputs_released,
            "区域巡防收尾不能确认释放输入"
        );
        let mut end = result?;
        publish(self.snapshot.clone())?;
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(e) = self.interrupted() {
            end = e;
        } else if remaining().is_zero() {
            end = PatrolEnd::TimedOut;
        }
        if end == PatrolEnd::Paused && !self.control.acknowledge_pause() {
            end = PatrolEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
