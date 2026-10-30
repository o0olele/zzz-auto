//! 深度追猎父挑战，目标类型与模拟室分开保存。
use crate::{
    charge_transport::{TrainingTransportEnd, TrainingTransportRun, TrainingTransportSnapshot},
    combat::{CombatRun, CombatSegmentEnd},
    deep_hunt_entry::{HuntEntryEnd, HuntEntryRun, HuntEntrySnapshot},
    deep_hunt_settlement::{HuntSettlementEnd, HuntSettlementRun, HuntSettlementSnapshot},
    simulation_battle::BattleTarget,
    simulation_deploy::{DeploymentEnd, DeploymentRun, DeploymentSnapshot, PreparedDeployment},
    simulation_teams::{TeamsEnd, TeamsRun, TeamsSnapshot},
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{
    GameSessionConfig, InferenceDevice, NativeSession, SessionHandoff,
    combat_vision::{CombatBundle, CombatRecognizer},
};
pub use od_core::deep_hunt::HuntRequest;
use od_core::{
    InputMode,
    app_combat::AppCombatPlan,
    battle_result::NormalBattleResult,
    charge_plan::ChargePlanItem,
    charge_transport::TrainingTransportRequest,
    combat::CombatProgress,
    deep_hunt::HuntDeployRequest,
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
pub enum HuntStage {
    Transport,
    Entry,
    Teams,
    Deployment,
    Move,
    Combat,
    Settlement,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuntEnd {
    Completed,
    InsufficientCharge,
    WeeklyRewardsAvailable,
    Paused,
    Stopped,
    TimedOut,
    Failed,
    AwaitingVerification,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HuntSnapshot {
    pub id: Uuid,
    pub request: HuntRequest,
    pub stage: HuntStage,
    pub transport: TrainingTransportSnapshot,
    pub entry: Option<HuntEntrySnapshot>,
    pub teams: Option<TeamsSnapshot>,
    pub deployment: Option<DeploymentSnapshot<HuntDeployRequest>>,
    pub combat: Option<CombatProgress>,
    pub movement: Option<od_core::challenge_move::ChallengeMove>,
    pub movement_ms: u64,
    pub settlement: Option<HuntSettlementSnapshot>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntProof {
    pub run_id: Uuid,
    pub execution_id: Uuid,
    pub plan: ChargePlanItem,
    pub charge_after: CardsCharge,
}
#[derive(Clone)]
pub(crate) struct HuntResources {
    root: PathBuf,
    route: PathBuf,
    hunt: PathBuf,
    simulation: PathBuf,
    daily: PathBuf,
    restore: PathBuf,
    deploy: PathBuf,
    combat: PathBuf,
    level: Option<PathBuf>,
    hashes: Vec<(PathBuf, Vec<u8>)>,
}
impl HuntResources {
    pub(crate) fn load(root: &Path, combat: &Path, requires_level: bool) -> Result<Self> {
        od_core::config::reject_links(root)?;
        let root = root.canonicalize()?;
        let route = maa_adapter::charge_transport::validate_charge_transport_bundle(
            &root.join("generated/charge-transport"),
        )?;
        let hunt =
            maa_adapter::deep_hunt::validate_deep_hunt_bundle(&root.join("generated/deep-hunt"))?;
        let simulation = maa_adapter::simulation::validate_simulation_bundle(
            &root.join("generated/simulation"),
        )?;
        let daily = maa_adapter::validate_basic_daily_bundle(&root.join("generated/basic-daily"))?;
        maa_adapter::yolo::validate_challenge_detector(&daily)?;
        let restore = maa_adapter::restore_charge::validate_restore_charge_bundle(
            &root.join("generated/restore-charge"),
        )?;
        let deploy = maa_adapter::simulation_deploy::validate_simulation_deploy_bundle(
            &root.join("generated/simulation-deploy"),
        )?;
        let combat = CombatBundle::load(combat)?.root;
        let level = requires_level
            .then(|| {
                maa_adapter::charge_level::validate_charge_level_bundle(
                    &root.join("generated/charge-level"),
                )
            })
            .transpose()?;
        let mut manifests = vec![
            route.join("charge_transport-manifest.json"),
            hunt.join("deep_hunt-manifest.json"),
            simulation.join("simulation-manifest.json"),
            daily.join("basic-daily-manifest.json"),
            restore.join("restore-charge-manifest.json"),
            deploy.join("simulation_deploy-manifest.json"),
            combat.join("combat-manifest.json"),
        ];
        if let Some(p) = &level {
            manifests.push(p.join("charge_level-manifest.json"));
        }
        let hashes = manifests
            .into_iter()
            .map(|p| Ok((p.clone(), Sha256::digest(std::fs::read(p)?).to_vec())))
            .collect::<Result<_>>()?;
        Ok(Self {
            root,
            route,
            hunt,
            simulation,
            daily,
            restore,
            deploy,
            combat,
            level,
            hashes,
        })
    }
    pub(crate) fn verify(&self) -> Result<()> {
        for (p, hash) in &self.hashes {
            od_core::config::reject_links(p)?;
            ensure!(
                Sha256::digest(std::fs::read(p)?).as_slice() == hash,
                "深度追猎冻结资源已变化"
            );
        }
        maa_adapter::charge_transport::validate_charge_transport_bundle(&self.route)?;
        maa_adapter::deep_hunt::validate_deep_hunt_bundle(&self.hunt)?;
        maa_adapter::simulation::validate_simulation_bundle(&self.simulation)?;
        maa_adapter::validate_basic_daily_bundle(&self.daily)?;
        maa_adapter::yolo::validate_challenge_detector(&self.daily)?;
        maa_adapter::restore_charge::validate_restore_charge_bundle(&self.restore)?;
        maa_adapter::simulation_deploy::validate_simulation_deploy_bundle(&self.deploy)?;
        CombatBundle::load(&self.combat)?;
        if let Some(p) = &self.level {
            maa_adapter::charge_level::validate_charge_level_bundle(p)?;
        }
        Ok(())
    }
}
enum Child {
    Transport(Box<TrainingTransportRun>),
    Entry(Box<HuntEntryRun>),
    Teams(Box<TeamsRun>),
    Deployment(Box<DeploymentRun<HuntDeployRequest>>),
    Combat(Box<CombatRun>),
    Settlement(Box<HuntSettlementRun>),
}
pub struct DeepHuntRun {
    agent_goal_bundle: Option<PathBuf>,
    snapshot: HuntSnapshot,
    execution_id: Uuid,
    resources: HuntResources,
    plan: AppCombatPlan,
    prepared: Option<PreparedDeployment>,
    target: BattleTarget,
    control: PipelineControl,
    child: Option<Child>,
    child_started: bool,
    loaded: Option<HuntStage>,
    recognizer: Option<CombatRecognizer>,
    end: Option<HuntEnd>,
}
impl DeepHuntRun {
    pub(crate) fn after_agent_transport(
        id: Uuid,
        request: HuntRequest,
        resources: HuntResources,
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
        let entry = HuntEntryRun::after_transport(request.clone(), &route, control.clone())?;
        let mut run = Self::from_frozen(id, request, resources, prepared, target, control)?;
        run.agent_goal_bundle = Some(goal_bundle);
        run.snapshot.transport = route.snapshot();
        run.snapshot.entry = Some(entry.snapshot());
        run.snapshot.stage = HuntStage::Entry;
        run.child = Some(Child::Entry(Box::new(entry)));
        run.child_started = false;
        Ok(run)
    }
    pub(crate) fn agent_completion_for_parent(
        &self,
    ) -> Option<(HuntProof, od_core::agent_goal::AgentGoalStatus)> {
        if self.agent_goal_bundle.is_none()
            || !self.snapshot.inputs_released
            || !matches!(self.snapshot.stage, HuntStage::Settlement | HuntStage::Done)
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
            HuntProof {
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
        request: HuntRequest,
        root: &Path,
        prepared: PreparedDeployment,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<Self> {
        let resources = HuntResources::load(
            root,
            prepared.resource_root()?,
            request.plan.level != "默认等级",
        )?;
        Self::from_frozen(id, request, resources, prepared, target, control)
    }
    pub(crate) fn from_frozen(
        id: Uuid,
        request: HuntRequest,
        resources: HuntResources,
        prepared: PreparedDeployment,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(!id.is_nil(), "深度追猎缺少运行身份");
        request.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            prepared.plan().config.script
                == request
                    .plan
                    .script(&request.teams)
                    .map_err(anyhow::Error::msg)?,
            "深度追猎战斗脚本与冻结计划不一致"
        );
        if matches!(target, BattleTarget::Fixture) {
            ensure!(
                prepared.plan().config.mode == InputMode::Foreground
                    && prepared.plan().config.directml_device.is_none(),
                "隔离深度追猎只复用前台 CPU 控制器"
            );
        }
        if let BattleTarget::Game { window, .. } = &target {
            ensure!(
                window.width == 1920 && window.height == 1080,
                "深度追猎须冻结 1080p 窗口"
            );
        }
        ensure!(
            request.plan.level == "默认等级" || resources.level.is_some(),
            "指定等级缺少冻结资源"
        );
        resources.verify()?;
        ensure!(
            resources.combat == prepared.resource_root()?,
            "深度追猎冻结战斗资源不一致"
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
            snapshot: HuntSnapshot {
                id,
                request,
                stage: HuntStage::Transport,
                transport: transport.snapshot(),
                entry: None,
                teams: None,
                deployment: None,
                combat: None,
                movement: None,
                movement_ms: 0,
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
    pub fn snapshot(&self) -> HuntSnapshot {
        self.snapshot.clone()
    }
    pub fn uses_control(&self, c: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &c.cancellation())
    }
    pub fn proof(&self) -> Option<HuntProof> {
        if self.control.requested().is_some() {
            return None;
        }
        self.proof_for_parent()
    }
    pub(crate) fn proof_for_parent(&self) -> Option<HuntProof> {
        if self.end != Some(HuntEnd::Completed)
            || !self.snapshot.inputs_released
            || self.snapshot.stage != HuntStage::Done
        {
            return None;
        }
        Some(HuntProof {
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
        (self.end == Some(HuntEnd::InsufficientCharge) && self.snapshot.inputs_released)
            .then(|| self.snapshot.entry.as_ref()?.flow.charge())
            .flatten()
    }
    pub fn weekly_rewards(&self) -> Option<u8> {
        self.control
            .requested()
            .is_none()
            .then(|| self.weekly_for_parent())
            .flatten()
    }
    pub(crate) fn weekly_for_parent(&self) -> Option<u8> {
        (self.end == Some(HuntEnd::WeeklyRewardsAvailable) && self.snapshot.inputs_released)
            .then(|| self.snapshot.entry.as_ref()?.flow.weekly_blocked())
            .flatten()
    }
    pub(crate) fn execution_id(&self) -> Uuid {
        self.execution_id
    }
    fn interrupted(&self) -> Option<HuntEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => HuntEnd::Paused,
            Interrupt::Stop => HuntEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(HuntEnd::Paused) && self.snapshot.inputs_released,
            "深度追猎尚未安全暂停"
        );
        if !self.child_started {
            ensure!(self.control.resume(), "停止不能继续");
        } else {
            match self.child.as_mut().context("深度追猎子流程缺失")? {
                Child::Transport(c) => c.resume_after_parent_pause()?,
                Child::Entry(c) => c.resume()?,
                Child::Teams(c) => c.resume()?,
                Child::Deployment(c) => c.resume()?,
                Child::Combat(_) if self.snapshot.stage == HuntStage::Move => {
                    ensure!(self.control.resume(), "停止不能继续");
                }
                Child::Combat(c) => c.resume_segment_after_parent_pause()?,
                Child::Settlement(c) => {
                    c.resume()?;
                    self.snapshot.stage = HuntStage::Settlement;
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
        let combat = self.snapshot.stage == HuntStage::Combat;
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
                let n = slot.as_mut().context("深度追猎隔离控制器缺失")?;
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
        let n = slot.as_mut().context("深度追猎控制器交接后缺失")?;
        n.check_combat_input_mode(mode)?;
        if self.control.cancellation().load(Ordering::Acquire) {
            return Ok(None);
        }
        if self.loaded != Some(self.snapshot.stage) {
            self.resources.verify()?;
            match self.snapshot.stage {
                HuntStage::Transport => n.load_charge_transport_bundle(&self.resources.route)?,
                HuntStage::Entry => {
                    n.clear_agent_goal_bundle()?;
                    n.load_deep_hunt_bundle(&self.resources.hunt)?;
                    if self.snapshot.request.plan.level != "默认等级" {
                        n.load_charge_level_bundle(
                            self.resources
                                .level
                                .as_deref()
                                .context("指定等级资源未冻结")?,
                        )?;
                    } else {
                        n.clear_charge_level_bundle()?;
                    }
                    n.load_restore_charge_bundle(&self.resources.restore)?;
                }
                HuntStage::Teams => {
                    n.load_simulation_bundle(&self.resources.simulation)?;
                    n.load_basic_daily_bundle(&self.resources.daily)?;
                }
                HuntStage::Deployment => n.load_simulation_deploy_bundle(&self.resources.deploy)?,
                HuntStage::Move | HuntStage::Combat => {
                    if self.snapshot.stage == HuntStage::Move {
                        n.load_challenge_detector(&self.resources.daily)?;
                    }
                    self.recognizer = Some(n.load_combat_bundle(&self.resources.combat)?)
                }
                HuntStage::Settlement => {
                    n.load_deep_hunt_bundle(&self.resources.hunt)?;
                    if let Some(path) = &self.agent_goal_bundle {
                        n.load_agent_goal_bundle(path)?;
                    } else {
                        n.clear_agent_goal_bundle()?;
                    }
                }
                HuntStage::Done => anyhow::bail!("深度追猎完成后不能交接控制器"),
            }
            self.loaded = Some(self.snapshot.stage);
        }
        Ok(Some(n))
    }
    pub fn execute_with_budget(
        &mut self,
        slot: &mut Option<NativeSession>,
        parent_remaining: Duration,
        mut publish: impl FnMut(HuntSnapshot) -> Result<()>,
    ) -> Result<HuntEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(HuntEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "深度追猎不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.request.timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<HuntEnd> {
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
                    return Ok(HuntEnd::TimedOut);
                }
                if self.snapshot.stage == HuntStage::Done {
                    return Ok(HuntEnd::Completed);
                }
                let Some(n) = self.session(slot)? else {
                    return Ok(self.interrupted().unwrap_or(HuntEnd::Stopped));
                };
                if let Some(e) = self.interrupted() {
                    return Ok(e);
                }
                if remaining().is_zero() {
                    return Ok(HuntEnd::TimedOut);
                }
                self.child_started = true;
                let snapshot = &mut self.snapshot;
                let mut save = |s: HuntSnapshot| publish(s);
                match self.child.as_mut().context("深度追猎当前子流程缺失")? {
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
                            TrainingTransportEnd::Paused => return Ok(HuntEnd::Paused),
                            TrainingTransportEnd::Stopped => return Ok(HuntEnd::Stopped),
                            TrainingTransportEnd::TimedOut => return Ok(HuntEnd::TimedOut),
                            TrainingTransportEnd::Failed => anyhow::bail!("深度追猎传送失败"),
                        }
                        let entry = HuntEntryRun::after_transport(
                            self.snapshot.request.clone(),
                            c,
                            self.control.clone(),
                        )?;
                        self.snapshot.entry = Some(entry.snapshot());
                        self.child = Some(Child::Entry(Box::new(entry)));
                        self.snapshot.stage = HuntStage::Entry;
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
                            HuntEntryEnd::Ready => {
                                ensure!(c.ready_for_parent(), "深度追猎详情未正常保存或释放")
                            }
                            HuntEntryEnd::Insufficient => {
                                ensure!(c.shortage_for_parent(), "深度追猎缺少实际不足证明");
                                return Ok(HuntEnd::InsufficientCharge);
                            }
                            HuntEntryEnd::WeeklyRewardsAvailable => {
                                return Ok(HuntEnd::WeeklyRewardsAvailable);
                            }
                            HuntEntryEnd::Paused => return Ok(HuntEnd::Paused),
                            HuntEntryEnd::Stopped => return Ok(HuntEnd::Stopped),
                            HuntEntryEnd::TimedOut => return Ok(HuntEnd::TimedOut),
                            HuntEntryEnd::Failed => anyhow::bail!("深度追猎详情准备失败"),
                            HuntEntryEnd::AwaitingVerification => {
                                return Ok(HuntEnd::AwaitingVerification);
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
                        self.snapshot.stage = HuntStage::Teams;
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
                            TeamsEnd::Paused => return Ok(HuntEnd::Paused),
                            TeamsEnd::Stopped => return Ok(HuntEnd::Stopped),
                            TeamsEnd::TimedOut => return Ok(HuntEnd::TimedOut),
                            TeamsEnd::Failed => anyhow::bail!("深度追猎选队失败"),
                        }
                        let entry = self
                            .snapshot
                            .entry
                            .as_ref()
                            .context("深度追猎详情检查点缺失")?
                            .flow
                            .clone();
                        let team = c.snapshot.flow.result().context("深度追猎选队结果缺失")?;
                        let d = DeploymentRun::after_teams(
                            self.snapshot.id,
                            HuntDeployRequest {
                                entry,
                                team,
                                timeout_ms: self.snapshot.request.timeout_ms,
                            },
                            TeamsEnd::Prepared,
                            &c.snapshot,
                            self.prepared.take().context("深度追猎冻结战斗脚本缺失")?,
                            self.control.clone(),
                        )?;
                        self.snapshot.deployment = Some(d.snapshot.clone());
                        self.child = Some(Child::Deployment(Box::new(d)));
                        self.snapshot.stage = HuntStage::Deployment;
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
                            DeploymentEnd::Paused => return Ok(HuntEnd::Paused),
                            DeploymentEnd::Stopped => return Ok(HuntEnd::Stopped),
                            DeploymentEnd::TimedOut => return Ok(HuntEnd::TimedOut),
                            DeploymentEnd::Failed => anyhow::bail!("深度追猎出战失败"),
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
                        self.snapshot.stage = HuntStage::Move;
                    }
                    Child::Combat(c) => {
                        if snapshot.stage == HuntStage::Move {
                            let mut movement =
                                snapshot.movement.clone().context("深度追猎战前移动缺失")?;
                            let move_started = Instant::now();
                            let move_initial = snapshot.movement_ms;
                            let move_budget =
                                Duration::from_millis(120_000_u64.saturating_sub(move_initial))
                                    .min(remaining());
                            snapshot.inputs_released = false;
                            let end = n.execute_challenge_move(
                                self.recognizer.as_ref().context("深度追猎移动识别器缺失")?,
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
                            ensure!(snapshot.inputs_released, "深度追猎战前移动输入未释放");
                            match end? {
                                maa_adapter::challenge_move::MoveEnd::Completed => (),
                                maa_adapter::challenge_move::MoveEnd::TimedOut => {
                                    return Ok(HuntEnd::TimedOut);
                                }
                                maa_adapter::challenge_move::MoveEnd::Cancelled => {
                                    return Ok(match self.control.requested() {
                                        Some(Interrupt::Pause) => HuntEnd::Paused,
                                        _ => HuntEnd::Stopped,
                                    });
                                }
                            }
                            snapshot.stage = HuntStage::Combat;
                            save(snapshot.clone())?;
                            continue;
                        }
                        let result = c.execute_segment_with_budget(
                            n,
                            self.recognizer.as_mut().context("深度追猎识别器缺失")?,
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
                                anyhow::bail!("深度追猎战斗撤退，不能登记成功")
                            }
                            CombatSegmentEnd::Interrupted(Interrupt::Pause) => {
                                return Ok(HuntEnd::Paused);
                            }
                            CombatSegmentEnd::Interrupted(Interrupt::Stop) => {
                                return Ok(HuntEnd::Stopped);
                            }
                        }
                        let mut c = HuntSettlementRun::after_combat(
                            result,
                            self.snapshot.inputs_released,
                            self.snapshot.request.timeout_ms.min(300_000),
                            self.snapshot
                                .request
                                .plan
                                .execution_mission_type()
                                .to_owned(),
                            self.control.clone(),
                        )?;
                        if self.agent_goal_bundle.is_some() {
                            c.enable_agent_goal()?;
                        }
                        self.snapshot.settlement = Some(c.snapshot.clone());
                        self.child = Some(Child::Settlement(Box::new(c)));
                        self.snapshot.stage = HuntStage::Settlement;
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
                            HuntSettlementEnd::Collected => (),
                            HuntSettlementEnd::Paused => return Ok(HuntEnd::Paused),
                            HuntSettlementEnd::Stopped => return Ok(HuntEnd::Stopped),
                            HuntSettlementEnd::TimedOut => return Ok(HuntEnd::TimedOut),
                            HuntSettlementEnd::Failed => anyhow::bail!("深度追猎结算失败"),
                        }
                        ensure!(
                            c.snapshot.flow.completed() && self.snapshot.inputs_released,
                            "深度追猎尚未实际返回或释放输入"
                        );
                        self.snapshot.stage = HuntStage::Done;
                    }
                }
                self.child_started = false;
                if self.snapshot.stage == HuntStage::Done {
                    self.child_started = true;
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                save(self.snapshot.clone())?;
            }
        })();
        self.snapshot.inputs_released = slot.as_mut().is_none_or(|n| n.release_inputs().is_ok());
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        self.end = Some(HuntEnd::Failed);
        ensure!(
            self.snapshot.inputs_released,
            "深度追猎收尾不能确认释放输入"
        );
        let mut end = result?;
        publish(self.snapshot.clone())?;
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(e) = self.interrupted() {
            end = e;
        } else if remaining().is_zero() {
            end = HuntEnd::TimedOut;
        }
        if end == HuntEnd::Paused && !self.control.acknowledge_pause() {
            end = HuntEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
