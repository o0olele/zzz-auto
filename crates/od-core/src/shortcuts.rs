use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutKey {
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
}

impl ShortcutKey {
    pub const ALL: [Self; 11] = [
        Self::F1,
        Self::F2,
        Self::F3,
        Self::F4,
        Self::F5,
        Self::F6,
        Self::F7,
        Self::F8,
        Self::F9,
        Self::F10,
        Self::F11,
    ];

    pub fn virtual_key(self) -> u32 {
        0x70 + Self::ALL
            .iter()
            .position(|key| *key == self)
            .expect("固定按键") as u32
    }

    pub fn from_legacy(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|key| format!("{key:?}").eq_ignore_ascii_case(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ShortcutSettings {
    #[serde(with = "required_key")]
    #[ts(as = "Option<ShortcutKey>")]
    pub pause_resume: Option<ShortcutKey>,
    #[serde(with = "required_key")]
    #[ts(as = "Option<ShortcutKey>")]
    pub stop: Option<ShortcutKey>,
}

// 字段缺失是错误；显式 null 才表示关闭。
mod required_key {
    use super::*;

    pub fn serialize<S: serde::Serializer>(
        key: &Option<ShortcutKey>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        key.serialize(serializer)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<ShortcutKey>, D::Error> {
        Option::deserialize(deserializer)
    }
}

impl Default for ShortcutSettings {
    fn default() -> Self {
        Self {
            pause_resume: Some(ShortcutKey::F9),
            stop: Some(ShortcutKey::F10),
        }
    }
}

impl ShortcutSettings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.pause_resume.is_some() && self.pause_resume == self.stop {
            Err("暂停 / 继续与停止快捷键不能相同")
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    PauseResume,
    Stop,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct ShortcutStatus {
    pub active: Option<ShortcutSettings>,
    pub registration_error: Option<String>,
    pub action_error: Option<String>,
}
