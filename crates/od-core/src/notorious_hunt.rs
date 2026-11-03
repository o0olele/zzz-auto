//! 周常免费恶名狩猎的配置和只读估算，不提供输入或成功证明。
use crate::{charge_plan::ChargePlanItem, daily::GameDay, predefined_teams::TeamConfig};
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct NotoriousHuntConfig {
    pub plan_list: Vec<ChargePlanItem>,
    pub r#loop: bool,
    pub weekly_challenge_start_weekday: u8,
}
impl Default for NotoriousHuntConfig {
    fn default() -> Self {
        Self {
            plan_list: vec![],
            r#loop: true,
            weekly_challenge_start_weekday: 1,
        }
    }
}
pub fn default_item() -> ChargePlanItem {
    ChargePlanItem {
        category_name: "恶名狩猎".into(),
        mission_type_name: "初生死路屠夫".into(),
        mission_name: None,
        ..Default::default()
    }
}
impl NotoriousHuntConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=7).contains(&self.weekly_challenge_start_weekday)
            || self.plan_list.len() > crate::charge_plan::MAX_PLANS
        {
            return Err("恶名狩猎开始星期须为 1..7，每组最多 64 条计划");
        }
        let mut ids = BTreeSet::new();
        for item in &self.plan_list {
            item.validate()?;
            if item.category_name != "恶名狩猎" || item.card_num.is_some() {
                return Err("周常恶名狩猎只能保存恶名狩猎副本，不能指定模拟室卡片");
            }
            if !ids
                .insert(uuid::Uuid::parse_str(&item.plan_id).map_err(|_| "恶名狩猎计划 ID 无效")?)
            {
                return Err("恶名狩猎计划 ID 不能重复");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum HuntDecision {
    NoPlans,
    BeforeStartDay,
    UnknownRewards,
    Exhausted,
    Completed,
    Ready,
    NeedsAgentTarget,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HuntPreviewItem {
    pub plan_id: String,
    pub effective_run_times: u32,
    pub script: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HuntAttempt {
    pub plan_id: String,
    pub script: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotoriousHuntPreview {
    pub game_day: GameDay,
    pub weekday: u8,
    pub rewards_left: Option<u8>,
    pub decision: HuntDecision,
    pub rounds_rebased: u32,
    pub items: Vec<HuntPreviewItem>,
    pub attempts: Vec<HuntAttempt>,
    pub pending_agent_plan_id: Option<String>,
}

fn rebase(config: &NotoriousHuntConfig, counts: &mut [u32]) -> u32 {
    if !config.r#loop {
        return 0;
    }
    let rounds = config
        .plan_list
        .iter()
        .zip(counts.iter())
        .filter(|(p, _)| p.plan_times > 0)
        .map(|(p, count)| count / p.plan_times)
        .min()
        .unwrap_or(0);
    for (plan, count) in config.plan_list.iter().zip(counts) {
        *count -= rounds * plan.plan_times;
    }
    rounds
}
pub fn preview(
    config: &NotoriousHuntConfig,
    teams: &TeamConfig,
    day: GameDay,
    rewards_left: Option<u8>,
) -> Result<NotoriousHuntPreview, &'static str> {
    config.validate()?;
    teams.validate()?;
    day.validate()?;
    if rewards_left.is_some_and(|n| n > 3) {
        return Err("周常剩余奖励次数须为 0..3，留空表示未知");
    }
    let weekday = chrono::NaiveDate::parse_from_str(&day.date, "%Y-%m-%d")
        .map_err(|_| "游戏日无效")?
        .weekday()
        .number_from_monday() as u8;
    let mut counts: Vec<_> = config.plan_list.iter().map(|p| p.run_times).collect();
    let rounds_rebased = rebase(config, &mut counts);
    let items = config
        .plan_list
        .iter()
        .zip(&counts)
        .map(|(p, count)| {
            Ok(HuntPreviewItem {
                plan_id: p.plan_id.clone(),
                effective_run_times: *count,
                script: p.script(teams)?.into(),
            })
        })
        .collect::<Result<Vec<_>, &'static str>>()?;
    let mut result = NotoriousHuntPreview {
        game_day: day,
        weekday,
        rewards_left,
        decision: HuntDecision::Ready,
        rounds_rebased,
        items,
        attempts: vec![],
        pending_agent_plan_id: None,
    };
    result.decision = if !config.plan_list.iter().any(|p| p.plan_times > 0) {
        HuntDecision::NoPlans
    } else if weekday < config.weekly_challenge_start_weekday {
        HuntDecision::BeforeStartDay
    } else if !config
        .plan_list
        .iter()
        .zip(&counts)
        .any(|(p, n)| *n < p.plan_times)
    {
        HuntDecision::Completed
    } else if rewards_left == Some(0) {
        HuntDecision::Exhausted
    } else if rewards_left.is_none() {
        HuntDecision::UnknownRewards
    } else {
        for _ in 0..rewards_left.unwrap_or(0) {
            rebase(config, &mut counts);
            let Some(index) = config
                .plan_list
                .iter()
                .zip(&counts)
                .position(|(p, n)| *n < p.plan_times)
            else {
                break;
            };
            let plan = &config.plan_list[index];
            if plan.mission_type_name == "代理人方案培养" {
                result.pending_agent_plan_id = Some(plan.plan_id.clone());
                break;
            }
            result.attempts.push(HuntAttempt {
                plan_id: plan.plan_id.clone(),
                script: result.items[index].script.clone(),
            });
            counts[index] += 1;
        }
        if result.pending_agent_plan_id.is_some() {
            HuntDecision::NeedsAgentTarget
        } else {
            HuntDecision::Ready
        }
    };
    Ok(result)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotoriousHuntGroupPreview {
    pub instance_id: u32,
    pub group_id: String,
    pub revision: u32,
    pub forecast: NotoriousHuntPreview,
}
pub fn preview_group(
    config: &crate::config::AccountConfig,
    instance_id: u32,
    group_id: &str,
    revision: u32,
    now: chrono::DateTime<chrono::Utc>,
    rewards_left: Option<u8>,
) -> Result<NotoriousHuntGroupPreview, &'static str> {
    if config.revision != revision {
        return Err("账号配置已变化，请刷新后重新估算");
    }
    let account = config
        .instances
        .iter()
        .find(|a| a.id == instance_id)
        .ok_or("恶名狩猎估算账号不存在")?;
    let group = account
        .groups
        .iter()
        .find(|g| g.id == group_id)
        .ok_or("恶名狩猎估算应用组不存在")?;
    Ok(NotoriousHuntGroupPreview {
        instance_id,
        group_id: group_id.into(),
        revision,
        forecast: preview(
            &group.notorious_hunt,
            &account.predefined_teams,
            GameDay::at(account.game_region, now)?,
            rewards_left,
        )?,
    })
}

/// 只转换设置和次数基线；旧每周剩余次数不构成新运行证明。
pub fn import(value: &serde_json::Value) -> Result<NotoriousHuntConfig, &'static str> {
    use serde_json::{Value, json};
    let object = value.as_object().ok_or("旧恶名狩猎配置必须是对象")?;
    let mut converted =
        serde_json::to_value(NotoriousHuntConfig::default()).map_err(|_| "默认设置无效")?;
    for key in ["loop", "weekly_challenge_start_weekday"] {
        if let Some(v) = object.get(key) {
            converted[key] = v.clone();
        }
    }
    if let Some(list) = object.get("plan_list") {
        let list = list.as_array().ok_or("旧恶名狩猎计划必须是数组")?;
        if list.len() > crate::charge_plan::MAX_PLANS {
            return Err("旧恶名狩猎计划最多 64 项");
        }
        let mut plans = vec![];
        for (index, value) in list.iter().enumerate() {
            let mut v = value
                .as_object()
                .ok_or("旧恶名狩猎计划项必须是对象")?
                .clone();
            if let Some(skipped) = v.remove("skipped")
                && !skipped.is_boolean()
            {
                return Err("旧临时跳过标记须为布尔值");
            }
            if matches!(
                v.get("tab_name").and_then(Value::as_str),
                Some("挑战" | "作战")
            ) {
                v.insert("tab_name".into(), json!("训练"));
            }
            if let Some(card) = v.get("card_num") {
                if card != "默认数量" && !card.is_null() {
                    return Err("周常恶名狩猎不能指定卡片数量");
                }
                v.insert("card_num".into(), Value::Null);
            }
            if v.get("plan_id").is_none_or(Value::is_null) {
                v.remove("plan_id");
                let hash = Sha256::digest(format!(
                    "zzz-auto-notorious-hunt/{index}/{}",
                    Value::Object(v.clone())
                ));
                let mut bytes = [0; 16];
                bytes.copy_from_slice(&hash[..16]);
                v.insert(
                    "plan_id".into(),
                    json!(uuid::Uuid::from_bytes(bytes).to_string()),
                );
            }
            let mut item = serde_json::to_value(default_item()).map_err(|_| "默认计划无效")?;
            item.as_object_mut().ok_or("默认计划无效")?.extend(v);
            plans.push(item);
        }
        converted["plan_list"] = Value::Array(plans);
    }
    let result: NotoriousHuntConfig =
        serde_json::from_value(converted).map_err(|_| "旧恶名狩猎计划字段或类型无效")?;
    result.validate()?;
    Ok(result)
}
