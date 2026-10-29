//! 深度追猎等级只采用完整的固定目录读数；点击不等于选中。
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub fn canonical_level(text: &str) -> Option<String> {
    let text: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    crate::charge_plan::LEVELS[1..]
        .iter()
        .find(|level| level.to_lowercase() == text)
        .map(|s| s.to_string())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LevelReading {
    pub current: Option<String>,
    pub choices: Vec<String>,
}
impl LevelReading {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self
            .current
            .as_ref()
            .is_some_and(|s| canonical_level(s).as_ref() != Some(s))
            || self.choices.len() > 5
            || self
                .choices
                .iter()
                .any(|s| canonical_level(s).as_ref() != Some(s))
            || self.choices.iter().collect::<BTreeSet<_>>().len() != self.choices.len()
        {
            return Err("等级读数不是唯一的完整目录项");
        }
        Ok(())
    }
    pub fn selected(&self, wanted: &str) -> bool {
        self.validate().is_ok()
            && self.choices.is_empty()
            && self.current.as_deref() == Some(wanted)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelView {
    Unknown,
    Blocked,
    Page(LevelReading),
}
