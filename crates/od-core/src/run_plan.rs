use crate::{
    applications,
    config::{AccountConfig, valid_identifier},
    daily::{GameDay, RunPolicy},
    run_history::DailyCompletions,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const TRIAL_APP_IDS: [&str; 13] = [
    "email",
    "coffee",
    "engagement_reward",
    "city_fund",
    "scratch_card",
    "ridu_weekly",
    "trigrams_collection",
    "hou_hou_bakery",
    "drive_disc_dismantle",
    "redemption_code",
    "random_play",
    "predefined_team_checker",
    "intel_board",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RunContext {
    pub instance_id: u32,
    pub instance_name: String,
    pub group_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub group_name: Option<String>,
    pub config_revision: u32,
    pub app_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub rerun_completed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub standalone: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub game_day: Option<GameDay>,
}

impl RunContext {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(name) = &self.group_name
            && (name.trim().is_empty()
                || name.chars().count() > 120
                || name.chars().any(char::is_control))
        {
            return Err("运行记录的应用组名称无效");
        }
        if let Some(day) = &self.game_day {
            day.validate()?;
        }
        if !(1..=9999).contains(&self.instance_id)
            || self.instance_name.trim().is_empty()
            || self.instance_name.chars().count() > 80
            || self.instance_name.chars().any(char::is_control)
            || !valid_identifier(&self.group_id)
            || self.app_ids.is_empty()
            || self.app_ids.len() > 256
            || self.app_ids.iter().any(|id| !valid_identifier(id))
        {
            return Err("运行记录的账号或应用组信息无效");
        }
        let unique: std::collections::HashSet<_> = self.app_ids.iter().collect();
        if unique.len() != self.app_ids.len() {
            return Err("运行计划的应用重复");
        }
        if self.standalone && (self.app_ids.len() != 1 || self.rerun_completed) {
            return Err("独立运行必须只包含一个应用");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AppAvailability {
    MailTrial,
    CoffeeTrial,
    DailyTrial,
    Unmigrated,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PlanDecision {
    Disabled,
    Run,
    AlreadyDone,
    NoWork,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PlannedApp {
    pub app_id: String,
    pub name: String,
    pub enabled: bool,
    pub availability: AppAvailability,
    pub decision: PlanDecision,
    pub completed_run_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GroupPlan {
    #[serde(default)]
    #[ts(as = "Option<crate::suibian_temple::SuibianTempleConfig>", optional)]
    pub suibian_temple_config: crate::suibian_temple::SuibianTempleConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub app_combat: Option<crate::app_combat::AppCombatPlan>,
    #[serde(default)]
    #[ts(as = "Option<crate::intel_board::IntelBoardConfig>", optional)]
    pub intel_board_config: crate::intel_board::IntelBoardConfig,
    #[serde(default)]
    #[ts(as = "Option<crate::intel_board::IntelHistory>", optional)]
    pub intel_board_history: crate::intel_board::IntelHistory,
    #[serde(default)]
    #[ts(as = "Option<crate::predefined_teams::TeamConfig>", optional)]
    pub predefined_teams: crate::predefined_teams::TeamConfig,
    #[serde(default)]
    #[ts(as = "Option<crate::random_play::RandomPlayConfig>", optional)]
    pub random_play_config: crate::random_play::RandomPlayConfig,
    #[serde(default)]
    #[ts(as = "Option<Vec<crate::redemption::RedemptionCode>>", optional)]
    pub redemption_codes: Vec<crate::redemption::RedemptionCode>,
    #[serde(default)]
    #[ts(as = "Option<crate::redemption::RedemptionHistory>", optional)]
    pub redemption_history: crate::redemption::RedemptionHistory,
    #[serde(default)]
    pub after_done: crate::after_done::AfterDoneAction,
    pub context: RunContext,
    pub mail_config: crate::mail::MailConfig,
    pub coffee_config: crate::coffee_run::CoffeeConfig,
    #[serde(default)]
    #[ts(as = "Option<crate::basic_daily::DailyConfig>", optional)]
    pub daily_config: crate::basic_daily::DailyConfig,
    #[serde(default)]
    #[ts(as = "Option<crate::dismantle::DismantleConfig>", optional)]
    pub dismantle_config: crate::dismantle::DismantleConfig,
    pub items: Vec<PlannedApp>,
    pub blockers: Vec<String>,
    pub executable: bool,
    pub all_done: bool,
    pub game_day: GameDay,
    pub run_policy: RunPolicy,
    pub warnings: Vec<String>,
    #[ts(type = "string")]
    pub valid_until: DateTime<Utc>,
}

pub fn plan_group(
    config: &AccountConfig,
    instance_id: u32,
    group_id: &str,
) -> Result<GroupPlan, String> {
    plan_group_at(config, instance_id, group_id, Utc::now(), None)
}

pub fn plan_group_at(
    config: &AccountConfig,
    instance_id: u32,
    group_id: &str,
    now: DateTime<Utc>,
    completed: Option<&DailyCompletions>,
) -> Result<GroupPlan, String> {
    plan_group_selection_at(config, instance_id, group_id, now, completed, None)
}

/// 本次选择只替代启用项，顺序与参数仍来自保存的组，不改写配置。
pub fn plan_group_selection_at(
    config: &AccountConfig,
    instance_id: u32,
    group_id: &str,
    now: DateTime<Utc>,
    completed: Option<&DailyCompletions>,
    selection: Option<&[String]>,
) -> Result<GroupPlan, String> {
    plan_group_with_options_at(
        config,
        instance_id,
        group_id,
        now,
        completed,
        GroupRunOptions {
            app_ids: selection,
            rerun_completed: false,
        },
    )
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GroupRunOptions<'a> {
    pub app_ids: Option<&'a [String]>,
    pub rerun_completed: bool,
}

/// 临时运行选项只影响当前计划，保留保存策略及已有每日证明。
pub fn plan_group_with_options_at(
    config: &AccountConfig,
    instance_id: u32,
    group_id: &str,
    now: DateTime<Utc>,
    completed: Option<&DailyCompletions>,
    options: GroupRunOptions<'_>,
) -> Result<GroupPlan, String> {
    plan_account_at(
        config,
        instance_id,
        group_id,
        now,
        completed,
        options,
        false,
    )
}

fn plan_account_at(
    config: &AccountConfig,
    instance_id: u32,
    group_id: &str,
    now: DateTime<Utc>,
    completed: Option<&DailyCompletions>,
    options: GroupRunOptions<'_>,
    standalone: bool,
) -> Result<GroupPlan, String> {
    let selection = options.app_ids;
    config.validate().map_err(|error| error.to_string())?;
    let instance = config
        .instances
        .iter()
        .find(|item| item.id == instance_id)
        .ok_or("账号不存在")?;
    let group = instance
        .groups
        .iter()
        .find(|item| item.id == group_id)
        .ok_or("应用组不存在")?;
    if let Some(ids) = selection {
        if ids.is_empty() || ids.len() > 256 {
            return Err("本次请选择 1..256 个应用".into());
        }
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        if unique.len() != ids.len() {
            return Err("本次所选应用不能重复".into());
        }
        if ids
            .iter()
            .any(|id| !group.apps.iter().any(|app| app.app_id == *id))
        {
            return Err("本次所选应用不属于当前应用组，请重新读取配置".into());
        }
    }
    let game_day = GameDay::at(instance.game_region, now).map_err(str::to_owned)?;
    if completed.is_some_and(|index| index.game_day != game_day) {
        return Err("完成记录的游戏日与计划不符，请重新预览".into());
    }
    let mut warnings = Vec::new();
    if standalone {
        warnings.push("独立运行每次执行所选应用；不会重置每日记录或增加游戏内领取次数".into());
    }
    if options.rerun_completed {
        warnings.push(
            "仅本次重新运行所选应用，包括本游戏日已完成项；不会重置每日记录或增加游戏内领取次数"
                .into(),
        );
    }
    if let Some(index) = completed
        && index.unreadable > 0
    {
        warnings.push(format!(
            "{} 份账号记录无法读取，不作为当日完成；已保留原文件",
            index.unreadable
        ));
    }
    let catalog = applications().map_err(|_| "内置应用登记无效")?;
    let mut blockers = Vec::new();
    if config.active_instance != instance_id {
        blockers.push("请先将所选账号设为当前账号，并确认游戏登录账号".into());
    }
    let mut items = Vec::new();
    let mut app_ids = Vec::new();
    for item in &group.apps {
        let enabled = selection.map_or(item.enabled, |ids| ids.contains(&item.app_id));
        let definition = catalog.iter().find(|app| app.id == item.app_id);
        let name = definition.map_or_else(|| item.app_id.clone(), |app| app.name.clone());
        // 是否有执行器由业务实现决定，登记的 implemented 标记不能开放未知运行入口。
        let availability = match (definition.is_some(), item.app_id.as_str()) {
            (true, "email") => AppAvailability::MailTrial,
            (true, "coffee") => AppAvailability::CoffeeTrial,
            (true, id) if crate::basic_daily::APP_IDS.contains(&id) => AppAvailability::DailyTrial,
            (true, _) => AppAvailability::Unmigrated,
            (false, _) => AppAvailability::Unknown,
        };
        let mut decision = PlanDecision::Disabled;
        let mut completed_run_id = None;
        if enabled {
            app_ids.push(item.app_id.clone());
            match availability {
                AppAvailability::MailTrial
                | AppAvailability::CoffeeTrial
                | AppAvailability::DailyTrial => {
                    completed_run_id = completed
                        .and_then(|index| index.apps.get(&item.app_id))
                        .copied();
                    decision = if group.run_policy == RunPolicy::SkipDailySuccess
                        && !options.rerun_completed
                        && !standalone
                        && completed_run_id.is_some()
                    {
                        PlanDecision::AlreadyDone
                    } else {
                        PlanDecision::Run
                    };
                }
                AppAvailability::Unmigrated => {
                    decision = PlanDecision::Blocked;
                    blockers.push(format!("{name} 尚无可用执行器，请先禁用该项"))
                }
                AppAvailability::Unknown => {
                    decision = PlanDecision::Blocked;
                    blockers.push(format!("未注册应用 {name}，请先禁用该项"))
                }
            }
        }
        items.push(PlannedApp {
            app_id: item.app_id.clone(),
            name,
            enabled,
            availability,
            decision,
            completed_run_id,
        });
    }
    if app_ids.is_empty() {
        blockers.push("应用组没有启用的应用".into());
    }
    if items
        .iter()
        .any(|item| item.app_id == "drive_disc_dismantle" && item.decision == PlanDecision::Run)
    {
        warnings.push(format!(
            "驱动盘拆解：{}，{}全选已弃置；所选驱动盘将被消耗。",
            group.dismantle.grade.label(),
            if group.dismantle.include_abandoned {
                ""
            } else {
                "不"
            }
        ));
    }
    let redemption_codes = config
        .redemption
        .pending(&game_day.date, &instance.redemption_used)
        .map_err(str::to_owned)?;
    if let Some(item) = items
        .iter_mut()
        .find(|item| item.app_id == "redemption_code" && item.enabled)
    {
        item.decision = if redemption_codes.is_empty() {
            PlanDecision::NoWork
        } else {
            PlanDecision::Run
        };
    }
    let has_pending = items.iter().any(|item| item.decision == PlanDecision::Run);
    let executable = blockers.is_empty() && has_pending;
    let all_done = blockers.is_empty() && !has_pending;
    Ok(GroupPlan {
        suibian_temple_config: group.suibian_temple.clone(),
        app_combat: None,
        intel_board_config: group.intel_board.clone(),
        intel_board_history: Default::default(),
        predefined_teams: instance.predefined_teams.clone(),
        redemption_codes,
        redemption_history: Default::default(),
        after_done: if standalone {
            crate::after_done::AfterDoneAction::None
        } else {
            config.general.after_done
        },
        valid_until: game_day.ends_at().map_err(str::to_owned)?,
        all_done,
        game_day: game_day.clone(),
        run_policy: if standalone {
            RunPolicy::Always
        } else {
            group.run_policy
        },
        warnings,
        mail_config: config.general.mail,
        coffee_config: group.coffee,
        daily_config: group.daily,
        dismantle_config: group.dismantle,
        random_play_config: group.random_play.clone(),
        context: RunContext {
            instance_id,
            instance_name: instance.name.clone(),
            group_id: group.id.clone(),
            group_name: Some(group.display_name().into()),
            config_revision: config.revision,
            app_ids,
            rerun_completed: options.rerun_completed,
            standalone,
            game_day: Some(game_day),
        },
        items,
        blockers,
        executable,
    })
}

/// 独立运行只借用组的参数，不改变启用项、每日策略和结束后动作。
pub fn plan_standalone_at(
    config: &AccountConfig,
    instance_id: u32,
    group_id: &str,
    app_id: &str,
    now: DateTime<Utc>,
    completed: Option<&DailyCompletions>,
) -> Result<GroupPlan, String> {
    let selection = [app_id.to_owned()];
    plan_account_at(
        config,
        instance_id,
        group_id,
        now,
        completed,
        GroupRunOptions {
            app_ids: Some(&selection),
            rerun_completed: false,
        },
        true,
    )
}
