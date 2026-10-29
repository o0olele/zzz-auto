//! 固定 1080p 编队卡片布局，来自 team_select.yml；名称不决定槽位。
use crate::predefined_teams::normalize_name;

pub const CARDS: [[i32; 4]; 6] = [
    [160, 132, 773, 249],
    [976, 128, 779, 257],
    [162, 414, 782, 258],
    [983, 416, 767, 250],
    [163, 694, 780, 255],
    [976, 694, 781, 255],
];

pub fn sources() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../resources/daily/team-slots-source-lock.json"
    ))
    .expect("固定编队槽位来源")
}

pub fn index(page: u8, card: usize) -> Option<u8> {
    if page >= 5 || card >= CARDS.len() {
        return None;
    }
    let index = page * 4 + card as u8;
    (index < 20).then_some(index)
}

pub fn is_ui_text(text: &str) -> bool {
    let name = normalize_name(text).to_uppercase();
    matches!(
        name.as_str(),
        "1P" | "2P" | "3P" | "TEAM" | "AGENT" | "BANGBOO" | "60"
    ) || name.contains("SELECT")
        || crate::predefined_teams::member_count(&name).is_some()
}

/// 卡片顶部 70px 内最大的完整非界面文字；同面积异名保留未知。
pub fn name(card: [i32; 4], texts: &[(String, [i32; 4])]) -> Option<String> {
    let [left, top, width, height] = card;
    let mut candidates: Vec<_> = texts
        .iter()
        .filter(|(text, [x, y, w, h])| {
            *w > 0
                && *h > 0
                && *x >= left
                && *x + *w <= left + width
                && *y >= top
                && *y + *h <= top + height.min(70)
                && !is_ui_text(text)
        })
        .map(|(text, [_, _, w, h])| (normalize_name(text), i64::from(*w) * i64::from(*h)))
        .filter(|(name, _)| {
            !name.is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control)
        })
        .collect();
    candidates.sort_by_key(|(_, area)| std::cmp::Reverse(*area));
    let (name, area) = candidates.first()?;
    (!candidates
        .iter()
        .skip(1)
        .any(|(other, size)| size == area && other != name))
    .then(|| name.clone())
}
