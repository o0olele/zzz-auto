//! 快捷手册资源栏的严格解析；未知字段不能当作零电量。
use crate::charge_plan::ChargeResources;
use serde::{Deserialize, Serialize};

pub const BAR_ROI: [i32; 4] = [1220, 35, 550, 75];
pub const DOUBLE_LABEL_ROI: [i32; 4] = [1000, 346, 400, 40];
pub const DOUBLE_COUNT_ROI: [i32; 4] = [1270, 346, 110, 40];
pub const DOUBLE_LABEL: &str = "每日怪物卡双倍掉落次数";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceWord {
    pub text: String,
    pub rect: [i32; 4],
}

fn number(text: &str) -> Option<u32> {
    (!text.is_empty() && text.len() <= 7 && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse::<u32>().ok())
        .flatten()
        .filter(|n| *n <= 1_000_000)
}
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == '／' { '/' } else { c })
        .collect()
}
fn current_charge(text: &str) -> Option<u32> {
    if let Some((current, cap)) = text.split_once('/') {
        return (cap == "240" && current.len() <= 3)
            .then(|| number(current))
            .flatten();
    }
    // 固定图中灰色斜杠会被识别为 1 / I / l；只接受完整 240 后缀且当前值至多三位的分数。
    for suffix in ["1240", "I240", "l240"] {
        if let Some(current) = text.strip_suffix(suffix)
            && (1..=3).contains(&current.len())
        {
            return number(current);
        }
    }
    (text.len() <= 3).then(|| number(text)).flatten()
}
pub fn parse_bar(words: &[ResourceWord]) -> Result<ChargeResources, &'static str> {
    if words.len() > 32 {
        return Err("资源栏文字数量超限");
    }
    let mut values = [None; 3];
    // 参考解析器使用的三个相对中心为 173 / 348 / 485。
    let centers = [1393, 1568, 1705];
    for word in words {
        let text = normalize(&word.text);
        if !text.bytes().any(|c| c.is_ascii_digit()) {
            continue;
        }
        let [x, y, w, h] = word.rect;
        if x < 1220
            || y < 35
            || w <= 0
            || h <= 0
            || x.checked_add(w).is_none_or(|r| r > 1770)
            || y.checked_add(h).is_none_or(|b| b > 110)
        {
            return Err("资源文字框超出固定栏位");
        }
        let center = x + w / 2;
        let slot = centers
            .iter()
            .enumerate()
            .min_by_key(|(_, anchor)| (center - **anchor).abs())
            .unwrap()
            .0;
        // 数字右对齐，位数会使框整体移动；按中心归位，不能裁掉完整电量分数。
        if (center - centers[slot]).abs() > 90 || w > 210 {
            return Err("资源文字不能唯一对应固定栏位");
        }
        let value = if slot == 0 {
            // 原资源栏的分母为灰色，白字过滤后可能只剩当前值；位置和三个唯一栏位仍须完整。
            current_charge(&text)
        } else {
            number(&text)
        }
        .ok_or("资源数值格式无效")?;
        if values[slot].replace(value).is_some() {
            return Err("同一资源栏位出现多个候选，不能选择最近值冒充确定结果");
        }
    }
    let resources = ChargeResources {
        charge: values[0].ok_or("当前电量未知")?,
        backup: values[1].ok_or("储蓄电量未知")?,
        ether: values[2].ok_or("以太电池未知")?,
    };
    resources.validate()?;
    Ok(resources)
}
pub fn parse_double(text: &str) -> Option<u8> {
    let text = normalize(text);
    let (left, cap) = text.split_once('/')?;
    (cap == "5" && left.len() == 1)
        .then(|| number(left))
        .flatten()
        .filter(|n| *n <= 5)
        .map(|n| n as u8)
}

/// 只有完整标签与独立单行次数共同支持的余量才可采用；未命中不是无活动。
pub fn resolve_double(title: &[ResourceWord], count: &[ResourceWord]) -> Option<u8> {
    let within = |word: &ResourceWord, [left, top, width, height]: [i32; 4]| {
        let [x, y, w, h] = word.rect;
        x >= left
            && y >= top
            && w > 0
            && h > 0
            && x.checked_add(w).is_some_and(|r| r <= left + width)
            && y.checked_add(h).is_some_and(|b| b <= top + height)
    };
    if title.is_empty()
        || title.len() > 8
        || count.len() != 1
        || title.iter().any(|w| !within(w, DOUBLE_LABEL_ROI))
        || !within(&count[0], DOUBLE_COUNT_ROI)
    {
        return None;
    }
    let mut title = title.to_vec();
    title.sort_by_key(|w| w.rect[0]);
    if title.windows(2).any(|pair| {
        let [x, y, width, height] = pair[0].rect;
        let [next_x, next_y, _, next_height] = pair[1].rect;
        x + width > next_x || (y + height / 2 - next_y - next_height / 2).abs() > 12
    }) {
        return None;
    }
    let joined = title.iter().map(|w| normalize(&w.text)).collect::<String>();
    let suffix = joined.strip_prefix(DOUBLE_LABEL)?;
    let left = parse_double(&count[0].text)?;
    (suffix.is_empty() || parse_double(suffix) == Some(left)).then_some(left)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeResourceObservation {
    pub handbook: bool,
    pub training: bool,
    pub resources: Option<ChargeResources>,
    /// 缺少可靠的双倍标签或分母时仍为未知，不能把未识别到活动当作零次。
    pub double_left: Option<u8>,
    pub issue: Option<String>,
}
