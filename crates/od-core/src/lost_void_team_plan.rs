//! 三类副本的编队意图与脚本选择；不提供游戏中已选队伍的证明。
use crate::{
    lost_void::{LostVoidChallenge, LostVoidMission as M},
    predefined_teams::{TeamConfig, UNKNOWN_AGENT},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LostVoidTeamKind {
    KeepCurrent,
    Predefined { index: u8 },
    MatrixManual { agents: [String; 3] },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidTeamPlan {
    pub kind: LostVoidTeamKind,
    pub script_name: String,
    pub expected_agents: [String; 3],
    pub up_agents: Vec<String>,
    pub chose_by_up: bool,
}
fn validate_agents(ids: &[String], allow_unknown: bool) -> Result<(), &'static str> {
    let known: BTreeSet<_> = crate::random_play::agents()
        .into_iter()
        .map(|a| a.id)
        .collect();
    let mut seen = BTreeSet::new();
    if ids.iter().any(|id| {
        if allow_unknown && id == UNKNOWN_AGENT {
            false
        } else {
            !known.contains(id) || !seen.insert(id)
        }
    }) {
        return Err("迷失之地代理人须来自冻结目录且不能重复");
    }
    Ok(())
}
/// None 表示 UP 配队仍缺本周画面，不能当作空列表回退。
pub fn plan_team(
    mission: M,
    challenge: &LostVoidChallenge,
    teams: &TeamConfig,
    up_agents: Option<&[String]>,
    up_completed_this_week: bool,
) -> Result<Option<LostVoidTeamPlan>, &'static str> {
    challenge.validate()?;
    teams.validate()?;
    if let Some(up) = up_agents {
        if up.len() != 3 {
            return Err("迷失之地 UP 名单必须完整且非空");
        }
        validate_agents(up, false)?;
    }
    let up = up_agents.unwrap_or_default().to_vec();
    if mission == M::MatrixAction && challenge.manually_choose_agent {
        validate_agents(&challenge.team_info, false)?;
        let agents: [String; 3] = challenge
            .team_info
            .clone()
            .try_into()
            .map_err(|_| "矩阵主战须有三名代理人")?;
        return Ok(Some(LostVoidTeamPlan {
            kind: LostVoidTeamKind::MatrixManual {
                agents: agents.clone(),
            },
            script_name: challenge.auto_battle.clone(),
            expected_agents: agents,
            up_agents: up,
            chose_by_up: false,
        }));
    }
    let mut index = challenge.predefined_team_idx;
    let mut chose_by_up = false;
    if mission == M::MatrixAction && index < 0 {
        index = 0;
    }
    if mission == M::TaskForce && challenge.choose_team_by_priority && !up_completed_this_week {
        let Some(up) = up_agents else { return Ok(None) };
        let mut best = 0;
        for (i, t) in teams.teams.iter().enumerate() {
            let count = up.iter().filter(|id| t.agent_ids.contains(id)).count();
            if count > best {
                best = count;
                index = i as i8;
                chose_by_up = true;
            }
        }
    }
    if index < 0 {
        Ok(Some(LostVoidTeamPlan {
            kind: LostVoidTeamKind::KeepCurrent,
            script_name: challenge.auto_battle.clone(),
            expected_agents: std::array::from_fn(|_| UNKNOWN_AGENT.into()),
            up_agents: up,
            chose_by_up,
        }))
    } else {
        let t = teams.get(index as u8)?;
        Ok(Some(LostVoidTeamPlan {
            kind: LostVoidTeamKind::Predefined { index: index as u8 },
            script_name: t.auto_battle.clone(),
            expected_agents: t.agent_ids.clone(),
            up_agents: up,
            chose_by_up,
        }))
    }
}
impl LostVoidTeamPlan {
    pub fn verify_roster(&self, actual: &[String]) -> Result<bool, &'static str> {
        if actual.is_empty() || actual.len() > 3 {
            return Err("迷失之地当前队伍人数无效");
        }
        validate_agents(actual, false)?;
        Ok(self
            .expected_agents
            .iter()
            .enumerate()
            .all(|(i, id)| id == UNKNOWN_AGENT || actual.get(i) == Some(id)))
    }
    pub fn has_up_member(&self, actual: &[String]) -> Result<bool, &'static str> {
        if !self.verify_roster(actual)? {
            return Ok(false);
        }
        Ok(actual.iter().any(|id| self.up_agents.contains(id)))
    }
}

/// 首次输入前要准备的编队。UP 尚未知时枚举完整三人名单，保留可能胜出的
/// 编队和配置回退；始终被更早同成员编队覆盖的槽位不构成可选分支。
pub fn potential_teams(
    mission: M,
    challenge: &LostVoidChallenge,
    teams: &TeamConfig,
    up_completed_this_week: bool,
) -> Result<Vec<LostVoidTeamKind>, &'static str> {
    let fallback =
        plan_team(mission, challenge, teams, None, true)?.ok_or("迷失之地缺少脚本回退方案")?;
    let mut candidates = BTreeSet::from([fallback.kind]);
    if mission != M::TaskForce || !challenge.choose_team_by_priority || up_completed_this_week {
        return Ok(candidates.into_iter().collect());
    }
    let members: Vec<[bool; 20]> = crate::random_play::agents()
        .into_iter()
        .map(|a| std::array::from_fn(|i| teams.teams[i].agent_ids.contains(&a.id)))
        .collect();
    for a in 0..members.len() {
        for b in a + 1..members.len() {
            for c in b + 1..members.len() {
                let mut best = 0;
                let mut winner = None;
                for (i, ((aa, bb), cc)) in members[a]
                    .iter()
                    .zip(&members[b])
                    .zip(&members[c])
                    .enumerate()
                {
                    let count = u8::from(*aa) + u8::from(*bb) + u8::from(*cc);
                    if count > best {
                        best = count;
                        winner = Some(i);
                    }
                }
                if let Some(i) = winner {
                    candidates.insert(LostVoidTeamKind::Predefined { index: i as u8 });
                }
            }
        }
    }
    Ok(candidates.into_iter().collect())
}

/// 脚本与身份预检共用候选编队，避免相同脚本名掩盖另一个槽位的资源需求。
pub fn potential_scripts(
    mission: M,
    challenge: &LostVoidChallenge,
    teams: &TeamConfig,
    up_completed_this_week: bool,
) -> Result<Vec<String>, &'static str> {
    potential_teams(mission, challenge, teams, up_completed_this_week)?
        .into_iter()
        .map(|kind| match kind {
            LostVoidTeamKind::Predefined { index } => Ok(teams.get(index)?.auto_battle.clone()),
            _ => Ok(challenge.auto_battle.clone()),
        })
        .collect::<Result<BTreeSet<_>, _>>()
        .map(|scripts| scripts.into_iter().collect())
}

/// unknown 或保持当前需要完整目录；它们不能当作已经确认的成员身份。
pub fn potential_agents(
    mission: M,
    challenge: &LostVoidChallenge,
    teams: &TeamConfig,
    up_completed_this_week: bool,
) -> Result<BTreeSet<String>, &'static str> {
    let mut required = BTreeSet::new();
    for kind in potential_teams(mission, challenge, teams, up_completed_this_week)? {
        let agents = match kind {
            LostVoidTeamKind::KeepCurrent => None,
            LostVoidTeamKind::Predefined { index } => {
                let ids = &teams.get(index)?.agent_ids;
                (!ids.iter().any(|id| id == UNKNOWN_AGENT)).then_some(ids.clone())
            }
            LostVoidTeamKind::MatrixManual { agents } => Some(agents),
        };
        let Some(agents) = agents else {
            return Ok(crate::random_play::agents()
                .into_iter()
                .map(|a| a.id)
                .collect());
        };
        required.extend(agents);
    }
    Ok(required)
}
