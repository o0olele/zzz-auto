//! 尚未开放的体力父运行器：同一会话中的页面交接、真实资源、逐次登记及循环。
//! 区域巡防接入冻结战斗，模拟室选关仍明确停在待验证。
use crate::{
    agent_challenge::{
        AgentChallengeEnd, AgentChallengeRequest, AgentChallengeRun, AgentChallengeSnapshot,
    },
    area_patrol::{AreaPatrolRun, PatrolEnd, PatrolRequest, PatrolSnapshot, PatrolStage},
    charge_combat::FrozenChargeCombat,
    charge_navigation::{ChargeNavEnd, ChargeNavRun, ChargeNavSnapshot},
    charge_run::ChargeRegistrar,
    compendium::{TransportEnd, TransportRun, TransportSnapshot},
    deep_hunt::{DeepHuntRun, HuntEnd, HuntRequest, HuntSnapshot, HuntStage},
    ether_synthesis_challenge::{
        SynthesisChallengeEnd, SynthesisChallengeRun, SynthesisChallengeSnapshot,
    },
    expert_challenge::{ExpertChallengeRun, ExpertEnd, ExpertRequest, ExpertSnapshot, ExpertStage},
    simulation_battle::BattleTarget,
    simulation_mission::{MissionEnd, MissionRun, MissionSnapshot},
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{GameSessionConfig, InferenceDevice, NativeSession, SessionHandoff};
use od_core::{
    InputMode,
    charge_navigation::ChargeNavTarget,
    charge_plan::ChargePlanItem,
    charge_resources::ChargeResourceObservation,
    charge_run::{ChargeRunSeed, ChargeSchedule, ChargeSelection},
    combat::CombatConfig,
    compendium::{TransportFlow, TransportRequest},
    config::ConfigStore,
    daily::GameDay,
    ether_synthesis::SynthesisRequest,
    pipeline::{Interrupt, PipelineControl},
    simulation_mission::{MissionFlow, MissionRequest},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeWorkerRequest {
    pub seed: ChargeRunSeed,
    pub timeout_ms: u32,
}
impl ChargeWorkerRequest {
    pub fn validate(&self) -> Result<()> {
        self.seed.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            (1000..=86_400_000).contains(&self.timeout_ms),
            "体力父流程时限无效"
        );
        let plans = self
            .seed
            .config
            .plan_list
            .iter()
            .filter(|p| p.plan_times > 0)
            .chain(
                self.seed
                    .config
                    .double_reward
                    .then_some(&self.seed.config.combat_simulation_double_reward_config),
            );
        for p in plans {
            if p.mission_type_name == "代理人方案培养" {
                AgentChallengeRequest {
                    plan: p.clone(),
                    restore_charge: self.seed.config.restore_charge,
                    teams: self.seed.teams.clone(),
                    timeout_ms: 900_000,
                }
                .validate()
                .map_err(anyhow::Error::msg)?;
                continue;
            }
            match p.category_name.as_str() {
                "合成电池" => SynthesisRequest {
                    plan: p.clone(),
                    timeout_ms: 300_000,
                }
                .validate()
                .map_err(anyhow::Error::msg)?,
                "实战模拟室" => MissionRequest::from_plan(p, 300_000)
                    .map_err(anyhow::Error::msg)?
                    .validate()
                    .map_err(anyhow::Error::msg)?,
                "区域巡防" => PatrolRequest {
                    plan: p.clone(),
                    restore_charge: self.seed.config.restore_charge,
                    teams: self.seed.teams.clone(),
                    timeout_ms: 900_000,
                }
                .validate()
                .map_err(anyhow::Error::msg)?,
                "专业挑战室" => ExpertRequest {
                    plan: p.clone(),
                    restore_charge: self.seed.config.restore_charge,
                    teams: self.seed.teams.clone(),
                    timeout_ms: 900_000,
                }
                .validate()
                .map_err(anyhow::Error::msg)?,
                "恶名狩猎" => HuntRequest {
                    plan: p.clone(),
                    restore_charge: self.seed.config.restore_charge,
                    teams: self.seed.teams.clone(),
                    timeout_ms: 900_000,
                }
                .validate()
                .map_err(anyhow::Error::msg)?,
                _ => anyhow::bail!("体力父流程尚缺少 {} 的消费与结算执行器", p.category_name),
            }
        }
        Ok(())
    }
    fn has_plans(&self) -> bool {
        self.seed.config.double_reward
            || self.seed.config.plan_list.iter().any(|p| p.plan_times > 0)
    }
    fn needs_simulation(&self) -> bool {
        self.seed.config.double_reward
            || self
                .seed
                .config
                .plan_list
                .iter()
                .any(|p| p.plan_times > 0 && p.category_name == "实战模拟室")
    }
    fn needs_patrol(&self) -> bool {
        self.seed
            .config
            .plan_list
            .iter()
            .any(|p| p.plan_times > 0 && p.category_name == "区域巡防")
    }
    fn needs_expert(&self) -> bool {
        self.seed
            .config
            .plan_list
            .iter()
            .any(|p| p.plan_times > 0 && p.category_name == "专业挑战室")
    }
    fn needs_hunt(&self) -> bool {
        self.seed
            .config
            .plan_list
            .iter()
            .any(|p| p.plan_times > 0 && p.category_name == "恶名狩猎")
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeWorkerStage {
    Agent,
    World,
    Handbook,
    ReadResources,
    Synthesis,
    Register,
    Transport,
    Mission,
    Patrol,
    Expert,
    Hunt,
    Finished,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeWorkerOutcome {
    AgentTargetsUnavailable,
    NeedsVerification,
    WeeklyRewardsAvailable,
    Completed,
    NoPlans,
    InsufficientCharge,
    UnavailableMaterials,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeWorkerEnd {
    AgentTargetsUnavailable,
    WeeklyRewardsAvailable,
    Completed,
    NoPlans,
    InsufficientCharge,
    UnavailableMaterials,
    AwaitingVerification,
    AwaitingSave,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeWorkerSnapshot {
    pub request: ChargeWorkerRequest,
    pub stage: ChargeWorkerStage,
    pub schedule: Option<ChargeSchedule>,
    pub navigation: Option<ChargeNavSnapshot>,
    pub synthesis: Option<SynthesisChallengeSnapshot>,
    pub transport: Option<TransportSnapshot>,
    pub mission: Option<MissionSnapshot>,
    pub patrol: Option<PatrolSnapshot>,
    pub expert: Option<ExpertSnapshot>,
    pub hunt: Option<HuntSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentChallengeSnapshot>,
    pub combat_settings: Option<CombatConfig>,
    pub combat_revision: Option<String>,
    pub observed: Option<ChargeResourceObservation>,
    pub outcome: Option<ChargeWorkerOutcome>,
    pub issue: Option<String>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub(crate) struct Resources {
    root: PathBuf,
    navigation: PathBuf,
    compendium: PathBuf,
    entry: PathBuf,
    synthesis: PathBuf,
    mission: Option<PathBuf>,
    patrol: Option<PathBuf>,
    expert: Option<PathBuf>,
    hunt: Option<PathBuf>,
    hashes: Vec<(PathBuf, Vec<u8>)>,
}
impl Resources {
    pub(crate) fn for_request(root: &Path, request: &ChargeWorkerRequest) -> Result<Self> {
        Self::load(
            root,
            request.needs_simulation(),
            request.needs_patrol(),
            request.needs_expert(),
            request.needs_hunt(),
        )
    }
    fn load(root: &Path, simulation: bool, patrol: bool, expert: bool, hunt: bool) -> Result<Self> {
        od_core::config::reject_links(root)?;
        let root = root.canonicalize()?;
        let navigation = maa_adapter::charge_navigation::validate_charge_navigation_bundle(
            &root.join("generated/charge-navigation"),
        )?;
        let compendium = maa_adapter::compendium::validate_compendium_bundle(
            &root.join("generated/compendium"),
        )?;
        let entry = maa_adapter::ether_synthesis_entry::validate_ether_synthesis_entry_bundle(
            &root.join("generated/ether-synthesis-entry"),
        )?;
        let synthesis = maa_adapter::ether_synthesis::validate_ether_synthesis_bundle(
            &root.join("generated/ether-synthesis"),
        )?;
        let mission = simulation
            .then(|| {
                maa_adapter::simulation_mission::validate_simulation_mission_bundle(
                    &root.join("generated/simulation-mission"),
                )
            })
            .transpose()?;
        let mut manifests = vec![
            navigation.join("charge_navigation-manifest.json"),
            compendium.join("compendium-manifest.json"),
            entry.join("ether_synthesis_entry-manifest.json"),
            synthesis.join("ether_synthesis-manifest.json"),
        ];
        let patrol = patrol
            .then(|| {
                maa_adapter::area_patrol::validate_area_patrol_bundle(
                    &root.join("generated/area-patrol"),
                )
            })
            .transpose()?;
        if let Some(p) = &patrol {
            manifests.push(p.join("area_patrol-manifest.json"));
        }
        let expert = expert
            .then(|| {
                maa_adapter::expert_challenge::validate_expert_challenge_bundle(
                    &root.join("generated/expert-challenge"),
                )
            })
            .transpose()?;
        if let Some(p) = &expert {
            manifests.push(p.join("expert_challenge-manifest.json"));
        }
        let hunt = hunt
            .then(|| {
                maa_adapter::deep_hunt::validate_deep_hunt_bundle(&root.join("generated/deep-hunt"))
            })
            .transpose()?;
        if let Some(p) = &hunt {
            manifests.push(p.join("deep_hunt-manifest.json"));
        }
        if let Some(m) = &mission {
            manifests.push(m.join("simulation_mission-manifest.json"));
        }
        let hashes = manifests
            .into_iter()
            .map(|p| Ok((p.clone(), Sha256::digest(std::fs::read(p)?).to_vec())))
            .collect::<Result<_>>()?;
        Ok(Self {
            root,
            navigation,
            compendium,
            entry,
            synthesis,
            mission,
            patrol,
            expert,
            hunt,
            hashes,
        })
    }
    pub(crate) fn verify(&self) -> Result<()> {
        for (path, hash) in &self.hashes {
            od_core::config::reject_links(path)?;
            ensure!(
                Sha256::digest(std::fs::read(path)?).as_slice() == hash,
                "体力父流程冻结资源已变化"
            );
        }
        maa_adapter::charge_navigation::validate_charge_navigation_bundle(&self.navigation)?;
        maa_adapter::compendium::validate_compendium_bundle(&self.compendium)?;
        maa_adapter::ether_synthesis_entry::validate_ether_synthesis_entry_bundle(&self.entry)?;
        maa_adapter::ether_synthesis::validate_ether_synthesis_bundle(&self.synthesis)?;
        if let Some(m) = &self.mission {
            maa_adapter::simulation_mission::validate_simulation_mission_bundle(m)?;
        }
        if let Some(p) = &self.patrol {
            maa_adapter::area_patrol::validate_area_patrol_bundle(p)?;
        }
        if let Some(p) = &self.expert {
            maa_adapter::expert_challenge::validate_expert_challenge_bundle(p)?;
        }
        if let Some(p) = &self.hunt {
            maa_adapter::deep_hunt::validate_deep_hunt_bundle(p)?;
        }
        Ok(())
    }
    fn load_into(&self, n: &mut NativeSession) -> Result<()> {
        self.verify()?;
        n.load_charge_navigation_bundle(&self.navigation)?;
        n.load_compendium_bundle(&self.compendium)?;
        n.load_ether_synthesis_entry_bundle(&self.entry)?;
        n.load_ether_synthesis_bundle(&self.synthesis)?;
        if let Some(m) = &self.mission {
            n.load_simulation_mission_bundle(m)?;
        }
        if let Some(p) = &self.patrol {
            n.load_area_patrol_bundle(p)?;
        }
        if let Some(p) = &self.expert {
            n.load_expert_challenge_bundle(p)?;
        }
        if let Some(p) = &self.hunt {
            n.load_deep_hunt_bundle(p)?;
        }
        Ok(())
    }
}
enum Child {
    Agent(Box<AgentChallengeRun>),
    Nav(ChargeNavRun),
    Synthesis(Box<SynthesisChallengeRun>),
    Transport(TransportRun),
    Mission(MissionRun),
    Patrol(Box<AreaPatrolRun>),
    Expert(Box<ExpertChallengeRun>),
    Hunt(Box<DeepHuntRun>),
}
/// 独占原生会话，外层不能在暂停期间替换窗口、截图方式或控制器。
/// 检查点用于审计，不能反序列化后恢复游戏输入。
pub struct ChargeWorker {
    snapshot: ChargeWorkerSnapshot,
    session: Option<NativeSession>,
    target: BattleTarget,
    combat: Option<FrozenChargeCombat>,
    game: bool,
    resources: Resources,
    loaded: bool,
    registrar: Option<ChargeRegistrar>,
    child: Child,
    child_started: bool,
    control: PipelineControl,
    end: Option<ChargeWorkerEnd>,
    steps: u32,
}
fn guard(
    snapshot: &ChargeWorkerSnapshot,
    registrar: Option<&ChargeRegistrar>,
    store: &ConfigStore,
) -> Result<()> {
    let seed = &snapshot.request.seed;
    ensure!(
        GameDay::at(seed.context.game_day.region, chrono::Utc::now())
            .map_err(anyhow::Error::msg)?
            == seed.context.game_day,
        "体力运行已跨游戏日，不能继续输入"
    );
    let current = store.read()?;
    if let Some(combat) = &snapshot.combat_settings {
        ensure!(
            current
                .instances
                .iter()
                .find(|a| a.id == seed.context.instance_id)
                .is_some_and(|a| &a.combat == combat),
            "体力冻结战斗设置已变化"
        );
    }
    if let Some(r) = registrar {
        return r.check_store(store);
    }
    let observed = ChargeRunSeed::from_config(
        &current,
        seed.context.instance_id,
        &seed.context.group_id,
        seed.context.run_id,
        chrono::Utc::now(),
    )
    .map_err(anyhow::Error::msg)?;
    ensure!(&observed == seed, "体力账号、应用组、修订或冻结配置已变化");
    Ok(())
}
impl ChargeWorker {
    pub fn new(
        request: ChargeWorkerRequest,
        root: &Path,
        session: NativeSession,
        game: bool,
        control: PipelineControl,
    ) -> Result<Self> {
        request.validate()?;
        ensure!(
            !request.needs_patrol() && !request.needs_expert() && !request.needs_hunt(),
            "训练挑战须先准备冻结战斗执行器"
        );
        Self::build(request, root, session, game, control, None)
    }
    /// 与账号配置来自同一目录；全部脚本编译、资源及目标预检先于首个输入。
    pub fn new_with_combat(
        request: ChargeWorkerRequest,
        root: &Path,
        session: NativeSession,
        game: bool,
        control: PipelineControl,
        store: &ConfigStore,
        combat_root: &Path,
    ) -> Result<Self> {
        request.validate()?;
        let target = Self::freeze_target(&session, game)?;
        let combat = FrozenChargeCombat::load(&request, root, combat_root, store, &target)?;
        Self::build(request, root, session, game, control, Some(combat))
    }
    fn freeze_target(session: &NativeSession, game: bool) -> Result<BattleTarget> {
        session.check_ether_synthesis_target(game)?;
        Ok(match (game, session.game_capture_target()?) {
            (true, Some((window, capture))) => BattleTarget::Game { window, capture },
            (false, None) => BattleTarget::Fixture,
            _ => anyhow::bail!("体力会话目标与运行方式不符"),
        })
    }
    fn build(
        request: ChargeWorkerRequest,
        root: &Path,
        session: NativeSession,
        game: bool,
        control: PipelineControl,
        combat: Option<FrozenChargeCombat>,
    ) -> Result<Self> {
        let resources = Resources::for_request(root, &request)?;
        Self::from_frozen(request, resources, session, game, control, combat)
    }
    pub(crate) fn from_frozen(
        request: ChargeWorkerRequest,
        resources: Resources,
        session: NativeSession,
        game: bool,
        control: PipelineControl,
        combat: Option<FrozenChargeCombat>,
    ) -> Result<Self> {
        request.validate()?;
        let target = Self::freeze_target(&session, game)?;
        resources.verify()?;
        if let Some(c) = &combat {
            c.verify()?;
        }
        let nav = ChargeNavRun::new(ChargeNavTarget::World, 30_000, control.clone())?;
        Ok(Self {
            snapshot: ChargeWorkerSnapshot {
                request,
                stage: ChargeWorkerStage::World,
                schedule: None,
                navigation: Some(nav.snapshot()),
                synthesis: None,
                transport: None,
                mission: None,
                patrol: None,
                expert: None,
                hunt: None,
                agent: None,
                combat_settings: combat.as_ref().map(|c| c.settings.clone()),
                combat_revision: combat.as_ref().map(|c| c.revision.clone()),
                observed: None,
                outcome: None,
                issue: None,
                active_ms: 0,
                inputs_released: true,
            },
            session: Some(session),
            target,
            combat,
            game,
            resources,
            loaded: false,
            registrar: None,
            child: Child::Nav(nav),
            child_started: false,
            control,
            end: None,
            steps: 0,
        })
    }
    pub fn snapshot(&self) -> ChargeWorkerSnapshot {
        self.snapshot.clone()
    }
    pub(crate) fn take_released_session(&mut self) -> Result<NativeSession> {
        ensure!(
            self.end.is_some() && self.snapshot.inputs_released,
            "体力尚未结束并释放输入"
        );
        let mut session = self.session.take().context("体力会话已交回")?;
        session.quiesce()?;
        Ok(session)
    }
    pub(crate) fn history_control(&self) -> PipelineControl {
        self.control.clone()
    }
    pub(crate) fn history_end(&self) -> Option<ChargeWorkerEnd> {
        self.end
    }
    pub(crate) fn include_history_time(&mut self, elapsed: Duration) {
        self.snapshot.active_ms = self
            .snapshot
            .active_ms
            .saturating_add(elapsed.as_millis() as u64);
        if self.snapshot.active_ms >= u64::from(self.snapshot.request.timeout_ms)
            && self.end.is_some_and(|e| {
                !matches!(
                    e,
                    ChargeWorkerEnd::Stopped
                        | ChargeWorkerEnd::Failed
                        | ChargeWorkerEnd::Paused
                        | ChargeWorkerEnd::AwaitingSave
                )
            })
        {
            self.end = Some(ChargeWorkerEnd::TimedOut);
        }
    }
    pub(crate) fn refresh_history_control(&mut self) {
        if self.snapshot.inputs_released
            && self.end.is_some_and(|e| {
                !matches!(e, ChargeWorkerEnd::Failed | ChargeWorkerEnd::AwaitingSave)
            })
            && let Some(end) = self.interrupted()
            && (end == ChargeWorkerEnd::Stopped
                || self.snapshot.stage == ChargeWorkerStage::Finished)
        {
            self.end = Some(
                if end == ChargeWorkerEnd::Paused && !self.control.acknowledge_pause() {
                    ChargeWorkerEnd::Stopped
                } else {
                    end
                },
            );
        }
    }
    pub(crate) fn history_progress(
        &self,
        store: &ConfigStore,
    ) -> Result<od_core::charge_application::ChargeAppProgress> {
        use od_core::{
            RunState,
            charge_application::{ChargeAppOutcome as O, ChargeAppSummary},
        };
        let mut progress = crate::charge_application::progress_from_snapshot(&self.snapshot)?;
        if let Some(r) = &self.registrar {
            progress.summary = Some(
                ChargeAppSummary::from_schedule(&r.history_schedule()?)
                    .map_err(anyhow::Error::msg)?,
            );
            progress.counts_committed = !r.has_pending_commit();
            progress.config_revision_after = progress
                .counts_committed
                .then(|| r.current_context().config_revision);
        }
        progress.world_returned = self.snapshot.stage == ChargeWorkerStage::Finished
            && self
                .snapshot
                .navigation
                .as_ref()
                .is_some_and(|n| n.flow.target() == ChargeNavTarget::World && n.flow.ready())
            && self.snapshot.inputs_released;
        progress.outcome = self.end.and_then(|end| match end {
            ChargeWorkerEnd::Completed => Some(O::Completed),
            ChargeWorkerEnd::NoPlans => Some(O::NoPlans),
            ChargeWorkerEnd::InsufficientCharge => Some(O::InsufficientCharge),
            ChargeWorkerEnd::UnavailableMaterials => Some(O::UnavailableMaterials),
            ChargeWorkerEnd::WeeklyRewardsAvailable => Some(O::WeeklyRewardsAvailable),
            ChargeWorkerEnd::AgentTargetsUnavailable => Some(O::AgentTargetsUnavailable),
            ChargeWorkerEnd::AwaitingVerification => Some(O::NeedsVerification),
            ChargeWorkerEnd::AwaitingSave => Some(O::AwaitingSave),
            ChargeWorkerEnd::Stopped => Some(O::Stopped),
            ChargeWorkerEnd::TimedOut => Some(O::TimedOut),
            ChargeWorkerEnd::Failed => Some(O::Failed),
            ChargeWorkerEnd::Paused => None,
        });
        progress.state = match self.end {
            Some(ChargeWorkerEnd::Completed | ChargeWorkerEnd::NoPlans) => RunState::Succeeded,
            Some(ChargeWorkerEnd::Paused | ChargeWorkerEnd::AwaitingSave) => RunState::Paused,
            Some(ChargeWorkerEnd::Stopped) => RunState::Stopped,
            Some(_) => RunState::Failed,
            None => RunState::Running,
        };
        if self.end == Some(ChargeWorkerEnd::Completed) {
            ensure!(
                self.control.requested().is_none(),
                "体力完成记录仍有控制请求"
            );
            let r = self.registrar.as_ref().context("体力完成记录缺少登记器")?;
            ensure!(!r.has_pending_commit(), "体力次数仍待提交");
            r.check_store(store)?;
            let journal = store
                .charge_checkpoint(progress.context.run_id)?
                .context("体力完成记录缺少本次日志")?;
            progress
                .verify_schedule(&journal)
                .map_err(anyhow::Error::msg)?;
        }
        progress.validate().map_err(anyhow::Error::msg)?;
        Ok(progress)
    }
    pub fn outcome(&self) -> Option<ChargeWorkerOutcome> {
        (matches!(
            self.end,
            Some(
                ChargeWorkerEnd::Completed
                    | ChargeWorkerEnd::NoPlans
                    | ChargeWorkerEnd::InsufficientCharge
                    | ChargeWorkerEnd::UnavailableMaterials
                    | ChargeWorkerEnd::WeeklyRewardsAvailable
            )
        ) && self.snapshot.stage == ChargeWorkerStage::Finished
            && self.snapshot.inputs_released
            && self.control.requested().is_none())
        .then_some(self.snapshot.outcome)
        .flatten()
    }
    fn interrupted(&self) -> Option<ChargeWorkerEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => ChargeWorkerEnd::Paused,
            Interrupt::Stop => ChargeWorkerEnd::Stopped,
        })
    }
    fn prepare_interface(&mut self) -> Result<bool> {
        match &self.target {
            BattleTarget::Fixture => {
                let n = self.session.as_mut().context("体力隔离控制器缺失")?;
                n.quiesce()?;
                n.check_ether_synthesis_target(false)?;
            }
            BattleTarget::Game { window, capture } => match NativeSession::handoff_game(
                &mut self.session,
                GameSessionConfig {
                    window,
                    resources: &self.resources.root,
                    capture: *capture,
                    input: InputMode::Foreground,
                    device: InferenceDevice::Cpu,
                },
                &self.control.cancellation(),
            )? {
                SessionHandoff::Cancelled => return Ok(false),
                SessionHandoff::Created => self.loaded = false,
                SessionHandoff::Reused => (),
            },
        }
        if self.control.requested().is_some() {
            return Ok(false);
        }
        if !self.loaded {
            self.resources
                .load_into(self.session.as_mut().context("体力界面会话缺失")?)?;
            self.loaded = true;
        }
        Ok(true)
    }
    fn navigate(&mut self, world: bool) -> Result<()> {
        let target = if world {
            ChargeNavTarget::World
        } else if self.snapshot.request.seed.config.double_reward && self.registrar.is_none() {
            ChargeNavTarget::SimulationCategory
        } else {
            ChargeNavTarget::Handbook
        };
        let nav = ChargeNavRun::new(target, 30_000, self.control.clone())?;
        self.snapshot.stage = if world {
            ChargeWorkerStage::World
        } else {
            ChargeWorkerStage::Handbook
        };
        self.snapshot.navigation = Some(nav.snapshot());
        self.child = Child::Nav(nav);
        self.child_started = false;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(ChargeWorkerEnd::Paused) && self.snapshot.inputs_released,
            "体力父流程尚未安全暂停"
        );
        if self.snapshot.stage == ChargeWorkerStage::Finished {
            self.navigate(true)?;
            ensure!(self.control.resume(), "停止不能继续");
        } else if self.child_started
            && matches!(
                self.snapshot.stage,
                ChargeWorkerStage::World
                    | ChargeWorkerStage::Handbook
                    | ChargeWorkerStage::Synthesis
                    | ChargeWorkerStage::Transport
                    | ChargeWorkerStage::Mission
                    | ChargeWorkerStage::Patrol
                    | ChargeWorkerStage::Expert
                    | ChargeWorkerStage::Hunt
                    | ChargeWorkerStage::Agent
            )
        {
            match &mut self.child {
                Child::Nav(c) => c.resume_after_parent_pause()?,
                Child::Synthesis(c) => c.resume()?,
                Child::Transport(c) => c.resume_after_parent_pause()?,
                Child::Mission(c) => c.resume_after_parent_pause()?,
                Child::Patrol(c) => c.resume()?,
                Child::Expert(c) => c.resume()?,
                Child::Hunt(c) => c.resume()?,
                Child::Agent(c) => c.resume()?,
            }
        } else {
            ensure!(self.control.resume(), "停止不能继续");
        }
        self.end = None;
        Ok(())
    }
    /// 只补交已取得的结果。即使已暂停或停止，也不会发送输入或自动执行下一轮。
    pub fn retry_saved_result(
        &mut self,
        store: &ConfigStore,
        mut publish: impl FnMut(ChargeWorkerSnapshot) -> Result<()>,
    ) -> Result<()> {
        ensure!(
            self.end == Some(ChargeWorkerEnd::AwaitingSave) && self.snapshot.inputs_released,
            "没有待补交的体力结果"
        );
        let r = self.registrar.as_mut().context("体力登记器缺失")?;
        if r.has_pending_commit() {
            r.retry_pending_persisted(store)?;
        }
        self.snapshot.schedule = Some(r.snapshot().clone());
        self.snapshot.issue = None;
        if self.snapshot.stage == ChargeWorkerStage::Register {
            self.navigate(true)?;
        }
        publish(self.snapshot.clone())?;
        self.end = self.interrupted();
        if self.end == Some(ChargeWorkerEnd::Paused) {
            self.control.acknowledge_pause();
        }
        Ok(())
    }
    fn plan(&self) -> Result<&ChargePlanItem> {
        Ok(&self
            .registrar
            .as_ref()
            .context("体力登记器缺失")?
            .snapshot()
            .pending()
            .context("体力尝试缺失")?
            .plan)
    }
    pub fn execute(
        &mut self,
        store: &ConfigStore,
        mut publish: impl FnMut(ChargeWorkerSnapshot) -> Result<()>,
    ) -> Result<ChargeWorkerEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(ChargeWorkerEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "体力父流程不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.request.timeout_ms).saturating_sub(initial),
        );
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<ChargeWorkerEnd> {
            if let Some(n) = self.session.as_mut() {
                n.quiesce()?;
            }
            if self.snapshot.stage != ChargeWorkerStage::Register {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                guard(&self.snapshot, self.registrar.as_ref(), store)?;
                self.resources.verify()?;
                if let Some(c) = &self.combat {
                    c.verify()?;
                }
            }
            loop {
                if self.snapshot.stage != ChargeWorkerStage::Register {
                    if let Some(end) = self.interrupted() {
                        return Ok(end);
                    }
                    if remaining().is_zero() {
                        return Ok(ChargeWorkerEnd::TimedOut);
                    }
                }
                ensure!(self.steps < 40_000, "体力父流程步骤达到上限");
                self.steps += 1;
                if self.snapshot.stage != ChargeWorkerStage::Register {
                    guard(&self.snapshot, self.registrar.as_ref(), store)?;
                    if !matches!(
                        self.snapshot.stage,
                        ChargeWorkerStage::Patrol
                            | ChargeWorkerStage::Agent
                            | ChargeWorkerStage::Expert
                            | ChargeWorkerStage::Hunt
                            | ChargeWorkerStage::Finished
                    ) && !self.prepare_interface()?
                    {
                        return Ok(self.interrupted().unwrap_or(ChargeWorkerEnd::Stopped));
                    }
                }
                match self.snapshot.stage {
                    ChargeWorkerStage::World | ChargeWorkerStage::Handbook => {
                        let Child::Nav(child) = &mut self.child else {
                            anyhow::bail!("体力导航子流程不匹配");
                        };
                        self.child_started = true;
                        let snapshot = &mut self.snapshot;
                        let registrar = self.registrar.as_ref();
                        let end = child.execute_with_budget(
                            self.session.as_mut().context("体力原生会话缺失")?,
                            self.game,
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.navigation = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                guard(snapshot, registrar, store)?;
                                publish(snapshot.clone())?;
                                guard(snapshot, registrar, store)
                            },
                        )?;
                        match end {
                            ChargeNavEnd::Ready => (),
                            ChargeNavEnd::Paused => return Ok(ChargeWorkerEnd::Paused),
                            ChargeNavEnd::Stopped => return Ok(ChargeWorkerEnd::Stopped),
                            ChargeNavEnd::TimedOut => return Ok(ChargeWorkerEnd::TimedOut),
                            ChargeNavEnd::Failed => anyhow::bail!("体力导航失败"),
                        }
                        ensure!(
                            child.snapshot().flow.ready() && child.snapshot().inputs_released,
                            "体力导航没有保存并释放输入的到达回执"
                        );
                        if self.snapshot.stage == ChargeWorkerStage::World {
                            if self.snapshot.outcome.is_some() {
                                self.snapshot.stage = ChargeWorkerStage::Finished;
                            } else if !self.snapshot.request.has_plans() {
                                self.snapshot.outcome = Some(ChargeWorkerOutcome::NoPlans);
                                self.snapshot.stage = ChargeWorkerStage::Finished;
                            } else {
                                self.navigate(false)?;
                            }
                        } else {
                            self.snapshot.stage = ChargeWorkerStage::ReadResources;
                            self.child_started = false;
                        }
                    }
                    ChargeWorkerStage::ReadResources => {
                        let observed = self
                            .session
                            .as_mut()
                            .context("体力原生会话缺失")?
                            .read_charge_resources(&self.control.cancellation(), remaining());
                        self.snapshot.inputs_released = self
                            .session
                            .as_mut()
                            .context("体力原生会话缺失")?
                            .release_inputs()
                            .is_ok();
                        ensure!(self.snapshot.inputs_released, "体力资源读取后输入未释放");
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(ChargeWorkerEnd::TimedOut);
                        }
                        let Some(observed) = observed? else {
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        };
                        self.snapshot.observed = Some(observed.clone());
                        if !observed.handbook || observed.resources.is_none() {
                            self.snapshot.issue = Some(
                                observed
                                    .issue
                                    .unwrap_or_else(|| "体力资源未取得完整同帧读数".into()),
                            );
                            return Ok(ChargeWorkerEnd::AwaitingVerification);
                        }
                        if self.registrar.is_none() {
                            if self.snapshot.request.seed.config.double_reward
                                && observed.double_left.is_none()
                            {
                                self.snapshot.issue = Some("实际双倍余量未知，不能当作零次".into());
                                return Ok(ChargeWorkerEnd::AwaitingVerification);
                            }
                            let request = self
                                .snapshot
                                .request
                                .seed
                                .clone()
                                .bind(observed.double_left)
                                .map_err(anyhow::Error::msg)?;
                            self.registrar = Some(ChargeRegistrar::new(
                                ChargeSchedule::new(request).map_err(anyhow::Error::msg)?,
                                self.control.clone(),
                            )?);
                        }
                        guard(&self.snapshot, self.registrar.as_ref(), store)?;
                        let r = self.registrar.as_mut().context("体力登记器缺失")?;
                        let selection = r.begin_persisted(
                            &r.current_context(),
                            observed.resources.context("体力资源缺失")?,
                            store,
                        );
                        self.snapshot.schedule = Some(r.snapshot().clone());
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        match selection? {
                            ChargeSelection::Ready(attempt) => {
                                if attempt.plan.mission_type_name == "代理人方案培养" {
                                    let c = self
                                        .combat
                                        .as_ref()
                                        .context("代理人缺少冻结战斗能力")?
                                        .create_agent(
                                            self.snapshot.request.seed.context.run_id,
                                            AgentChallengeRequest {
                                                plan: attempt.plan,
                                                teams: self.snapshot.request.seed.teams.clone(),
                                                restore_charge: self
                                                    .snapshot
                                                    .request
                                                    .seed
                                                    .config
                                                    .restore_charge,
                                                timeout_ms: 900_000,
                                            },
                                            self.target.clone(),
                                            self.control.clone(),
                                        )?;
                                    self.snapshot.agent = Some(c.snapshot());
                                    self.child = Child::Agent(Box::new(c));
                                    self.snapshot.stage = ChargeWorkerStage::Agent;
                                } else if attempt.plan.category_name == "合成电池" {
                                    let c = SynthesisChallengeRun::new(
                                        self.snapshot.request.seed.context.run_id,
                                        SynthesisRequest {
                                            plan: attempt.plan,
                                            timeout_ms: 300_000,
                                        },
                                        &self.resources.root,
                                        self.control.clone(),
                                    )?;
                                    self.snapshot.synthesis = Some(c.snapshot());
                                    self.child = Child::Synthesis(Box::new(c));
                                    self.snapshot.stage = ChargeWorkerStage::Synthesis;
                                } else if attempt.plan.category_name == "区域巡防" {
                                    let c = self
                                        .combat
                                        .as_ref()
                                        .context("区域巡防缺少冻结战斗能力")?
                                        .create(
                                            self.snapshot.request.seed.context.run_id,
                                            PatrolRequest {
                                                plan: attempt.plan,
                                                teams: self.snapshot.request.seed.teams.clone(),
                                                restore_charge: self
                                                    .snapshot
                                                    .request
                                                    .seed
                                                    .config
                                                    .restore_charge,
                                                timeout_ms: 900_000,
                                            },
                                            self.target.clone(),
                                            self.control.clone(),
                                        )?;
                                    self.snapshot.patrol = Some(c.snapshot());
                                    self.child = Child::Patrol(Box::new(c));
                                    self.snapshot.stage = ChargeWorkerStage::Patrol;
                                } else if attempt.plan.category_name == "专业挑战室" {
                                    let c = self
                                        .combat
                                        .as_ref()
                                        .context("专业挑战室缺少冻结战斗能力")?
                                        .create_expert(
                                            self.snapshot.request.seed.context.run_id,
                                            ExpertRequest {
                                                plan: attempt.plan,
                                                teams: self.snapshot.request.seed.teams.clone(),
                                                restore_charge: self
                                                    .snapshot
                                                    .request
                                                    .seed
                                                    .config
                                                    .restore_charge,
                                                timeout_ms: 900_000,
                                            },
                                            self.target.clone(),
                                            self.control.clone(),
                                        )?;
                                    self.snapshot.expert = Some(c.snapshot());
                                    self.child = Child::Expert(Box::new(c));
                                    self.snapshot.stage = ChargeWorkerStage::Expert;
                                } else if attempt.plan.category_name == "恶名狩猎" {
                                    let c = self
                                        .combat
                                        .as_ref()
                                        .context("深度追猎缺少冻结战斗能力")?
                                        .create_hunt(
                                            self.snapshot.request.seed.context.run_id,
                                            HuntRequest {
                                                plan: attempt.plan,
                                                teams: self.snapshot.request.seed.teams.clone(),
                                                restore_charge: self
                                                    .snapshot
                                                    .request
                                                    .seed
                                                    .config
                                                    .restore_charge,
                                                timeout_ms: 900_000,
                                            },
                                            self.target.clone(),
                                            self.control.clone(),
                                        )?;
                                    self.snapshot.hunt = Some(c.snapshot());
                                    self.child = Child::Hunt(Box::new(c));
                                    self.snapshot.stage = ChargeWorkerStage::Hunt;
                                } else {
                                    let mission = MissionRequest::from_plan(&attempt.plan, 300_000)
                                        .map_err(anyhow::Error::msg)?;
                                    let c = TransportRun::new(
                                        TransportFlow::new(TransportRequest {
                                            mission_type: mission.mission_type,
                                            timeout_ms: 300_000,
                                        })
                                        .map_err(anyhow::Error::msg)?,
                                        self.control.clone(),
                                    )?;
                                    self.snapshot.transport = Some(c.snapshot.clone());
                                    self.child = Child::Transport(c);
                                    self.snapshot.stage = ChargeWorkerStage::Transport;
                                }
                                self.child_started = false;
                            }
                            ChargeSelection::Completed => {
                                self.snapshot.outcome = Some(ChargeWorkerOutcome::Completed);
                                self.navigate(true)?;
                            }
                            ChargeSelection::InsufficientCharge => {
                                self.snapshot.outcome =
                                    Some(ChargeWorkerOutcome::InsufficientCharge);
                                self.navigate(true)?;
                            }
                            ChargeSelection::UnavailableMaterials => {
                                self.snapshot.outcome =
                                    Some(ChargeWorkerOutcome::UnavailableMaterials);
                                self.navigate(true)?;
                            }
                            ChargeSelection::WeeklyRewardsAvailable => {
                                self.snapshot.outcome =
                                    Some(ChargeWorkerOutcome::WeeklyRewardsAvailable);
                                self.navigate(true)?;
                            }
                            ChargeSelection::AgentTargetsUnavailable => {
                                self.snapshot.outcome =
                                    Some(ChargeWorkerOutcome::AgentTargetsUnavailable);
                                self.navigate(true)?;
                            }
                            ChargeSelection::NeedsVerification => {
                                self.snapshot.outcome =
                                    Some(ChargeWorkerOutcome::NeedsVerification);
                                self.snapshot.issue =
                                    Some("正常完成已登记，代理人达成文字仍需确认".into());
                                self.navigate(true)?;
                            }
                        }
                    }
                    ChargeWorkerStage::Synthesis => {
                        let Child::Synthesis(child) = &mut self.child else {
                            anyhow::bail!("体力合成子流程不匹配");
                        };
                        self.child_started = true;
                        let snapshot = &mut self.snapshot;
                        let registrar = self.registrar.as_ref();
                        let end = child.execute_with_budget(
                            self.session.as_mut().context("体力原生会话缺失")?,
                            self.game,
                            remaining(),
                            |s| {
                                let has_result = s
                                    .exchange
                                    .as_ref()
                                    .is_some_and(|e| e.flow.result().is_some());
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.synthesis = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                if !has_result {
                                    guard(snapshot, registrar, store)?;
                                }
                                publish(snapshot.clone())?;
                                if !has_result {
                                    guard(snapshot, registrar, store)?;
                                }
                                Ok(())
                            },
                        )?;
                        match end {
                            SynthesisChallengeEnd::Completed => {
                                self.snapshot.stage = ChargeWorkerStage::Register
                            }
                            SynthesisChallengeEnd::Paused => return Ok(ChargeWorkerEnd::Paused),
                            SynthesisChallengeEnd::Stopped => return Ok(ChargeWorkerEnd::Stopped),
                            SynthesisChallengeEnd::TimedOut => {
                                return Ok(ChargeWorkerEnd::TimedOut);
                            }
                            SynthesisChallengeEnd::Failed => anyhow::bail!("体力合成失败"),
                        }
                        // 已取得的结果先交给日志登记；不能因账号后来变化而丢掉实际消费。
                        continue;
                    }
                    ChargeWorkerStage::Register => {
                        let r = self.registrar.as_mut().context("体力登记器缺失")?;
                        let saved = match &self.child {
                            Child::Agent(c) => r.record_completed_agent_persisted(c, store),
                            Child::Synthesis(c) => r.record_completed_synthesis_persisted(c, store),
                            Child::Patrol(c) => r.record_completed_patrol_persisted(c, store),
                            Child::Expert(c) => r.record_completed_expert_persisted(c, store),
                            Child::Hunt(c) => r.record_completed_hunt_persisted(c, store),
                            _ => anyhow::bail!("体力结果子流程不匹配"),
                        };
                        if let Err(error) = saved {
                            if r.has_pending_commit() {
                                self.snapshot.issue = Some(error.to_string());
                                return Ok(ChargeWorkerEnd::AwaitingSave);
                            }
                            return Err(error);
                        }
                        self.snapshot.schedule = Some(r.snapshot().clone());
                        self.navigate(true)?;
                    }
                    ChargeWorkerStage::Agent => {
                        let Child::Agent(child) = &mut self.child else {
                            anyhow::bail!("代理人子流程不匹配");
                        };
                        self.child_started = true;
                        self.loaded = false;
                        let snapshot = &mut self.snapshot;
                        let registrar = self.registrar.as_ref();
                        let end = child.execute_with_budget(&mut self.session, remaining(), |s| {
                            let has_result = s.result.is_some();
                            snapshot.inputs_released = s.inputs_released;
                            snapshot.agent = Some(s);
                            snapshot.active_ms =
                                initial.saturating_add(started.elapsed().as_millis() as u64);
                            if !has_result {
                                guard(snapshot, registrar, store)?;
                            }
                            publish(snapshot.clone())?;
                            if !has_result {
                                guard(snapshot, registrar, store)?;
                            }
                            Ok(())
                        });
                        // 最后一份检查点回调失败或迟到控制不能丢掉已完成的事实。
                        if child.result_for_parent().is_some() {
                            self.snapshot.agent = Some(child.snapshot());
                            self.snapshot.stage = ChargeWorkerStage::Register;
                            continue;
                        }
                        match end? {
                            AgentChallengeEnd::Paused => return Ok(ChargeWorkerEnd::Paused),
                            AgentChallengeEnd::Stopped => return Ok(ChargeWorkerEnd::Stopped),
                            AgentChallengeEnd::TimedOut => return Ok(ChargeWorkerEnd::TimedOut),
                            AgentChallengeEnd::AwaitingVerification => {
                                self.snapshot.issue =
                                    Some("代理人具体目标或消费条件尚未确认".into());
                                return Ok(ChargeWorkerEnd::AwaitingVerification);
                            }
                            _ => anyhow::bail!("代理人挑战没有本次可信结果"),
                        }
                    }
                    ChargeWorkerStage::Patrol => {
                        let Child::Patrol(child) = &mut self.child else {
                            anyhow::bail!("体力区域子流程不匹配");
                        };
                        self.child_started = true;
                        self.loaded = false;
                        let snapshot = &mut self.snapshot;
                        let registrar = self.registrar.as_ref();
                        let end =
                            child.execute_with_budget(&mut self.session, remaining(), |s| {
                                let has_result = s.stage == PatrolStage::Done
                                    || s.entry.as_ref().is_some_and(|e| e.flow.insufficient());
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.patrol = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                if !has_result {
                                    guard(snapshot, registrar, store)?;
                                }
                                publish(snapshot.clone())?;
                                if !has_result {
                                    guard(snapshot, registrar, store)?;
                                }
                                Ok(())
                            })?;
                        match end {
                            PatrolEnd::Completed | PatrolEnd::InsufficientCharge => {
                                self.snapshot.stage = ChargeWorkerStage::Register
                            }
                            PatrolEnd::Paused => return Ok(ChargeWorkerEnd::Paused),
                            PatrolEnd::Stopped => return Ok(ChargeWorkerEnd::Stopped),
                            PatrolEnd::TimedOut => return Ok(ChargeWorkerEnd::TimedOut),
                            PatrolEnd::Failed => anyhow::bail!("体力区域挑战失败"),
                        }
                        continue;
                    }
                    ChargeWorkerStage::Expert => {
                        let Child::Expert(child) = &mut self.child else {
                            anyhow::bail!("体力专业子流程不匹配");
                        };
                        self.child_started = true;
                        self.loaded = false;
                        let snapshot = &mut self.snapshot;
                        let registrar = self.registrar.as_ref();
                        let end =
                            child.execute_with_budget(&mut self.session, remaining(), |s| {
                                let has_result = s.stage == ExpertStage::Done
                                    || s.entry.as_ref().is_some_and(|e| e.flow.insufficient());
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.expert = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                if !has_result {
                                    guard(snapshot, registrar, store)?;
                                }
                                publish(snapshot.clone())?;
                                if !has_result {
                                    guard(snapshot, registrar, store)?;
                                }
                                Ok(())
                            })?;
                        match end {
                            ExpertEnd::Completed | ExpertEnd::InsufficientCharge => {
                                self.snapshot.stage = ChargeWorkerStage::Register
                            }
                            ExpertEnd::Paused => return Ok(ChargeWorkerEnd::Paused),
                            ExpertEnd::Stopped => return Ok(ChargeWorkerEnd::Stopped),
                            ExpertEnd::TimedOut => return Ok(ChargeWorkerEnd::TimedOut),
                            ExpertEnd::AwaitingVerification => {
                                self.snapshot.issue =
                                    Some("燃竭关闭状态或常规40电量尚未确认，不能消费".into());
                                return Ok(ChargeWorkerEnd::AwaitingVerification);
                            }
                            ExpertEnd::Failed => anyhow::bail!("体力专业挑战失败"),
                        }
                        continue;
                    }
                    ChargeWorkerStage::Hunt => {
                        let Child::Hunt(child) = &mut self.child else {
                            anyhow::bail!("体力追猎子流程不匹配");
                        };
                        self.child_started = true;
                        self.loaded = false;
                        let snapshot = &mut self.snapshot;
                        let registrar = self.registrar.as_ref();
                        let end =
                            child.execute_with_budget(&mut self.session, remaining(), |s| {
                                let has_result = s.stage == HuntStage::Done
                                    || s.entry.as_ref().is_some_and(|e| {
                                        e.flow.insufficient() || e.flow.weekly_blocked().is_some()
                                    });
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.hunt = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                if !has_result {
                                    guard(snapshot, registrar, store)?;
                                }
                                publish(snapshot.clone())?;
                                if !has_result {
                                    guard(snapshot, registrar, store)?;
                                }
                                Ok(())
                            })?;
                        match end {
                            HuntEnd::Completed
                            | HuntEnd::InsufficientCharge
                            | HuntEnd::WeeklyRewardsAvailable => {
                                self.snapshot.stage = ChargeWorkerStage::Register
                            }
                            HuntEnd::Paused => return Ok(ChargeWorkerEnd::Paused),
                            HuntEnd::Stopped => return Ok(ChargeWorkerEnd::Stopped),
                            HuntEnd::TimedOut => return Ok(ChargeWorkerEnd::TimedOut),
                            HuntEnd::AwaitingVerification => {
                                self.snapshot.issue = Some(
                                    "目标、周常耗尽、深度模式或60电量尚未确认，不能消费".into(),
                                );
                                return Ok(ChargeWorkerEnd::AwaitingVerification);
                            }
                            HuntEnd::Failed => anyhow::bail!("体力追猎挑战失败"),
                        }
                        continue;
                    }
                    ChargeWorkerStage::Transport => {
                        let Child::Transport(child) = &mut self.child else {
                            anyhow::bail!("体力传送子流程不匹配");
                        };
                        self.child_started = true;
                        let snapshot = &mut self.snapshot;
                        let registrar = self.registrar.as_ref();
                        let end = child.execute_with_budget(
                            self.session.as_mut().context("体力原生会话缺失")?,
                            self.game,
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.transport = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                guard(snapshot, registrar, store)?;
                                publish(snapshot.clone())?;
                                guard(snapshot, registrar, store)
                            },
                        )?;
                        match end {
                            TransportEnd::Arrived => (),
                            TransportEnd::Paused => return Ok(ChargeWorkerEnd::Paused),
                            TransportEnd::Stopped => return Ok(ChargeWorkerEnd::Stopped),
                            TransportEnd::TimedOut => return Ok(ChargeWorkerEnd::TimedOut),
                            TransportEnd::Failed => anyhow::bail!("体力传送失败"),
                        }
                        let request = MissionRequest::from_plan(self.plan()?, 300_000)
                            .map_err(anyhow::Error::msg)?;
                        let c = MissionRun::new(
                            MissionFlow::new(request).map_err(anyhow::Error::msg)?,
                            self.control.clone(),
                        )?;
                        self.snapshot.mission = Some(c.snapshot.clone());
                        self.child = Child::Mission(c);
                        self.child_started = false;
                        self.snapshot.stage = ChargeWorkerStage::Mission;
                    }
                    ChargeWorkerStage::Mission => {
                        let Child::Mission(child) = &mut self.child else {
                            anyhow::bail!("体力选关子流程不匹配");
                        };
                        self.child_started = true;
                        let snapshot = &mut self.snapshot;
                        let registrar = self.registrar.as_ref();
                        let end = child.execute_with_budget(
                            self.session.as_mut().context("体力原生会话缺失")?,
                            self.game,
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.mission = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                guard(snapshot, registrar, store)?;
                                publish(snapshot.clone())?;
                                guard(snapshot, registrar, store)
                            },
                        )?;
                        match end {
                            MissionEnd::AwaitingVerification => {
                                ensure!(
                                    child
                                        .snapshot
                                        .flow
                                        .receipt()
                                        .is_some_and(|r| !r.selection_verified),
                                    "模拟室选关回执无效"
                                );
                                self.snapshot.issue =
                                    Some("模拟室选中态尚未取得验证，不能进入消费或登记次数".into());
                                return Ok(ChargeWorkerEnd::AwaitingVerification);
                            }
                            MissionEnd::Paused => return Ok(ChargeWorkerEnd::Paused),
                            MissionEnd::Stopped => return Ok(ChargeWorkerEnd::Stopped),
                            MissionEnd::TimedOut => return Ok(ChargeWorkerEnd::TimedOut),
                            MissionEnd::Failed => anyhow::bail!("体力选关失败"),
                        }
                    }
                    ChargeWorkerStage::Finished => {
                        return Ok(match self.snapshot.outcome.context("体力终态缺失")? {
                            ChargeWorkerOutcome::AgentTargetsUnavailable => {
                                ChargeWorkerEnd::AgentTargetsUnavailable
                            }
                            ChargeWorkerOutcome::NeedsVerification => {
                                ChargeWorkerEnd::AwaitingVerification
                            }
                            ChargeWorkerOutcome::Completed => ChargeWorkerEnd::Completed,
                            ChargeWorkerOutcome::WeeklyRewardsAvailable => {
                                ChargeWorkerEnd::WeeklyRewardsAvailable
                            }
                            ChargeWorkerOutcome::NoPlans => ChargeWorkerEnd::NoPlans,
                            ChargeWorkerOutcome::InsufficientCharge => {
                                ChargeWorkerEnd::InsufficientCharge
                            }
                            ChargeWorkerOutcome::UnavailableMaterials => {
                                ChargeWorkerEnd::UnavailableMaterials
                            }
                        });
                    }
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.snapshot.inputs_released = self
            .session
            .as_mut()
            .is_none_or(|n| n.release_inputs().is_ok());
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        self.end = Some(ChargeWorkerEnd::Failed);
        ensure!(self.snapshot.inputs_released, "体力父流程收尾无法释放输入");
        let mut end = result?;
        if end == ChargeWorkerEnd::AwaitingSave {
            self.end = Some(end);
            publish(self.snapshot.clone())?;
            return Ok(end);
        }
        publish(self.snapshot.clone())?;
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = ChargeWorkerEnd::TimedOut;
        }
        if end == ChargeWorkerEnd::Paused && !self.control.acknowledge_pause() {
            end = ChargeWorkerEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
