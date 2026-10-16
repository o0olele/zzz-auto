use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GameRegion {
    #[default]
    Cn,
    CnB,
    Us,
    Eu,
    Asia,
    Twhkmo,
}

impl GameRegion {
    pub fn date_offset_hours(self) -> i64 {
        match self {
            Self::Cn | Self::CnB | Self::Asia | Self::Twhkmo => 4,
            Self::Us => -9,
            Self::Eu => -3,
        }
    }

    pub fn options() -> serde_json::Value {
        serde_json::json!([
            {"value": Self::Cn, "label": "国服"}, {"value": Self::CnB, "label": "B服"},
            {"value": Self::Us, "label": "美服"}, {"value": Self::Eu, "label": "欧服"},
            {"value": Self::Asia, "label": "亚服"}, {"value": Self::Twhkmo, "label": "港澳台服"}
        ])
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RunPolicy {
    #[default]
    Always,
    SkipDailySuccess,
}

impl RunPolicy {
    pub fn options() -> serde_json::Value {
        serde_json::json!([
            {"value": Self::Always, "label": "每次运行"},
            {"value": Self::SkipDailySuccess, "label": "跳过本游戏日已完成"}
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GameDay {
    pub region: GameRegion,
    pub date: String,
}

impl GameDay {
    pub fn at(region: GameRegion, now: DateTime<Utc>) -> Result<Self, &'static str> {
        let shifted = now
            .checked_add_signed(Duration::hours(region.date_offset_hours()))
            .ok_or("游戏日超出日期范围")?;
        Ok(Self {
            region,
            date: shifted.format("%Y-%m-%d").to_string(),
        })
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let parsed =
            NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").map_err(|_| "游戏日日期无效")?;
        if parsed.format("%Y-%m-%d").to_string() != self.date {
            return Err("游戏日日期格式无效");
        }
        Ok(())
    }

    pub fn contains(&self, time: DateTime<Utc>) -> bool {
        Self::at(self.region, time).is_ok_and(|day| day == *self)
    }

    pub fn ends_at(&self) -> Result<DateTime<Utc>, &'static str> {
        let date =
            NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").map_err(|_| "游戏日日期无效")?;
        date.succ_opt()
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .and_then(|time| {
                time.checked_sub_signed(Duration::hours(self.region.date_offset_hours()))
            })
            .map(|time| time.and_utc())
            .ok_or("游戏日超出日期范围")
    }
}
