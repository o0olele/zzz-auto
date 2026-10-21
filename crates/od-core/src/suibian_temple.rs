//! 随便观的冻结设置与旧配置转换。德丰大押仅保留旧值，不开放已停用的商店。
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AdventureDuration {
    #[serde(rename = "min_3")]
    Min3,
    #[serde(rename = "min_15")]
    Min15,
    #[serde(rename = "hour_1")]
    Hour1,
    #[serde(rename = "hour_2")]
    Hour2,
    #[serde(rename = "hour_6")]
    Hour6,
    #[serde(rename = "hour_12")]
    Hour12,
    #[default]
    #[serde(rename = "hour_20")]
    Hour20,
}
impl AdventureDuration {
    pub fn label(self) -> &'static str {
        match self {
            Self::Min3 => "3分钟",
            Self::Min15 => "15分钟",
            Self::Hour1 => "1小时",
            Self::Hour2 => "2小时",
            Self::Hour6 => "6小时",
            Self::Hour12 => "12小时",
            Self::Hour20 => "20小时",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BangbooPrice {
    S1,
    S2,
    S3,
    #[default]
    S4,
    None,
}
impl BangbooPrice {
    pub fn minimum(self) -> Option<u32> {
        match self {
            Self::S1 => Some(40000),
            Self::S2 => Some(35000),
            Self::S3 => Some(30000),
            Self::S4 => Some(25000),
            Self::None => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PawnshopOmnicoinGoods {
    HifiMasterCopy,
    SeniorInvestigatorLog,
    WEngineEnergyModule,
    EtherPlatingAgent,
    PrepaidPowerCard,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PawnshopCrestGoods {
    BangbooSystemWidget,
    Denny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct LegacyPawnshopConfig {
    pub omnicoin_enabled: bool,
    pub omnicoin_priority: Vec<PawnshopOmnicoinGoods>,
    pub crest_enabled: bool,
    pub crest_priority: Vec<PawnshopCrestGoods>,
    pub crest_unlimited_denny_enabled: bool,
}
impl Default for LegacyPawnshopConfig {
    fn default() -> Self {
        use PawnshopOmnicoinGoods::*;
        Self {
            omnicoin_enabled: true,
            omnicoin_priority: vec![
                HifiMasterCopy,
                SeniorInvestigatorLog,
                WEngineEnergyModule,
                EtherPlatingAgent,
                PrepaidPowerCard,
            ],
            crest_enabled: true,
            crest_priority: vec![
                PawnshopCrestGoods::BangbooSystemWidget,
                PawnshopCrestGoods::Denny,
            ],
            crest_unlimited_denny_enabled: false,
        }
    }
}
impl LegacyPawnshopConfig {
    fn validate(&self) -> Result<(), &'static str> {
        if self.omnicoin_priority.len() > 5
            || self.crest_priority.len() > 2
            || self
                .omnicoin_priority
                .iter()
                .enumerate()
                .any(|(i, v)| self.omnicoin_priority[..i].contains(v))
            || self
                .crest_priority
                .iter()
                .enumerate()
                .any(|(i, v)| self.crest_priority[..i].contains(v))
        {
            return Err("旧德丰大押优先级不能重复或超过商品数量");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct SuibianTempleConfig {
    pub auto_manage_enabled: bool,
    pub yum_cha_sin: bool,
    pub yum_cha_sin_period_refresh: bool,
    pub adventure_duration: AdventureDuration,
    pub adventure_missions: [String; 4],
    pub craft_drag_times: u8,
    pub good_goods_purchase_enabled: bool,
    pub boo_box_purchase_enabled: bool,
    pub boo_box_adventure_price: BangbooPrice,
    pub boo_box_craft_price: BangbooPrice,
    pub boo_box_sell_price: BangbooPrice,
    pub legacy_pawnshop: LegacyPawnshopConfig,
    pub timeout_ms: u32,
}
impl Default for SuibianTempleConfig {
    fn default() -> Self {
        Self {
            auto_manage_enabled: true,
            yum_cha_sin: true,
            yum_cha_sin_period_refresh: true,
            adventure_duration: AdventureDuration::Hour20,
            adventure_missions: [
                "RESEARCH_3_4",
                "RESEARCH_2_4",
                "RESEARCH_1_4",
                "COMMUNITY_3_4",
            ]
            .map(str::to_owned),
            craft_drag_times: 10,
            good_goods_purchase_enabled: false,
            boo_box_purchase_enabled: false,
            boo_box_adventure_price: BangbooPrice::S4,
            boo_box_craft_price: BangbooPrice::S4,
            boo_box_sell_price: BangbooPrice::S4,
            legacy_pawnshop: Default::default(),
            timeout_ms: 900_000,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdventureMission {
    pub district: &'static str,
    pub area: u8,
    pub stage: u8,
}
impl AdventureMission {
    pub fn parse(id: &str) -> Option<Self> {
        let mut parts = id.split('_');
        let district = match parts.next()? {
            "CRAFT" => "制造区",
            "COMMUNITY" => "社区旧址",
            "RESEARCH" => "科研院旧址",
            _ => return None,
        };
        let area = match parts.next()? {
            "1" => 1,
            "2" => 2,
            "3" => 3,
            _ => return None,
        };
        let stage = match parts.next()? {
            "1" => 1,
            "2" => 2,
            "3" => 3,
            "4" => 4,
            _ => return None,
        };
        parts.next().is_none().then_some(Self {
            district,
            area,
            stage,
        })
    }
    pub fn title(&self) -> String {
        format!("{}{}", self.district, self.area)
    }
}
impl SuibianTempleConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self
            .adventure_missions
            .iter()
            .any(|m| AdventureMission::parse(m).is_none())
        {
            return Err("随便观须保存四个有效游历副本（区域 1..3、子副本 1..4）");
        }
        if self.craft_drag_times > 50 || !(1000..=3_600_000).contains(&self.timeout_ms) {
            return Err("制造下拉次数须为 0..50，总超时须为 1000..3600000ms");
        }
        self.legacy_pawnshop.validate()
    }
    pub fn duration_options() -> Value {
        json!(
            [
                AdventureDuration::Min3,
                AdventureDuration::Min15,
                AdventureDuration::Hour1,
                AdventureDuration::Hour2,
                AdventureDuration::Hour6,
                AdventureDuration::Hour12,
                AdventureDuration::Hour20
            ]
            .map(|value| json!({"value":value,"label":value.label()}))
        )
    }
    pub fn mission_options() -> Value {
        let mut options = Vec::new();
        for region in ["CRAFT", "COMMUNITY", "RESEARCH"] {
            for area in 1..=3 {
                for stage in 1..=4 {
                    let id = format!("{region}_{area}_{stage}");
                    let mission = AdventureMission::parse(&id).expect("固定副本有效");
                    options.push(
                        json!({"value":id,"label":format!("{}-{}",mission.title(),mission.stage)}),
                    );
                }
            }
        }
        json!(options)
    }
    pub fn price_options() -> Value {
        json!([BangbooPrice::S1,BangbooPrice::S2,BangbooPrice::S3,BangbooPrice::S4,BangbooPrice::None]
            .map(|value| json!({"value":value,"label":value.minimum().map_or("不购买".into(),|v| format!("{v} 及以上"))})))
    }
}

pub fn import(value: &Value) -> Result<SuibianTempleConfig, &'static str> {
    let object = value.as_object().ok_or("旧随便观配置必须为对象")?;
    let mut converted =
        serde_json::to_value(SuibianTempleConfig::default()).map_err(|_| "默认随便观配置无效")?;
    for key in [
        "auto_manage_enabled",
        "yum_cha_sin",
        "yum_cha_sin_period_refresh",
        "craft_drag_times",
        "good_goods_purchase_enabled",
        "boo_box_purchase_enabled",
    ] {
        if let Some(v) = object.get(key) {
            converted[key] = v.clone();
        }
    }
    for key in [
        "adventure_duration",
        "boo_box_adventure_price",
        "boo_box_craft_price",
        "boo_box_sell_price",
    ] {
        if let Some(v) = object.get(key) {
            let text = v.as_str().ok_or("旧随便观时长或价格不是枚举名称")?;
            // 旧 Python 保存的是 StrEnum 的 name（HOUR_20、S4、NONE）。
            if text != text.to_ascii_uppercase() {
                return Err("旧随便观时长或价格枚举名称无效");
            }
            converted[key] = json!(text.to_ascii_lowercase());
        }
    }
    for index in 0..4 {
        if let Some(v) = object.get(&format!("adventure_mission_{}", index + 1)) {
            converted["adventure_missions"][index] = v.clone();
        }
    }
    for (old, new) in [
        ("pawnshop_omnicoin_enabled", "omnicoin_enabled"),
        ("pawnshop_crest_enabled", "crest_enabled"),
        (
            "pawnshop_crest_unlimited_denny_enabled",
            "crest_unlimited_denny_enabled",
        ),
    ] {
        if let Some(v) = object.get(old) {
            converted["legacy_pawnshop"][new] = v.clone();
        }
    }
    for (old, new) in [
        ("pawnshop_omnicoin_priority", "omnicoin_priority"),
        ("pawnshop_crest_priority", "crest_priority"),
    ] {
        if let Some(v) = object.get(old) {
            let values = v.as_array().ok_or("旧德丰大押优先级必须为列表")?;
            let mut names = Vec::new();
            for v in values {
                let text = v.as_str().ok_or("旧德丰大押优先级必须为名称")?;
                if text != text.to_ascii_uppercase() {
                    return Err("旧德丰大押商品枚举名称无效");
                }
                names.push(text.to_ascii_lowercase());
            }
            converted["legacy_pawnshop"][new] = json!(names);
        }
    }
    let config: SuibianTempleConfig =
        serde_json::from_value(converted).map_err(|_| "旧随便观字段类型或枚举无效")?;
    config.validate()?;
    Ok(config)
}
