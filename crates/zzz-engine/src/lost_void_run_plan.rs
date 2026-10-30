//! 组首次输入前固定迷失之地的全部候选；预览只读，实际启动只补交旧记录。
use crate::{
    lost_void_application::{LostVoidApplicationRun, LostVoidApplicationSpec},
    lost_void_combat::{FrozenLostVoidPreparation, LostVoidCombatSeed, LostVoidResourcePaths},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    RunRecord,
    app_combat::AppCombatPlan,
    config::AccountConfig,
    lost_void::{LostVoidDecision, plan as decide},
    lost_void_record_store::{LostVoidApplicationPlanning, LostVoidRecordStore},
    pipeline::PipelineControl,
    run_plan::{GroupPlan, PlanDecision},
};
use std::path::Path;
use uuid::Uuid;

fn freeze(
    plan: &GroupPlan,
    combat: &AppCombatPlan,
    resources: &Path,
    data: &Path,
) -> Result<FrozenLostVoidPreparation> {
    combat.validate().map_err(anyhow::Error::msg)?;
    let seed = LostVoidCombatSeed {
        run_id: Uuid::new_v4(),
        mission: plan.lost_void_config.mission_name,
        challenge: plan.lost_void_config.challenge.clone(),
        teams: plan.predefined_teams.clone(),
        settings: combat.config.clone(),
        script_revision: combat.script_revision.clone(),
        // 后续跨周、UP 完成或取消优先时，只能消费首次固定的完整候选族。
        up_completed_this_week: false,
    };
    seed.validate_roster_settings()?;
    FrozenLostVoidPreparation::load(
        seed,
        LostVoidResourcePaths {
            scene: &resources.join("generated/lost-void-scene"),
            daily: Some(&resources.join("generated/basic-daily")),
            combat: &resources.join("generated/combat"),
        },
        data,
    )?
    .with_npc_detector(&resources.join("generated/basic-daily"))
}

/// 不创建日志、运行锁或控制器；启动时仍须重新核对日周记录及冻结资源。
pub fn preview(
    plan: &mut GroupPlan,
    config: &AccountConfig,
    resources: &Path,
    data: &Path,
) -> Result<()> {
    if !plan
        .items
        .iter()
        .any(|i| i.app_id == "lost_void" && i.enabled && i.decision == PlanDecision::Run)
    {
        return Ok(());
    }
    let store = LostVoidRecordStore::new(data)?;
    let planning = LostVoidApplicationPlanning {
        progress: store.progress(plan.context.instance_id, &plan.game_day)?,
        pending_claims: store.pending_claims(plan.context.instance_id, &plan.game_day)?,
        recovered_settlements: 0,
    };
    let decision = decide(
        &plan.lost_void_config,
        &plan.game_day,
        &planning.progress.history(),
    )
    .map_err(anyhow::Error::msg)?;
    if decision != LostVoidDecision::Run && planning.pending_claims.is_empty() {
        plan.warnings.push("迷失之地只读进度显示当前目标已满足；实际开始仍先补交原记录并重新核对，不会仅凭预览跳过待领取。".into());
    }
    let account = config
        .instances
        .iter()
        .find(|a| a.id == plan.context.instance_id)
        .context("迷失之地账号缺失")?;
    let bundle =
        maa_adapter::combat_vision::CombatBundle::load(&resources.join("generated/combat"))?;
    let mut settings = account.combat.clone();
    settings.script = plan.lost_void_config.challenge.auto_battle.clone();
    settings.dodge_only = false;
    settings.start_delay_ms = 0;
    let combat = AppCombatPlan {
        config: settings,
        script_revision: crate::combat::script_catalog(&bundle, data)?.revision,
    };
    let frozen = freeze(plan, &combat, resources, data)?;
    plan.warnings.push(format!(
        "迷失之地每次核对日周目标及待领取；首次输入前冻结 {} 个候选战斗脚本，使用前台键鼠。",
        frozen.script_names().len()
    ));
    plan.lost_void_combat = Some(combat);
    plan.lost_void_planning = Some(planning);
    plan.executable =
        plan.blockers.is_empty() && plan.items.iter().any(|i| i.decision == PlanDecision::Run);
    plan.all_done = plan.blockers.is_empty() && !plan.executable;
    Ok(())
}

pub struct PreparedLostVoidApp {
    frozen: FrozenLostVoidPreparation,
    spec: LostVoidApplicationSpec,
    context: od_core::run_plan::RunContext,
}
impl PreparedLostVoidApp {
    pub fn load(plan: &GroupPlan, resources: &Path, data: &Path) -> Result<Self> {
        plan.context.validate().map_err(anyhow::Error::msg)?;
        let replay = od_core::run_history::RunStore::new(data)?
            .replay_lost_void_pending(plan.context.instance_id)?;
        ensure!(
            replay.unreadable == 0,
            "账号存在无法核验的历史，不能把缺失的迷失之地事实作为零进度；原文件已保留"
        );
        LostVoidRecordStore::new(data)?
            .application_planning(plan.context.instance_id, &plan.game_day)?;
        let frozen = freeze(
            plan,
            plan.lost_void_combat
                .as_ref()
                .context("缺少迷失之地冻结战斗设置")?,
            resources,
            data,
        )?;
        Ok(Self {
            frozen,
            context: plan.context.clone(),
            spec: LostVoidApplicationSpec {
                instance_id: plan.context.instance_id,
                region: plan.game_day.region,
                config: plan.lost_void_config.clone(),
                options: Default::default(),
                timeout_ms: od_core::lost_void_application::DEFAULT_APPLICATION_TIMEOUT_MS,
            },
        })
    }
    pub fn verify(&self) -> Result<()> {
        self.frozen.verify()
    }
    pub fn create(
        self,
        record: &RunRecord,
        data: &Path,
        control: PipelineControl,
    ) -> Result<LostVoidApplicationRun> {
        ensure!(
            record.context.as_ref() == Some(&self.context) && record.matches_account_context(),
            "迷失之地交接与冻结账号计划不符"
        );
        let progress = record.lost_void.as_ref().context("迷失之地启动摘要缺失")?;
        ensure!(
            progress.config == self.spec.config
                && progress.active_ms == 0
                && progress.challenges_started == 0
                && progress.end.is_none(),
            "迷失之地只能从本次未执行分项创建实际对象"
        );
        let mut frozen = self.frozen.for_next_challenge(record.id, false)?;
        if let Some(scan) = record.daily.iter().find_map(|p| p.team_check.as_ref()) {
            ensure!(
                scan.config == frozen.seed().teams && scan.saved_revision.is_some(),
                "前序编队检查与迷失之地冻结配置不符"
            );
            frozen = frozen.with_group_teams(scan.merged().map_err(anyhow::Error::msg)?)?;
        }
        LostVoidApplicationRun::new(self.spec, frozen, data, control)?
            .with_run_context(progress.context.clone())
    }
}
