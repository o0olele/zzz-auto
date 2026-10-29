//! 体力父循环首个输入之前冻结区域战斗能力；循环中只克隆已编译脚本。
use crate::{
    agent_challenge::{AgentChallengeRequest, AgentChallengeRun, AgentResources},
    area_patrol::{AreaPatrolRun, PatrolRequest, PatrolResources},
    charge_worker::ChargeWorkerRequest,
    deep_hunt::{DeepHuntRun, HuntRequest, HuntResources},
    expert_challenge::{ExpertChallengeRun, ExpertRequest, ExpertResources},
    simulation_battle::BattleTarget,
    simulation_deploy::PreparedDeployment,
};
use anyhow::{Context, Result, ensure};
use maa_adapter::combat_vision::CombatBundle;
use od_core::{
    InputMode, app_combat::AppCombatPlan, charge_run::ChargeRunSeed, combat::CombatConfig,
    config::ConfigStore, pipeline::PipelineControl,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub(crate) fn check_team_agents(
    config: &od_core::charge_plan::ChargePlanConfig,
    teams: &od_core::predefined_teams::TeamConfig,
    bundle: &CombatBundle,
) -> Result<()> {
    for plan in config
        .plan_list
        .iter()
        .filter(|p| p.plan_times > 0 && p.category_name != "合成电池" && p.predefined_team_idx >= 0)
    {
        for agent in &teams
            .get(plan.predefined_team_idx as u8)
            .map_err(anyhow::Error::msg)?
            .agent_ids
        {
            ensure!(
                agent == od_core::predefined_teams::UNKNOWN_AGENT
                    || bundle.spec.agents.iter().any(|a| &a.id == agent),
                "战斗资源缺少编队代理人 {agent}，请更新战斗资源"
            );
        }
    }
    Ok(())
}

pub(crate) struct FrozenChargeCombat {
    agent_ids: BTreeSet<String>,
    pub(crate) settings: CombatConfig,
    pub(crate) revision: String,
    prepared: BTreeMap<String, PreparedDeployment>,
    resources: Option<PatrolResources>,
    expert_resources: Option<ExpertResources>,
    hunt_resources: Option<HuntResources>,
    agent_resources: BTreeMap<String, AgentResources>,
}
impl FrozenChargeCombat {
    pub(crate) fn load(
        request: &ChargeWorkerRequest,
        root: &Path,
        combat_root: &Path,
        store: &ConfigStore,
        target: &BattleTarget,
    ) -> Result<Self> {
        let config = store.read()?;
        let seed = &request.seed;
        ensure!(
            ChargeRunSeed::from_config(
                &config,
                seed.context.instance_id,
                &seed.context.group_id,
                seed.context.run_id,
                chrono::Utc::now()
            )
            .map_err(anyhow::Error::msg)?
                == *seed,
            "体力战斗准备与当前账号、修订或冻结计划不符"
        );
        let settings = config
            .instances
            .iter()
            .find(|a| a.id == seed.context.instance_id)
            .context("体力战斗账号缺失")?
            .combat
            .clone();
        if matches!(target, BattleTarget::Fixture) {
            ensure!(
                settings.mode == InputMode::Foreground && settings.directml_device.is_none(),
                "隔离体力战斗只允许前台 CPU 控制器"
            );
        }
        if let BattleTarget::Game { window, .. } = target {
            ensure!(
                window.width == 1920 && window.height == 1080,
                "体力战斗须冻结 1080p 窗口"
            );
        }
        let active = |category: &str| {
            seed.config
                .plan_list
                .iter()
                .any(|p| p.plan_times > 0 && p.category_name == category)
        };
        let resources = active("区域巡防")
            .then(|| PatrolResources::load(root, combat_root))
            .transpose()?;
        let expert_resources = active("专业挑战室")
            .then(|| ExpertResources::load(root, combat_root))
            .transpose()?;
        let hunt_resources = active("恶名狩猎")
            .then(|| {
                HuntResources::load(
                    root,
                    combat_root,
                    seed.config.plan_list.iter().any(|p| {
                        p.plan_times > 0 && p.category_name == "恶名狩猎" && p.level != "默认等级"
                    }),
                )
            })
            .transpose()?;
        let bundle = CombatBundle::load(combat_root)?;
        check_team_agents(&seed.config, &seed.teams, &bundle)?;
        let mut agent_resources = BTreeMap::new();
        for p in seed
            .config
            .plan_list
            .iter()
            .filter(|p| p.plan_times > 0 && p.mission_type_name == "代理人方案培养")
        {
            let r = AgentResources::load(
                &AgentChallengeRequest {
                    plan: p.clone(),
                    teams: seed.teams.clone(),
                    restore_charge: seed.config.restore_charge,
                    timeout_ms: 900_000,
                },
                root,
                combat_root,
            )?;
            agent_resources.insert(p.plan_id.clone(), r);
        }
        let revision = crate::combat::script_catalog(&bundle, store.data_directory())?.revision;
        let mut prepared = BTreeMap::new();
        for p in seed.config.plan_list.iter().filter(|p| {
            p.plan_times > 0
                && ["区域巡防", "专业挑战室", "恶名狩猎"].contains(&p.category_name.as_str())
        }) {
            let script = p.script(&seed.teams).map_err(anyhow::Error::msg)?;
            if !prepared.contains_key(script) {
                let mut combat = settings.clone();
                combat.script = script.into();
                combat.dodge_only = false;
                combat.start_delay_ms = 0;
                let p = PreparedDeployment::load(
                    AppCombatPlan {
                        config: combat,
                        script_revision: revision.clone(),
                    },
                    &bundle.root,
                    store.data_directory(),
                )?;
                prepared.insert(script.to_string(), p);
            }
        }
        ensure!(!prepared.is_empty(), "体力战斗准备缺少启用的训练计划");
        Ok(Self {
            agent_ids: bundle.spec.agents.iter().map(|a| a.id.clone()).collect(),
            settings,
            revision,
            prepared,
            resources,
            expert_resources,
            hunt_resources,
            agent_resources,
        })
    }
    pub(crate) fn create(
        &self,
        id: uuid::Uuid,
        request: PatrolRequest,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<AreaPatrolRun> {
        let script = request
            .plan
            .script(&request.teams)
            .map_err(anyhow::Error::msg)?;
        let prepared = self
            .prepared
            .get(script)
            .context("体力运行缺少预先编译的区域脚本")?
            .clone();
        AreaPatrolRun::from_frozen(
            id,
            request,
            self.resources.as_ref().context("区域资源未冻结")?.clone(),
            prepared,
            target,
            control,
        )
    }
    pub(crate) fn verify_teams(&self, seed: &ChargeRunSeed) -> Result<()> {
        for plan in seed.config.plan_list.iter().filter(|p| {
            p.plan_times > 0 && p.category_name != "合成电池" && p.predefined_team_idx >= 0
        }) {
            for agent in &seed
                .teams
                .get(plan.predefined_team_idx as u8)
                .map_err(anyhow::Error::msg)?
                .agent_ids
            {
                ensure!(
                    agent == od_core::predefined_teams::UNKNOWN_AGENT
                        || self.agent_ids.contains(agent),
                    "冻结战斗资源缺少前序扫描代理人 {agent}，不能继续体力输入"
                );
            }
        }
        Ok(())
    }
    pub(crate) fn verify(&self) -> Result<()> {
        for r in self.agent_resources.values() {
            r.verify()?;
        }
        if let Some(r) = &self.resources {
            r.verify()?;
        }
        if let Some(r) = &self.expert_resources {
            r.verify()?;
        }
        if let Some(r) = &self.hunt_resources {
            r.verify()?;
        }
        Ok(())
    }
    pub(crate) fn create_agent(
        &self,
        id: uuid::Uuid,
        request: AgentChallengeRequest,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<AgentChallengeRun> {
        let script = request
            .plan
            .script(&request.teams)
            .map_err(anyhow::Error::msg)?;
        AgentChallengeRun::from_frozen(
            id,
            request.clone(),
            self.agent_resources
                .get(&request.plan.plan_id)
                .context("代理人资源未在首个输入前冻结")?
                .clone(),
            self.prepared
                .get(script)
                .context("代理人脚本未预先编译")?
                .clone(),
            target,
            control,
        )
    }
    pub(crate) fn create_expert(
        &self,
        id: uuid::Uuid,
        request: ExpertRequest,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<ExpertChallengeRun> {
        let script = request
            .plan
            .script(&request.teams)
            .map_err(anyhow::Error::msg)?;
        let prepared = self
            .prepared
            .get(script)
            .context("专业脚本未预先编译")?
            .clone();
        ExpertChallengeRun::from_frozen(
            id,
            request,
            self.expert_resources
                .as_ref()
                .context("专业资源未冻结")?
                .clone(),
            prepared,
            target,
            control,
        )
    }
    pub(crate) fn create_hunt(
        &self,
        id: uuid::Uuid,
        request: HuntRequest,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<DeepHuntRun> {
        let script = request
            .plan
            .script(&request.teams)
            .map_err(anyhow::Error::msg)?;
        let prepared = self
            .prepared
            .get(script)
            .context("追猎脚本未预先编译")?
            .clone();
        DeepHuntRun::from_frozen(
            id,
            request,
            self.hunt_resources
                .as_ref()
                .context("追猎资源未冻结")?
                .clone(),
            prepared,
            target,
            control,
        )
    }
}
