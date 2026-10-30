//! 迷失之地配置与只读藏品决策。结果不构成点击或完成证明。
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LostVoidTask {
    #[default]
    BountyCommission,
    EvalPoint,
    PeriodReward,
    WeeklyPlanTimes,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LostVoidMission {
    #[default]
    Purge,
    TaskForce,
    MatrixAction,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct LostVoidConfig {
    pub daily_plan_times: u16,
    pub weekly_plan_times: u16,
    pub extra_task: LostVoidTask,
    pub mission_name: LostVoidMission,
    pub challenge_config: String,
    pub challenge: LostVoidChallenge,
}
impl Default for LostVoidConfig {
    fn default() -> Self {
        Self {
            daily_plan_times: 5,
            weekly_plan_times: 2,
            extra_task: LostVoidTask::BountyCommission,
            mission_name: LostVoidMission::Purge,
            challenge_config: "默认-成就模式".into(),
            challenge: LostVoidChallenge::default(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct LostVoidChallenge {
    pub predefined_team_idx: i8,
    pub choose_team_by_priority: bool,
    pub manually_choose_agent: bool,
    pub team_info: Vec<String>,
    pub auto_battle: String,
    pub artifact_priority_new: bool,
    pub artifact_priority: Vec<String>,
    pub artifact_priority_2: Vec<String>,
    pub region_type_priority: Vec<String>,
    /// 1..=3，旧配置的“第一个 / 第二个 / 第三个”。
    pub period_buff_no: u8,
    /// 0..=4 或 999；保留旧版“一直刷新”的显式值。
    pub buy_only_priority_2: u16,
    pub store_gold: bool,
    pub store_blood: bool,
    pub store_blood_min: u8,
    pub investigation_strategy: String,
    pub chase_new_mode: bool,
}
impl Default for LostVoidChallenge {
    fn default() -> Self {
        // 对齐默认-成就模式.sample.yml，而非参数演示文件。
        Self {
            predefined_team_idx: -1,
            choose_team_by_priority: false,
            manually_choose_agent: false,
            team_info: vec!["unknown".into(); 3],
            auto_battle: "全配队通用".into(),
            artifact_priority_new: true,
            artifact_priority: vec!["无详情 获得鸣徽".into()],
            artifact_priority_2: vec!["终结".into(), "绝境".into(), "通用".into()],
            region_type_priority: [
                "战斗-武备",
                "战斗-鸣徽",
                "挑战-无伤",
                "邦布商店",
                "挚交会谈",
                "战斗-道中危机",
                "战斗-终结之役",
                "战斗-硬币",
                "挑战-限时",
            ]
            .map(String::from)
            .to_vec(),
            period_buff_no: 1,
            buy_only_priority_2: 3,
            store_gold: true,
            store_blood: true,
            store_blood_min: 50,
            investigation_strategy: "攻其不备战略".into(),
            chase_new_mode: true,
        }
    }
}
pub const REGIONS: [&str; 14] = [
    "入口",
    "战斗-鸣徽",
    "战斗-武备",
    "战斗-硬币",
    "挑战-无伤",
    "挑战-限时",
    "挑战-收割",
    "偶遇事件",
    "代价之间",
    "休憩调息",
    "邦布商店",
    "挚交会谈",
    "战斗-道中危机",
    "战斗-终结之役",
];
fn text_valid(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= 240
        && !value.chars().any(char::is_control)
}
fn name_valid(value: &str) -> bool {
    text_valid(value) && !value.contains(['/', '\\', ':']) && !matches!(value, "." | "..")
}
fn rules_valid(rules: &[String]) -> bool {
    rules.len() <= 128 && rules.iter().all(|rule| text_valid(rule))
}
impl LostVoidConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.daily_plan_times > 999
            || self.weekly_plan_times > 999
            || !name_valid(&self.challenge_config)
        {
            return Err("迷失之地次数须为 0..999，挑战方案名称须为有效名称");
        }
        self.challenge.validate()
    }
}
impl LostVoidChallenge {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(-1..20).contains(&self.predefined_team_idx)
            || !name_valid(&self.auto_battle)
            || self.team_info.len() != 3
            || !self.team_info.iter().all(|s| name_valid(s))
        {
            return Err("迷失之地编队、代理人或战斗脚本无效");
        }
        if !(1..=3).contains(&self.period_buff_no)
            || !(self.buy_only_priority_2 <= 4 || self.buy_only_priority_2 == 999)
            || self.store_blood_min > 100
        {
            return Err("迷失之地周期增益、刷新次数或最低血量无效");
        }
        if !rules_valid(&self.artifact_priority)
            || !rules_valid(&self.artifact_priority_2)
            || !rules_valid(&self.region_type_priority)
            || self
                .region_type_priority
                .iter()
                .any(|s| !REGIONS.contains(&s.as_str()))
            || self
                .region_type_priority
                .iter()
                .collect::<HashSet<_>>()
                .len()
                != self.region_type_priority.len()
            || !strategies()?.contains(&self.investigation_strategy)
        {
            return Err("迷失之地藏品、区域优先级或调查战略无效");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LostVoidArtifact {
    pub category: String,
    pub name: String,
    pub level: String,
    #[serde(default)]
    pub is_gear: bool,
    #[serde(default)]
    pub template_id: Option<String>,
}
pub fn artifacts() -> Result<Vec<LostVoidArtifact>, &'static str> {
    serde_yaml::from_str(include_str!("../../../resources/lost-void/artifacts.yml"))
        .map_err(|_| "迷失之地藏品目录无效")
}
pub fn strategies() -> Result<Vec<String>, &'static str> {
    #[derive(Deserialize)]
    struct Strategy {
        strategy_name: String,
    }
    let list: Vec<Strategy> =
        serde_yaml::from_str(include_str!("../../../resources/lost-void/strategies.yml"))
            .map_err(|_| "迷失之地调查战略目录无效")?;
    Ok(list.into_iter().map(|s| s.strategy_name).collect())
}

/// 旧应用设置与被引用的挑战方案分别转换；不采用旧运行记录。
pub fn import(
    value: &serde_json::Value,
    challenge: Option<&serde_json::Value>,
) -> Result<LostVoidConfig, &'static str> {
    let object = value.as_object().ok_or("旧迷失之地配置必须是对象")?;
    let mut converted = serde_json::Map::new();
    for key in ["daily_plan_times", "weekly_plan_times", "challenge_config"] {
        if let Some(v) = object.get(key) {
            converted.insert(key.into(), v.clone());
        }
    }
    if let Some(v) = object.get("mission_name") {
        converted.insert(
            "mission_name".into(),
            serde_json::to_value(match v.as_str() {
                Some("战线肃清") => LostVoidMission::Purge,
                Some("特遣调查") => LostVoidMission::TaskForce,
                Some("矩阵行动") => LostVoidMission::MatrixAction,
                _ => return Err("旧迷失之地副本名称无效"),
            })
            .unwrap(),
        );
    }
    if let Some(v) = object.get("extra_task") {
        converted.insert(
            "extra_task".into(),
            serde_json::to_value(match v.as_str() {
                Some("完成悬赏委托") => LostVoidTask::BountyCommission,
                Some("刷满业绩点") => LostVoidTask::EvalPoint,
                Some("刷满周期奖励") => LostVoidTask::PeriodReward,
                Some("完成周计划次数") => LostVoidTask::WeeklyPlanTimes,
                _ => return Err("旧迷失之地周目标无效"),
            })
            .unwrap(),
        );
    }
    // 旧方案缺字段时采用旧类的属性默认值，与新版默认预设分开。
    if let Some(value) = challenge {
        let mut old = value
            .as_object()
            .ok_or("旧迷失之地挑战方案必须是对象")?
            .clone();
        if let Some(v) = old.get("period_buff_no") {
            old.insert(
                "period_buff_no".into(),
                serde_json::json!(match v.as_str() {
                    Some("第一个") => 1,
                    Some("第二个") => 2,
                    Some("第三个") => 3,
                    _ => return Err("旧迷失之地周期增益无效"),
                }),
            );
        }
        let mut defaults = serde_json::to_value(LostVoidChallenge::default()).unwrap();
        defaults["artifact_priority_new"] = false.into();
        defaults["artifact_priority"] = serde_json::json!([]);
        defaults["artifact_priority_2"] = serde_json::json!([]);
        defaults["region_type_priority"] = serde_json::json!([]);
        defaults["investigation_strategy"] = "鸣徽狂热战略".into();
        defaults["chase_new_mode"] = false.into();
        defaults["store_blood"] = false.into();
        defaults.as_object_mut().unwrap().extend(old);
        converted.insert("challenge".into(), defaults);
    }
    let config: LostVoidConfig = serde_json::from_value(converted.into())
        .map_err(|_| "旧迷失之地字段类型或挑战方案字段无效")?;
    config.validate()?;
    Ok(config)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactCandidate {
    pub slot: u8,
    pub category: String,
    pub name: String,
    pub level: String,
    pub is_new: bool,
    pub is_primary: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArtifactSelection {
    pub dynamic_priority: Vec<String>,
    pub battle_priority: Vec<String>,
    pub abandon: Vec<String>,
    pub ignored_slots: Vec<u8>,
    pub priority_1: bool,
    pub priority_2: bool,
    pub fallback: bool,
    pub new_first: bool,
}
impl Default for ArtifactSelection {
    fn default() -> Self {
        Self {
            dynamic_priority: vec![],
            battle_priority: vec![],
            abandon: vec![],
            ignored_slots: vec![],
            priority_1: true,
            priority_2: true,
            fallback: true,
            new_first: true,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactChoice {
    pub slot: u8,
    pub reason: String,
}
fn normalize(value: &str) -> String {
    let value: String = value
        .chars()
        .filter(|c| !c.is_whitespace() && !['·', ':', '：', '[', ']', '【', '】'].contains(c))
        .collect();
    if value == "击破" {
        "异常击破".into()
    } else {
        value
    }
}
fn category_matches(category: &str, rule: &str) -> bool {
    normalize(category) == normalize(rule)
}
fn matches(candidate: &ArtifactCandidate, rule: &str) -> bool {
    if let Some((category, item)) = rule.split_once(' ') {
        category_matches(&candidate.category, category)
            && (candidate.name == item.trim()
                || (matches!(item.trim(), "S" | "A" | "B") && candidate.level == item.trim()))
    } else {
        category_matches(&candidate.category, rule) || candidate.name == rule
    }
}
pub fn choose_artifacts(
    config: &LostVoidChallenge,
    candidates: &[ArtifactCandidate],
    count: u8,
    options: &ArtifactSelection,
) -> Result<Vec<ArtifactChoice>, &'static str> {
    config.validate()?;
    if candidates.len() > 32
        || count > 32
        || !rules_valid(&options.dynamic_priority)
        || !rules_valid(&options.battle_priority)
        || !rules_valid(&options.abandon)
        || options.ignored_slots.len() > 32
    {
        return Err("迷失之地候选数量或动态规则无效");
    }
    let catalog = artifacts()?;
    let mut slots = HashSet::new();
    for c in candidates {
        if c.slot >= 32
            || !slots.insert(c.slot)
            || !catalog
                .iter()
                .any(|a| a.category == c.category && a.name == c.name && a.level == c.level)
        {
            return Err("迷失之地候选槽位重复或藏品不在目录中");
        }
    }
    let fallback = options.fallback
        || [
            &options.dynamic_priority,
            &config.artifact_priority,
            &config.artifact_priority_2,
            &options.battle_priority,
        ]
        .iter()
        .all(|r| r.is_empty());
    let mut ordered: Vec<_> = candidates
        .iter()
        .filter(|c| !options.ignored_slots.contains(&c.slot))
        .collect();
    ordered.sort_by_key(|c| c.slot);
    let mut result = Vec::<ArtifactChoice>::new();
    for primary in [true, false] {
        let group: Vec<_> = ordered
            .iter()
            .copied()
            .filter(|c| c.is_primary == primary)
            .collect();
        let mut push = |c: &ArtifactCandidate, reason: String| {
            if !result.iter().any(|r| r.slot == c.slot) {
                result.push(ArtifactChoice {
                    slot: c.slot,
                    reason,
                });
            }
        };
        if options.new_first && config.artifact_priority_new {
            let mut new: Vec<_> = group.iter().copied().filter(|c| c.is_new).collect();
            new.sort_by_key(|c| {
                (
                    match c.level.as_str() {
                        "S" => 0,
                        "A" => 1,
                        "B" => 2,
                        _ => 3,
                    },
                    c.slot,
                )
            });
            for c in new {
                push(c, "NEW 优先".into());
            }
        }
        for (enabled, rules, label) in [
            (options.priority_1, &options.dynamic_priority, "动态"),
            (options.priority_1, &config.artifact_priority, "配置一"),
            (options.priority_2, &config.artifact_priority_2, "配置二"),
            (options.priority_2, &options.battle_priority, "战斗追加"),
        ] {
            if enabled {
                for rule in rules {
                    for c in &group {
                        if matches(c, rule) {
                            push(c, format!("{label}：{rule}"));
                        }
                    }
                }
            }
        }
        if fallback {
            for abandoned in [false, true] {
                for c in &group {
                    if options
                        .abandon
                        .iter()
                        .any(|r| category_matches(&c.category, r))
                        == abandoned
                    {
                        push(
                            c,
                            if abandoned {
                                "放弃组补位"
                            } else {
                                "普通补位"
                            }
                            .into(),
                        );
                    }
                }
            }
        }
    }
    result.truncate(usize::from(count));
    Ok(result)
}

/// 只用于只读计划。旧记录不能直接形成新版执行完成证明。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LostVoidHistory {
    pub day: Option<crate::daily::GameDay>,
    pub daily_run_times: u16,
    pub weekly_run_times: u16,
    pub bounty_commission_complete: bool,
    pub eval_point_complete: bool,
    pub period_reward_complete: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LostVoidDecision {
    Run,
    DailyLimit,
    WeeklyComplete,
}
pub fn plan(
    config: &LostVoidConfig,
    day: &crate::daily::GameDay,
    history: &LostVoidHistory,
) -> Result<LostVoidDecision, &'static str> {
    use chrono::{Datelike, NaiveDate};
    config.validate()?;
    day.validate()?;
    let date =
        NaiveDate::parse_from_str(&day.date, "%Y-%m-%d").map_err(|_| "迷失之地游戏日无效")?;
    let same_week = if let Some(old) = &history.day {
        old.validate()?;
        let previous =
            NaiveDate::parse_from_str(&old.date, "%Y-%m-%d").map_err(|_| "迷失之地记录日期无效")?;
        if previous > date {
            return Err("迷失之地记录日期晚于当前游戏日");
        }
        old.region == day.region && previous.iso_week() == date.iso_week()
    } else {
        false
    };
    let weekly = same_week
        && match config.extra_task {
            LostVoidTask::BountyCommission => history.bounty_commission_complete,
            LostVoidTask::EvalPoint => history.eval_point_complete,
            LostVoidTask::PeriodReward => history.period_reward_complete,
            LostVoidTask::WeeklyPlanTimes => history.weekly_run_times >= config.weekly_plan_times,
        };
    // 0 次计划即已达到目标，即使当前没有历史记录。
    if weekly
        || (config.extra_task == LostVoidTask::WeeklyPlanTimes && config.weekly_plan_times == 0)
    {
        Ok(LostVoidDecision::WeeklyComplete)
    } else if (history.day.as_ref() == Some(day)
        && history.daily_run_times >= config.daily_plan_times)
        || config.daily_plan_times == 0
    {
        Ok(LostVoidDecision::DailyLimit)
    } else {
        Ok(LostVoidDecision::Run)
    }
}
