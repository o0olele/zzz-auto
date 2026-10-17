use crate::{
    applications,
    config::{AccountConfig, valid_identifier},
    daily::{GameDay, RunPolicy},
    run_history::DailyCompletions,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

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
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AppAvailability {
    MailTrial,
    Unmigrated,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PlanDecision {
    Disabled,
    Run,
    AlreadyDone,
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
    pub context: RunContext,
    pub mail_config: crate::mail::MailConfig,
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
    let game_day = GameDay::at(instance.game_region, now).map_err(str::to_owned)?;
    if completed.is_some_and(|index| index.game_day != game_day) {
        return Err("完成记录的游戏日与计划不符，请重新预览".into());
    }
    let mut warnings = Vec::new();
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
        let definition = catalog.iter().find(|app| app.id == item.app_id);
        let name = definition.map_or_else(|| item.app_id.clone(), |app| app.name.clone());
        // 是否有执行器由业务实现决定，登记的 implemented 标记不能开放未知运行入口。
        let availability = match (definition.is_some(), item.app_id.as_str()) {
            (true, "email") => AppAvailability::MailTrial,
            (true, _) => AppAvailability::Unmigrated,
            (false, _) => AppAvailability::Unknown,
        };
        let mut decision = PlanDecision::Disabled;
        let mut completed_run_id = None;
        if item.enabled {
            app_ids.push(item.app_id.clone());
            match availability {
                AppAvailability::MailTrial => {
                    completed_run_id = completed
                        .and_then(|index| index.apps.get(&item.app_id))
                        .copied();
                    decision = if group.run_policy == RunPolicy::SkipDailySuccess
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
            enabled: item.enabled,
            availability,
            decision,
            completed_run_id,
        });
    }
    if app_ids.is_empty() {
        blockers.push("应用组没有启用的应用".into());
    }
    let has_pending = items.iter().any(|item| item.decision == PlanDecision::Run);
    let executable = blockers.is_empty() && has_pending;
    let all_done = blockers.is_empty() && !has_pending;
    Ok(GroupPlan {
        valid_until: game_day.ends_at().map_err(str::to_owned)?,
        all_done,
        game_day: game_day.clone(),
        run_policy: group.run_policy,
        warnings,
        mail_config: config.general.mail,
        context: RunContext {
            instance_id,
            instance_name: instance.name.clone(),
            group_id: group.id.clone(),
            group_name: Some(group.display_name().into()),
            config_revision: config.revision,
            app_ids,
            game_day: Some(game_day),
        },
        items,
        blockers,
        executable,
    })
}
