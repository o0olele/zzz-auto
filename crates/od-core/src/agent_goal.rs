//! 培养达成只接受完整正向文字；不把缺少标记当作达成。
use crate::simulation_settlement::SettlementView;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentGoalStatus {
    Achieved,
    NotAchieved,
    Unknown,
}
impl AgentGoalStatus {
    pub fn from_words(words: &[String]) -> Self {
        let words: Vec<String> = words
            .iter()
            .map(|w| w.chars().filter(|c| !c.is_whitespace()).collect())
            .filter(|s: &String| !s.is_empty())
            .collect();
        if words.len() == 1 && ["已达成", "已達成"].contains(&words[0].as_str()) {
            return Self::Achieved;
        }
        if words.len() == 1 && ["未达成", "未達成"].contains(&words[0].as_str()) {
            return Self::NotAchieved;
        }
        if words
            .iter()
            .any(|w| w.chars().any(|c| ['已', '达', '達', '成'].contains(&c)))
        {
            Self::Unknown
        } else {
            Self::NotAchieved
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSettlementObservation {
    pub view: SettlementView,
    pub goal: Option<AgentGoalStatus>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentFinishState {
    goal: Option<AgentGoalStatus>,
    posted: bool,
}
impl AgentFinishState {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.posted && self.goal.is_none() {
            return Err("培养完成动作缺少同次达成读数");
        }
        Ok(())
    }
    pub fn prepare(&mut self, goal: AgentGoalStatus) -> Result<(), &'static str> {
        self.validate()?;
        if self.goal.is_some() || self.posted {
            return Err("培养完成读数不能重复绑定");
        }
        self.goal = Some(goal);
        Ok(())
    }
    pub fn expected(&self) -> Option<AgentGoalStatus> {
        if self.posted { None } else { self.goal }
    }
    pub fn posted(&mut self) -> Result<(), &'static str> {
        if self.goal.is_none() || self.posted {
            return Err("培养完成投递缺少意图或已经投递");
        }
        self.posted = true;
        self.validate()
    }
    pub fn not_posted(&mut self) -> Result<(), &'static str> {
        if self.posted || self.goal.is_none() {
            return Err("已投递或无意图的培养完成不能撤销");
        }
        self.goal = None;
        self.validate()
    }
    pub fn status(&self) -> Option<AgentGoalStatus> {
        self.posted.then_some(self.goal).flatten()
    }
    /// 仅返回已保存的文字读数；调用方仍须取得正常战斗和实际结算返回证明。
    pub fn observed(&self) -> Option<AgentGoalStatus> {
        self.goal
    }
}
