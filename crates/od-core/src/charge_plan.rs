//! 体力计划配置与无输入估算；旧次数不是新版每日成功证明。
use crate::{daily::GameDay, predefined_teams::TeamConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::OnceLock};
use ts_rs::TS;

pub const MAX_PLANS: usize = 64;
// 仅在执行与展示时解析；保存值参与历史前缀指纹，不能在反序列化时改写。
pub const MISSION_TYPE_ALIASES: [(&str, &str, &str); 1] =
    [("区域巡防", "迅雷与裂爪", "迅脊与裂爪")];
pub fn execution_mission_type<'a>(category: &str, name: &'a str) -> &'a str {
    MISSION_TYPE_ALIASES
        .iter()
        .find(|(c, old, _)| *c == category && *old == name)
        .map_or(name, |(_, _, current)| *current)
}
pub const LEVELS: [&str; 6] = [
    "默认等级",
    "等级Lv.65",
    "等级Lv.60",
    "等级Lv.50",
    "等级Lv.40",
    "等级Lv.30",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ChargeMission {
    pub mission_type_name: String,
    #[serde(default)]
    pub mission_type_name_display: Option<String>,
    #[serde(default)]
    pub mission_list: Vec<ChargeMissionName>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ChargeMissionName {
    pub mission_name: String,
    #[serde(default)]
    pub mission_name_display: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ChargeCategory {
    pub category_name: String,
    #[serde(default)]
    pub mission_type_list: Vec<ChargeMission>,
}
pub fn catalog() -> &'static Vec<ChargeCategory> {
    static CATALOG: OnceLock<Vec<ChargeCategory>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        #[derive(Deserialize)]
        struct Tab {
            tab_name: String,
            #[serde(default)]
            category_list: Vec<ChargeCategory>,
        }
        serde_yaml::from_str::<Vec<Tab>>(include_str!(
            "../../../resources/charge-plan/compendium.yml"
        ))
        .expect("固定快捷手册目录有效")
        .into_iter()
        .find(|t| t.tab_name == "训练")
        .expect("包含训练目录")
        .category_list
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RestoreCharge {
    #[default]
    None,
    BackupOnly,
    EtherOnly,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct ChargePlanItem {
    pub plan_id: String,
    pub tab_name: String,
    pub category_name: String,
    pub mission_type_name: String,
    pub mission_name: Option<String>,
    pub level: String,
    pub auto_battle_config: String,
    pub run_times: u32,
    pub plan_times: u32,
    pub card_num: Option<u8>,
    pub predefined_team_idx: i8,
    pub notorious_hunt_buff_num: u8,
}
impl Default for ChargePlanItem {
    fn default() -> Self {
        Self {
            plan_id: uuid::Uuid::nil().to_string(),
            tab_name: "训练".into(),
            category_name: "实战模拟室".into(),
            mission_type_name: "基础材料".into(),
            mission_name: Some("调查专项".into()),
            level: "默认等级".into(),
            auto_battle_config: "全配队通用".into(),
            run_times: 0,
            plan_times: 1,
            card_num: None,
            predefined_team_idx: -1,
            notorious_hunt_buff_num: 1,
        }
    }
}
impl ChargePlanItem {
    pub fn execution_mission_type(&self) -> &str {
        execution_mission_type(&self.category_name, &self.mission_type_name)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if uuid::Uuid::parse_str(&self.plan_id).is_err()
            || self.tab_name != "训练"
            || self.run_times > 1_000_000
            || self.plan_times > 1000
            || self.card_num.is_some_and(|n| !(1..=5).contains(&n))
            || !(-1..20).contains(&self.predefined_team_idx)
            || !(1..=3).contains(&self.notorious_hunt_buff_num)
            || !LEVELS.contains(&self.level.as_str())
        {
            return Err("体力计划 ID、次数、卡片、编队、等级或鸣徽无效");
        }
        crate::intel_board::IntelBoardConfig {
            predefined_team_idx: self.predefined_team_idx,
            auto_battle_config: self.auto_battle_config.clone(),
            exp_grind_mode: false,
        }
        .validate()
        .map_err(|_| "体力计划战斗脚本名称无效")?;
        let category = catalog()
            .iter()
            .find(|c| c.category_name == self.category_name)
            .ok_or("体力计划副本分类不在固定目录中")?;
        if category.mission_type_list.is_empty() {
            if !self.mission_type_name.is_empty() || self.mission_name.is_some() {
                return Err("合成电池不能指定副本类型或关卡");
            }
        } else {
            let mission = category
                .mission_type_list
                .iter()
                .find(|m| m.mission_type_name == self.execution_mission_type())
                .ok_or("体力计划副本类型不在所属分类中")?;
            if mission.mission_list.is_empty() {
                if self.mission_name.is_some() {
                    return Err("此体力副本没有下级关卡，须保留空值");
                }
            } else if !mission
                .mission_list
                .iter()
                .any(|m| Some(&m.mission_name) == self.mission_name.as_ref())
            {
                return Err("体力计划关卡不在所属副本类型中");
            }
        }
        Ok(())
    }
    pub fn estimated_charge(&self) -> u32 {
        match self.category_name.as_str() {
            "实战模拟室" => u32::from(self.card_num.unwrap_or(1)) * 20,
            "专业挑战室" => 40,
            _ => 60,
        }
    }
    pub fn script<'a>(&'a self, teams: &'a TeamConfig) -> Result<&'a str, &'static str> {
        self.validate()?;
        if self.predefined_team_idx < 0 {
            Ok(&self.auto_battle_config)
        } else {
            Ok(&teams.get(self.predefined_team_idx as u8)?.auto_battle)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct ChargePlanConfig {
    pub plan_list: Vec<ChargePlanItem>,
    pub r#loop: bool,
    pub daily_reset_plan_times: bool,
    pub last_daily_reset_day: Option<GameDay>,
    pub skip_plan: bool,
    pub double_reward: bool,
    pub combat_simulation_double_reward_config: ChargePlanItem,
    pub restore_charge: RestoreCharge,
}
impl Default for ChargePlanConfig {
    fn default() -> Self {
        Self {
            plan_list: vec![],
            r#loop: true,
            daily_reset_plan_times: false,
            last_daily_reset_day: None,
            skip_plan: false,
            double_reward: false,
            combat_simulation_double_reward_config: Default::default(),
            restore_charge: RestoreCharge::None,
        }
    }
}
impl ChargePlanConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.plan_list.len() > MAX_PLANS {
            return Err("每组体力计划最多 64 项");
        }
        let mut ids = BTreeSet::new();
        for item in &self.plan_list {
            item.validate()?;
            let id = uuid::Uuid::parse_str(&item.plan_id).map_err(|_| "体力计划 ID 无效")?;
            if !ids.insert(id) {
                return Err("体力计划 ID 不能重复");
            }
        }
        self.combat_simulation_double_reward_config.validate()?;
        if self.combat_simulation_double_reward_config.category_name != "实战模拟室" {
            return Err("双倍临时计划须为实战模拟室");
        }
        if let Some(day) = &self.last_daily_reset_day {
            day.validate()?;
        }
        Ok(())
    }
}

/// 仅供人工输入的估算及后续识别器使用；缺少任一字段时整体资源未知。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChargeResources {
    pub charge: u32,
    pub backup: u32,
    pub ether: u32,
}
impl ChargeResources {
    pub fn validate(self) -> Result<(), &'static str> {
        if [self.charge, self.backup, self.ether]
            .into_iter()
            .any(|n| n > 1_000_000)
        {
            return Err("电量估算字段不能超过 1000000");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChargeDecision {
    Disabled,
    Completed,
    Ready,
    NeedsRestore,
    Insufficient,
    UnknownResources,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ChargePreviewItem {
    pub plan_id: String,
    pub effective_run_times: u32,
    pub estimated_charge: u32,
    pub script: String,
    pub decision: ChargeDecision,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ChargePlanPreview {
    pub config: ChargePlanConfig,
    pub game_day: GameDay,
    pub reset_daily: bool,
    pub rounds_rebased: u32,
    pub items: Vec<ChargePreviewItem>,
    pub next_plan_id: Option<String>,
    pub temporary: Option<ChargePlanItem>,
    pub temporary_script: Option<String>,
    pub double_check_pending: bool,
}
pub fn preview(
    config: &ChargePlanConfig,
    teams: &TeamConfig,
    day: GameDay,
    resources: Option<ChargeResources>,
    double_left: Option<u8>,
) -> Result<ChargePlanPreview, &'static str> {
    config.validate()?;
    teams.validate()?;
    day.validate()?;
    if let Some(resources) = resources {
        resources.validate()?;
    }
    if double_left.is_some_and(|n| n > 5) {
        return Err("实战模拟室双倍剩余次数须为 0..5");
    }
    let reset_daily =
        config.daily_reset_plan_times && config.last_daily_reset_day.as_ref() != Some(&day);
    let counts: Vec<_> = config
        .plan_list
        .iter()
        .map(|p| if reset_daily { 0 } else { p.run_times })
        .collect();
    let rounds_rebased = if config.r#loop {
        config
            .plan_list
            .iter()
            .zip(&counts)
            .filter(|(p, _)| p.plan_times > 0)
            .map(|(p, count)| count / p.plan_times)
            .min()
            .unwrap_or(0)
    } else {
        0
    };
    let mut items = Vec::new();
    let mut next = None;
    let mut blocked = false;
    for (p, count) in config.plan_list.iter().zip(counts) {
        let effective = count - rounds_rebased * p.plan_times;
        let cost = p.estimated_charge();
        let decision = if p.plan_times == 0 {
            ChargeDecision::Disabled
        } else if effective >= p.plan_times {
            ChargeDecision::Completed
        } else if let Some(r) = resources {
            if r.charge >= cost {
                ChargeDecision::Ready
            } else {
                let need = cost - r.charge;
                let restored = (matches!(
                    config.restore_charge,
                    RestoreCharge::BackupOnly | RestoreCharge::Both
                ) && r.backup >= need)
                    || (matches!(
                        config.restore_charge,
                        RestoreCharge::EtherOnly | RestoreCharge::Both
                    ) && r.ether * 60 >= need);
                if restored {
                    ChargeDecision::NeedsRestore
                } else {
                    ChargeDecision::Insufficient
                }
            }
        } else {
            ChargeDecision::UnknownResources
        };
        if next.is_none() && !blocked {
            match decision {
                ChargeDecision::Ready | ChargeDecision::NeedsRestore => {
                    next = Some(p.plan_id.clone())
                }
                ChargeDecision::UnknownResources => blocked = true,
                ChargeDecision::Insufficient if !config.skip_plan => blocked = true,
                _ => {}
            }
        }
        items.push(ChargePreviewItem {
            plan_id: p.plan_id.clone(),
            effective_run_times: effective,
            estimated_charge: cost,
            script: p.script(teams)?.into(),
            decision,
        });
    }
    let temporary = if config.double_reward {
        resources.zip(double_left).and_then(|(r, left)| {
            let cards = (r.charge / 20).min(u32::from(left)) as u8;
            (cards > 0).then(|| {
                let mut p = config.combat_simulation_double_reward_config.clone();
                p.run_times = 0;
                p.plan_times = 1;
                p.card_num = Some(cards);
                p
            })
        })
    } else {
        None
    };
    let temporary_script = temporary
        .as_ref()
        .map(|p| p.script(teams))
        .transpose()?
        .map(str::to_owned);
    Ok(ChargePlanPreview {
        config: config.clone(),
        game_day: day,
        reset_daily,
        rounds_rebased,
        items,
        next_plan_id: next,
        temporary,
        temporary_script,
        double_check_pending: config.double_reward && double_left.is_none(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ChargeGroupPreview {
    pub instance_id: u32,
    pub group_id: String,
    pub revision: u32,
    pub forecast: ChargePlanPreview,
}
pub fn preview_group(
    config: &crate::config::AccountConfig,
    instance_id: u32,
    group_id: &str,
    revision: u32,
    now: chrono::DateTime<chrono::Utc>,
    resources: Option<ChargeResources>,
    double_left: Option<u8>,
) -> Result<ChargeGroupPreview, &'static str> {
    if config.revision != revision {
        return Err("账号配置已变化，请刷新后重新估算");
    }
    let instance = config
        .instances
        .iter()
        .find(|a| a.id == instance_id)
        .ok_or("体力估算账号不存在")?;
    let group = instance
        .groups
        .iter()
        .find(|g| g.id == group_id)
        .ok_or("体力估算应用组不存在")?;
    let day = GameDay::at(instance.game_region, now)?;
    Ok(ChargeGroupPreview {
        instance_id,
        group_id: group_id.into(),
        revision,
        forecast: preview(
            &group.charge_plan,
            &instance.predefined_teams,
            day,
            resources,
            double_left,
        )?,
    })
}

pub fn import(
    value: &serde_json::Value,
    region: crate::daily::GameRegion,
) -> Result<ChargePlanConfig, &'static str> {
    use serde_json::{Value, json};
    let object = value.as_object().ok_or("旧体力计划必须是对象")?;
    let mut converted =
        serde_json::to_value(ChargePlanConfig::default()).map_err(|_| "体力默认设置无效")?;
    for key in [
        "loop",
        "daily_reset_plan_times",
        "skip_plan",
        "double_reward",
    ] {
        if let Some(v) = object.get(key) {
            converted[key] = v.clone();
        }
    }
    if let Some(v) = object.get("restore_charge") {
        converted["restore_charge"] = json!(match v.as_str() {
            Some("不使用") => RestoreCharge::None,
            Some("使用储蓄电量") => RestoreCharge::BackupOnly,
            Some("使用以太电池") => RestoreCharge::EtherOnly,
            Some("同时使用储蓄电量和以太电池") => RestoreCharge::Both,
            _ => return Err("旧恢复电量方式无效"),
        });
    }
    if let Some(date) = object.get("last_daily_reset_dt") {
        let date = date.as_str().ok_or("旧体力每日重置日期不是文本")?;
        if !date.is_empty() {
            // 旧 AppRunRecord / os_utils.get_dt 保存 YYYYMMDD，新版 GameDay 使用 ISO 日期。
            let parsed = chrono::NaiveDate::parse_from_str(date, "%Y%m%d")
                .map_err(|_| "旧体力每日重置日期无效")?;
            if parsed.format("%Y%m%d").to_string() != date {
                return Err("旧体力每日重置日期须为 YYYYMMDD");
            }
            converted["last_daily_reset_day"] = json!(GameDay {
                region,
                date: parsed.format("%Y-%m-%d").to_string()
            });
        }
    }
    fn item(value: &Value, index: usize) -> Result<Value, &'static str> {
        let mut v = value.as_object().ok_or("旧体力计划项必须是对象")?.clone();
        if let Some(skipped) = v.remove("skipped")
            && !skipped.is_boolean()
        {
            return Err("旧体力临时跳过标记不是布尔值");
        }
        if let Some(card) = v.get("card_num") {
            v.insert(
                "card_num".into(),
                match card.as_str() {
                    Some("默认数量") => Value::Null,
                    Some("1" | "2" | "3" | "4" | "5") => {
                        json!(card.as_str().unwrap().parse::<u8>().unwrap())
                    }
                    _ => return Err("旧体力卡片数量无效"),
                },
            );
        }
        if v.get("plan_id").is_none_or(Value::is_null) {
            v.remove("plan_id");
            let hash = Sha256::digest(format!(
                "zzz-auto-charge-plan/{index}/{}",
                Value::Object(v.clone())
            ));
            let mut bytes = [0; 16];
            bytes.copy_from_slice(&hash[..16]);
            v.insert(
                "plan_id".into(),
                json!(uuid::Uuid::from_bytes(bytes).to_string()),
            );
        }
        Ok(Value::Object(v))
    }
    if let Some(list) = object.get("plan_list") {
        let list = list.as_array().ok_or("旧体力计划列表不是数组")?;
        if list.len() > MAX_PLANS {
            return Err("旧体力计划最多 64 项");
        }
        converted["plan_list"] = Value::Array(
            list.iter()
                .enumerate()
                .map(|(i, v)| item(v, i))
                .collect::<Result<_, _>>()?,
        );
    }
    if let Some(v) = object.get("combat_simulation_double_reward_config") {
        converted["combat_simulation_double_reward_config"] = item(v, MAX_PLANS)?;
    }
    let config: ChargePlanConfig =
        serde_json::from_value(converted).map_err(|_| "旧体力计划字段或类型无效")?;
    config.validate()?;
    Ok(config)
}
