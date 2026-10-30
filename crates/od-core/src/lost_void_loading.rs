//! 明确加载观察；不提供出口意图、下一层身份或输入许可。
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadingKind {
    NowLoading,
    Lore,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadingObservation {
    pub kind: LoadingKind,
    pub fingerprint: String,
}
impl LoadingObservation {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.fingerprint.len() != 64 || !self.fingerprint.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("加载观察摘要无效");
        }
        Ok(())
    }
    fn new(kind: LoadingKind, text: &str) -> Self {
        Self {
            kind,
            fingerprint: hex::encode(Sha256::digest(text.as_bytes())),
        }
    }
}
fn normalized(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}
pub fn now_loading(words: &[String]) -> Option<LoadingObservation> {
    if words.is_empty() || words.len() > 3 {
        return None;
    }
    let text = normalized(&words.join("")).to_ascii_uppercase();
    (text.trim_end_matches(['.', '…']) == "NOWLOADING")
        .then(|| LoadingObservation::new(LoadingKind::NowLoading, "NOWLOADING"))
}
pub fn lore_logo() -> &'static [u8] {
    include_bytes!("../../../resources/lost-void/scene/loading-lore-logo.png")
}
pub fn lore_source() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../resources/lost-void/loading-lore-source.json"
    ))
    .expect("固定加载标识来源")
}
/// lore 内容随机，仅接受固定标识、括号标题、多行正文和有纹理灰度插画组合。
pub fn lore_loading(
    title: &[String],
    body: &[String],
    logo: bool,
    gray_art: bool,
) -> Option<LoadingObservation> {
    if !gray_art || !logo || title.len() != 1 || !(2..=12).contains(&body.len()) {
        return None;
    }
    let title = normalized(&title[0]);
    let inner = title
        .strip_prefix('【')
        .and_then(|t| t.strip_suffix('】'))
        .or_else(|| title.strip_prefix('[').and_then(|t| t.strip_suffix(']')))?;
    if !(2..=40).contains(&inner.chars().count()) || inner.chars().any(char::is_control) {
        return None;
    }
    let body: Vec<_> = body
        .iter()
        .map(|s| normalized(s))
        .filter(|s| !s.is_empty())
        .collect();
    let text = body.join("\n");
    let chinese = text
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .count();
    if body.len() < 2
        || !(12..=512).contains(&chinese)
        || text.chars().count() > 800
        || text.chars().any(|c| c.is_control() && c != '\n')
        || body.iter().any(|s| {
            [
                "确认",
                "取消",
                "确定",
                "退出挑战",
                "返回大厅",
                "开始挑战",
                "预备编队",
            ]
            .contains(&s.as_str())
        })
    {
        return None;
    }
    Some(LoadingObservation::new(
        LoadingKind::Lore,
        &format!("{title}\n{text}"),
    ))
}
