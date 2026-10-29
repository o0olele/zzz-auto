//! 账号级预备编队。位置对应游戏中的固定槽位，扫描可同步名称，脚本保持原下标。
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;

pub const TEAM_COUNT: usize = 20;
pub const UNKNOWN_AGENT: &str = "unknown";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PredefinedTeam {
    pub name: String,
    pub auto_battle: String,
    pub agent_ids: [String; 3],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TeamConfig {
    #[serde(default = "default_check_timeout")]
    #[ts(as = "Option<u32>", optional)]
    pub check_timeout_ms: u32,
    pub teams: Vec<PredefinedTeam>,
}
pub const fn default_check_timeout() -> u32 {
    300_000
}

impl Default for TeamConfig {
    fn default() -> Self {
        Self {
            check_timeout_ms: default_check_timeout(),
            teams: (1..=TEAM_COUNT)
                .map(|i| PredefinedTeam {
                    name: format!("编队{i}"),
                    auto_battle: "全配队通用".into(),
                    agent_ids: std::array::from_fn(|_| UNKNOWN_AGENT.into()),
                })
                .collect(),
        }
    }
}

impl TeamConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1000..=900_000).contains(&self.check_timeout_ms) {
            return Err("编队检查超时须为 1000..900000ms");
        }
        if self.teams.len() != TEAM_COUNT {
            return Err("预备编队须完整保留游戏中的 20 个槽位");
        }
        let agents: BTreeSet<_> = crate::random_play::agents()
            .into_iter()
            .map(|a| a.id)
            .collect();
        for team in &self.teams {
            if team.name.is_empty()
                || team.name.trim() != team.name
                || team.name.chars().count() > 80
                || team.name.chars().any(char::is_control)
            {
                return Err("编队名称须为 1..80 个字符，无首尾空白或控制字符");
            }
            if team.auto_battle.is_empty()
                || team.auto_battle.len() > 240
                || team.auto_battle.trim() != team.auto_battle
                || team.auto_battle.chars().any(char::is_control)
                || team.auto_battle.contains(['/', '\\', ':'])
                || matches!(team.auto_battle.as_str(), "." | "..")
            {
                return Err("编队战斗脚本名称无效");
            }
            let mut selected = BTreeSet::new();
            for id in &team.agent_ids {
                if id != UNKNOWN_AGENT && (!agents.contains(id) || !selected.insert(id)) {
                    return Err("编队成员须为已登记代理人或 unknown，不能重复同一代理人");
                }
            }
        }
        Ok(())
    }

    pub fn get(&self, index: u8) -> Result<&PredefinedTeam, &'static str> {
        self.teams
            .get(usize::from(index))
            .ok_or("预备编队编号超出 1..20")
    }
}

/// 旧 team.yml 的数组位置就是槽位；不足 20 项按旧版默认值补齐。
pub fn import(value: &serde_json::Value) -> Result<TeamConfig, &'static str> {
    let object = value.as_object().ok_or("旧预备编队配置必须是对象")?;
    let mut config = TeamConfig::default();
    if let Some(list) = object.get("team_list") {
        let list = list.as_array().ok_or("旧 team_list 必须是数组")?;
        if list.len() > TEAM_COUNT {
            return Err("旧预备编队超过 20 项，不能截断导入");
        }
        for (index, entry) in list.iter().enumerate() {
            let entry = entry.as_object().ok_or("旧编队条目必须是对象")?;
            let team = &mut config.teams[index];
            for (key, target) in [
                ("name", &mut team.name),
                ("auto_battle", &mut team.auto_battle),
            ] {
                if let Some(value) = entry.get(key) {
                    *target = value.as_str().ok_or("旧编队名称和脚本必须是文本")?.into();
                }
            }
            if let Some(ids) = entry.get("agent_id_list") {
                let ids = ids.as_array().ok_or("旧编队代理人列表必须是数组")?;
                if ids.len() > 3 {
                    return Err("旧编队代理人超过三名");
                }
                for (slot, id) in ids.iter().enumerate() {
                    team.agent_ids[slot] = id.as_str().ok_or("旧编队代理人编号必须是文本")?.into();
                }
            }
        }
    }
    for team in &mut config.teams {
        team.name = normalize_name(&team.name);
    }
    config.validate()?;
    Ok(config)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TeamMembers {
    pub index: u8,
    /// 保留启动时的名称，绑定原配置；实际读到的名称单独保存。
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub observed_name: Option<String>,
    pub member_count: u8,
    pub agent_ids: [String; 3],
}

pub fn normalize_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 合并已识别槽位；旧回执只更新成员，新扫描可带实际名称，不修改脚本。
pub fn apply_members(
    config: &TeamConfig,
    results: &[TeamMembers],
) -> Result<TeamConfig, &'static str> {
    config.validate()?;
    if results.is_empty() || results.len() > TEAM_COUNT {
        return Err("没有有效的编队识别结果");
    }
    let mut next = config.clone();
    let mut indices = BTreeSet::new();
    for result in results {
        let team = next
            .teams
            .get_mut(usize::from(result.index))
            .ok_or("识别结果的编队槽位无效")?;
        if !indices.insert(result.index) || result.name != team.name {
            return Err("编队识别结果重复或名称与冻结配置不一致");
        }
        if result.member_count > 3
            || result
                .agent_ids
                .iter()
                .filter(|id| id.as_str() != UNKNOWN_AGENT)
                .count()
                != usize::from(result.member_count)
        {
            return Err("识别出的代理人数与画面编队人数不一致");
        }
        team.agent_ids = result.agent_ids.clone();
        if let Some(name) = &result.observed_name {
            let name = normalize_name(name);
            if name.is_empty() {
                return Err("编队实际名称不能为空");
            }
            team.name = name;
        }
    }
    next.validate()?;
    Ok(next)
}

/// 与 difflib.SequenceMatcher 的匹配块算法一致（本处名称最长 80 字，无 autojunk）。
pub fn name_similarity(candidate: &str, recognized: &str) -> f64 {
    let a: Vec<_> = candidate.chars().collect();
    let b: Vec<_> = recognized.chars().collect();
    if a.len() > 80 || b.len() > 80 {
        return 0.0;
    }
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let mut pending = vec![(0, a.len(), 0, b.len())];
    let mut matched = 0;
    while let Some((alo, ahi, blo, bhi)) = pending.pop() {
        let (mut best_a, mut best_b, mut best_len) = (alo, blo, 0);
        let mut previous = vec![0; b.len() + 1];
        for (i, a_char) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut current = vec![0; b.len() + 1];
            for j in blo..bhi {
                if *a_char == b[j] {
                    let length = previous[j] + 1;
                    current[j + 1] = length;
                    if length > best_len {
                        (best_a, best_b, best_len) = (i + 1 - length, j + 1 - length, length);
                    }
                }
            }
            previous = current;
        }
        if best_len == 0 {
            continue;
        }
        matched += best_len;
        if alo < best_a && blo < best_b {
            pending.push((alo, best_a, blo, best_b));
        }
        if best_a + best_len < ahi && best_b + best_len < bhi {
            pending.push((best_a + best_len, ahi, best_b + best_len, bhi));
        }
    }
    2.0 * matched as f64 / (a.len() + b.len()) as f64
}

/// 同分的模糊结果不选，避免重复或接近的名字覆盖错误槽位。
pub fn match_name(config: &TeamConfig, recognized: &str) -> Option<u8> {
    let exact: Vec<_> = config
        .teams
        .iter()
        .enumerate()
        .filter(|(_, team)| team.name == recognized)
        .map(|(index, _)| index)
        .collect();
    if !exact.is_empty() {
        return (exact.len() == 1).then(|| exact[0] as u8);
    }
    let mut scores: Vec<_> = config
        .teams
        .iter()
        .enumerate()
        .filter_map(|(index, team)| {
            let score = name_similarity(&team.name, recognized);
            (score >= 0.6).then_some((index, score))
        })
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (index, best) = scores.first().copied()?;
    if scores
        .get(1)
        .is_some_and(|(_, second)| (best - second).abs() < 1e-9)
    {
        return None;
    }
    u8::try_from(index).ok()
}

pub fn member_count(text: &str) -> Option<u8> {
    let text: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == '／' { '/' } else { c })
        .collect();
    let chars: Vec<_> = text.chars().collect();
    if chars.len() != 3 || !matches!(chars[1], '/' | '1') {
        return None;
    }
    let active = chars[0].to_digit(10)?;
    let count = chars[2].to_digit(10)?;
    (count <= 3 && active <= count).then_some(count as u8)
}

#[derive(Debug, Clone)]
pub struct AvatarMatch {
    pub agent_id: String,
    pub rect: [i32; 4],
    pub score: f64,
}

/// 按卡片中的三个位置去重。不同角色在同一位置的近似分数视为不明确。
pub fn resolve_members(
    name_x: i32,
    name_y: i32,
    count: u8,
    matches: &[AvatarMatch],
) -> Option<[String; 3]> {
    if count > 3 {
        return None;
    }
    let mut slots: [Vec<&AvatarMatch>; 3] = Default::default();
    for hit in matches {
        let [x, y, w, h] = hit.rect;
        if w < 40 || h < 40 || w > 260 || h > 260 || !hit.score.is_finite() {
            continue;
        }
        let center_x = x + w / 2 - name_x + 10;
        let center_y = y + h / 2 - name_y;
        if !(0..582).contains(&center_x) || !(40..240).contains(&center_y) {
            continue;
        }
        slots[(center_x / 194) as usize].push(hit);
    }
    let mut result = std::array::from_fn(|_| UNKNOWN_AGENT.to_string());
    for (slot, candidates) in slots.iter_mut().enumerate() {
        candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
        let Some(best) = candidates.first() else {
            continue;
        };
        if candidates.iter().skip(1).any(|other| {
            other.agent_id != best.agent_id
                && best.score - other.score <= (best.score * 0.05).max(1.0)
        }) {
            return None;
        }
        result[slot] = best.agent_id.clone();
    }
    let known: Vec<_> = result
        .iter()
        .filter(|id| id.as_str() != UNKNOWN_AGENT)
        .collect();
    if known.len() != usize::from(count)
        || known.iter().collect::<BTreeSet<_>>().len() != known.len()
    {
        return None;
    }
    Some(result)
}
