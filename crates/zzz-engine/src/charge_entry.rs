//! 应用组首个输入之前冻结体力资源、计划及全部启用脚本。
use crate::{
    charge_combat::FrozenChargeCombat,
    charge_worker::{ChargeWorker, ChargeWorkerRequest, Resources},
    simulation_battle::BattleTarget,
    simulation_deploy::PreparedDeployment,
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{NativeSession, combat_vision::CombatBundle};
use od_core::{
    app_combat::AppCombatPlan,
    charge_run::ChargeRunSeed,
    config::{AccountConfig, ConfigStore},
    pipeline::PipelineControl,
    run_plan::GroupPlan,
};
use std::{collections::BTreeSet, path::Path};

pub(crate) fn combat_preview(
    config: &AccountConfig,
    plan: &GroupPlan,
    root: &Path,
    data: &Path,
) -> Result<Option<AppCombatPlan>> {
    if !od_core::charge_entry::needs_combat(&plan.charge_plan_config) {
        return Ok(None);
    }
    let account = config
        .instances
        .iter()
        .find(|a| a.id == plan.context.instance_id)
        .context("体力账号缺失")?;
    let bundle = CombatBundle::load(root)?;
    crate::charge_combat::check_team_agents(
        &plan.charge_plan_config,
        &plan.predefined_teams,
        &bundle,
    )?;
    let revision = crate::combat::script_catalog(&bundle, data)?.revision;
    let mut scripts = BTreeSet::new();
    let mut first = None;
    for item in plan
        .charge_plan_config
        .plan_list
        .iter()
        .filter(|p| p.plan_times > 0 && p.category_name != "合成电池")
    {
        let script = item
            .script(&plan.predefined_teams)
            .map_err(anyhow::Error::msg)?;
        if !scripts.insert(script.to_owned()) {
            continue;
        }
        let mut settings = account.combat.clone();
        settings.script = script.into();
        settings.dodge_only = false;
        settings.start_delay_ms = 0;
        let frozen = AppCombatPlan {
            config: settings,
            script_revision: revision.clone(),
        };
        PreparedDeployment::load(frozen.clone(), &bundle.root, data)?;
        if first.is_none() {
            first = Some(frozen);
        }
    }
    ensure!(first.is_some(), "体力战斗脚本缺失");
    Ok(first)
}

pub(crate) struct PreparedCharge {
    request: ChargeWorkerRequest,
    resources: Resources,
    combat: Option<FrozenChargeCombat>,
    target: BattleTarget,
}
impl PreparedCharge {
    pub(crate) fn after_team_scan(mut self, record: &od_core::RunRecord) -> Result<Self> {
        let source = &record.charge.as_ref().context("体力启动摘要缺失")?.context;
        if source.config_revision != self.request.seed.context.config_revision {
            let scan = record
                .daily
                .iter()
                .find_map(|p| p.team_check.as_ref())
                .context("体力缺少前序编队保存证明")?;
            ensure!(
                scan.config == self.request.seed.teams
                    && scan.saved_revision == Some(source.config_revision),
                "前序编队与体力冻结配置不符"
            );
            self.request.seed.teams = scan.merged().map_err(anyhow::Error::msg)?;
            self.request.seed.context.config_revision = source.config_revision;
        }
        if let Some(combat) = &self.combat {
            combat.verify_teams(&self.request.seed)?;
        }
        Ok(self)
    }
    pub(crate) fn load(
        plan: &GroupPlan,
        store: &ConfigStore,
        root: &Path,
        target: BattleTarget,
    ) -> Result<Self> {
        od_core::charge_entry::validate(&plan.charge_plan_config).map_err(anyhow::Error::msg)?;
        let config = store.read()?;
        let seed = ChargeRunSeed::from_config(
            &config,
            plan.context.instance_id,
            &plan.context.group_id,
            uuid::Uuid::new_v4(),
            chrono::Utc::now(),
        )
        .map_err(anyhow::Error::msg)?;
        ensure!(
            seed.context.config_revision == plan.context.config_revision
                && seed.context.game_day == plan.game_day
                && seed.config == *plan.charge_plan_config
                && seed.teams == plan.predefined_teams,
            "体力预检与冻结应用组不符"
        );
        let request = ChargeWorkerRequest {
            seed,
            timeout_ms: od_core::charge_entry::TIMEOUT_MS,
        };
        request.validate()?;
        let resources = Resources::for_request(root, &request)?;
        let combat = if od_core::charge_entry::needs_combat(&request.seed.config) {
            let c = FrozenChargeCombat::load(
                &request,
                root,
                &root.join("generated/combat"),
                store,
                &target,
            )?;
            let preview = plan
                .charge_combat
                .as_ref()
                .context("体力冻结战斗预览缺失")?;
            let mut expected = c.settings.clone();
            expected.script = preview.config.script.clone();
            expected.dodge_only = false;
            expected.start_delay_ms = 0;
            ensure!(
                expected == preview.config && c.revision == preview.script_revision,
                "体力战斗设置或脚本已变化，请重新预览"
            );
            Some(c)
        } else {
            None
        };
        Ok(Self {
            request,
            resources,
            combat,
            target,
        })
    }
    pub(crate) fn verify(&self) -> Result<()> {
        self.resources.verify()?;
        if let Some(c) = &self.combat {
            c.verify()?;
        }
        Ok(())
    }
    pub(crate) fn create(
        mut self,
        id: uuid::Uuid,
        session: NativeSession,
        control: PipelineControl,
    ) -> Result<ChargeWorker> {
        self.verify()?;
        let game = matches!(self.target, BattleTarget::Game { .. });
        match (&self.target, session.game_capture_target()?) {
            (BattleTarget::Fixture, None) => (),
            (BattleTarget::Game { window, capture }, Some((actual, method))) => {
                ensure!(
                    capture == &method
                        && serde_json::to_value(window)? == serde_json::to_value(actual)?,
                    "体力目标窗口或截图方式已变化"
                );
            }
            _ => anyhow::bail!("体力会话与预检目标不符"),
        }
        self.request.seed.context.run_id = id;
        ChargeWorker::from_frozen(
            self.request,
            self.resources,
            session,
            game,
            control,
            self.combat,
        )
    }
}
