//! 模拟室培养标记决定本次关卡，投递后回读不代替选中证明。
use crate::{
    charge_plan::catalog, simulation_cards::CardsCharge, simulation_mission::MissionLabel,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMissionRequest {
    pub mission_type: String,
    pub timeout_ms: u32,
}
impl AgentMissionRequest {
    pub fn names(&self) -> Result<Vec<&str>, &'static str> {
        if !(1000..=900_000).contains(&self.timeout_ms)
            || !crate::agent_transport::valid_mission("实战模拟室", &self.mission_type)
        {
            return Err("代理人选关需要本次具体模拟室类型和有效时限");
        }
        Ok(catalog()
            .iter()
            .find(|c| c.category_name == "实战模拟室")
            .ok_or("缺少模拟室目录")?
            .mission_type_list
            .iter()
            .find(|m| m.mission_type_name == self.mission_type)
            .ok_or("缺少模拟室类型")?
            .mission_list
            .iter()
            .map(|m| m.mission_name.as_str())
            .collect())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.names()?.is_empty() {
            return Err("代理人选关类型缺少关卡");
        }
        Ok(())
    }
    pub fn known_list(&self, labels: &[MissionLabel]) -> Result<bool, &'static str> {
        let names = self.names()?;
        if labels.len() > 32 {
            return Err("代理人列表文字过多");
        }
        let all: BTreeSet<_> = catalog()
            .iter()
            .find(|c| c.category_name == "实战模拟室")
            .ok_or("模拟室目录缺失")?
            .mission_type_list
            .iter()
            .flat_map(|m| m.mission_list.iter().map(|n| n.mission_name.as_str()))
            .collect();
        let mut visible = Vec::new();
        for label in labels {
            label.validate()?;
            if let Some(index) = names.iter().position(|n| *n == label.text) {
                visible.push((index, label));
            } else if all.contains(label.text.as_str()) {
                return Err("代理人列表属于其他类型");
            }
        }
        visible.sort_by_key(|(_, l)| l.rect[0]);
        if visible
            .windows(2)
            .any(|p| p[0].0 >= p[1].0 || p[0].1.rect[0] + p[0].1.rect[2] > p[1].1.rect[0])
        {
            return Err("代理人关卡文字重复、重叠或目录顺序异常");
        }
        Ok(!visible.is_empty())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMissionTarget {
    pub mission_name: String,
    pub marker_sha256: String,
}
impl AgentMissionTarget {
    pub fn validate(&self, request: &AgentMissionRequest) -> Result<(), &'static str> {
        if !request.names()?.contains(&self.mission_name.as_str())
            || self.marker_sha256.len() != 64
            || !self
                .marker_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("代理人关卡缺少完整目录名称或标记指纹");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMissionView {
    Unknown,
    Blocked,
    Other,
    Categories {
        names: Vec<String>,
    },
    Challenge {
        labels: Vec<MissionLabel>,
        charge: CardsCharge,
        back: bool,
        candidate: Option<AgentMissionTarget>,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMissionAction {
    Back,
    ChooseType,
    Scroll(u8),
    ChooseMission,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMissionReceipt {
    pub mission_type: String,
    pub target: AgentMissionTarget,
    pub charge: CardsCharge,
    pub selection_verified: bool,
}
impl AgentMissionReceipt {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.target.validate(&AgentMissionRequest {
            mission_type: self.mission_type.clone(),
            timeout_ms: 1000,
        })?;
        self.charge.validate()?;
        if self.selection_verified {
            return Err("代理人投递回执不能代替选中证明");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Inspect,
    Back,
    Type,
    Search,
    Choose,
    Readback,
    Submitted,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Receipt {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMissionFlow {
    request: AgentMissionRequest,
    phase: Phase,
    pending: Option<(AgentMissionAction, Receipt)>,
    scrolls: u8,
    target: Option<AgentMissionTarget>,
    receipt: Option<AgentMissionReceipt>,
}
impl AgentMissionFlow {
    pub fn new(request: AgentMissionRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            phase: Phase::Inspect,
            pending: None,
            scrolls: 0,
            target: None,
            receipt: None,
        })
    }
    pub fn request(&self) -> &AgentMissionRequest {
        &self.request
    }
    pub fn target(&self) -> Option<&AgentMissionTarget> {
        self.target.as_ref()
    }
    pub fn receipt(&self) -> Option<&AgentMissionReceipt> {
        self.receipt.as_ref()
    }
    pub fn completed(&self) -> bool {
        self.phase == Phase::Submitted
    }
    pub fn unavailable(&self) -> bool {
        self.phase == Phase::Unavailable
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if let Some(t) = &self.target {
            t.validate(&self.request)?;
        }
        if let Some(r) = &self.receipt {
            r.validate()?;
            if r.mission_type != self.request.mission_type
                || self.target.as_ref() != Some(&r.target)
            {
                return Err("代理人回读不能替换本次类型或目标");
            }
        }
        if self.scrolls > 10
            || self.completed() != self.receipt.is_some()
            || self.target.is_some()
                != matches!(
                    self.phase,
                    Phase::Choose | Phase::Readback | Phase::Submitted
                )
            || (self.unavailable() && self.scrolls != 10)
        {
            return Err("代理人选关检查点不一致");
        }
        if let Some((a, r)) = self.pending
            && (!self.phase_allows(a)
                || (r == Receipt::Posted
                    && !matches!(a, AgentMissionAction::Back | AgentMissionAction::ChooseType)))
        {
            return Err("代理人选关投递意图不一致");
        }
        Ok(())
    }
    fn phase_allows(&self, a: AgentMissionAction) -> bool {
        matches!(
            (self.phase, a),
            (Phase::Back, AgentMissionAction::Back)
                | (Phase::Type, AgentMissionAction::ChooseType)
                | (Phase::Choose, AgentMissionAction::ChooseMission)
        ) || matches!((self.phase,a),(Phase::Search,AgentMissionAction::Scroll(n)) if n == self.scrolls && n < 10)
    }
    fn candidate_valid(&self, labels: &[MissionLabel], target: &AgentMissionTarget) -> bool {
        target.validate(&self.request).is_ok()
            && self.request.known_list(labels) == Ok(true)
            && labels
                .iter()
                .filter(|l| l.text == target.mission_name)
                .count()
                == 1
    }
    fn page_allows(&self, a: AgentMissionAction, v: &AgentMissionView) -> bool {
        match (a, v) {
            (AgentMissionAction::Back, AgentMissionView::Challenge { back: true, .. }) => true,
            (AgentMissionAction::ChooseType, AgentMissionView::Categories { names }) => {
                names
                    .iter()
                    .filter(|n| **n == self.request.mission_type)
                    .count()
                    == 1
            }
            (
                AgentMissionAction::Scroll(_),
                AgentMissionView::Challenge {
                    labels,
                    charge,
                    candidate: None,
                    ..
                },
            ) => charge.validate().is_ok() && self.request.known_list(labels) == Ok(true),
            (
                AgentMissionAction::ChooseMission,
                AgentMissionView::Challenge {
                    labels,
                    charge,
                    candidate: Some(t),
                    ..
                },
            ) => {
                charge.validate().is_ok()
                    && self.candidate_valid(labels, t)
                    && self.target.as_ref() == Some(t)
            }
            _ => false,
        }
    }
    pub fn next(
        &mut self,
        v: &AgentMissionView,
    ) -> Result<Option<AgentMissionAction>, &'static str> {
        self.validate()?;
        if self.completed() || self.unavailable() {
            return Ok(None);
        }
        if matches!(v, AgentMissionView::Blocked | AgentMissionView::Other) {
            return Err("代理人选择出现不支持的页面");
        }
        if let AgentMissionView::Challenge { labels, charge, .. } = v {
            charge.validate()?;
            if matches!(self.phase, Phase::Search | Phase::Choose | Phase::Readback) {
                self.request.known_list(labels)?;
            }
        }
        if self.phase == Phase::Inspect {
            self.phase = match v {
                AgentMissionView::Categories { .. } => Phase::Type,
                AgentMissionView::Challenge { .. } => Phase::Back,
                _ => return Ok(None),
            };
        }
        if let Some((a, r)) = self.pending {
            if r != Receipt::Posted {
                return Err("代理人选择输入投递尚未确认");
            }
            let next = match (a, v) {
                (AgentMissionAction::Back, AgentMissionView::Categories { .. }) => Phase::Type,
                (AgentMissionAction::ChooseType, AgentMissionView::Challenge { labels, .. })
                    if self.request.known_list(labels)? =>
                {
                    Phase::Search
                }
                _ => return Ok(None),
            };
            self.phase = next;
            self.pending = None;
        }
        if self.phase == Phase::Readback {
            if let AgentMissionView::Challenge { labels, charge, .. } = v {
                let target = self.target.as_ref().ok_or("本次代理人关卡缺失")?;
                if self.candidate_valid(labels, target) {
                    self.receipt = Some(AgentMissionReceipt {
                        mission_type: self.request.mission_type.clone(),
                        target: target.clone(),
                        charge: *charge,
                        selection_verified: false,
                    });
                    self.phase = Phase::Submitted;
                }
            }
            self.validate()?;
            return Ok(None);
        }
        if self.phase == Phase::Search
            && let AgentMissionView::Challenge {
                labels, candidate, ..
            } = v
        {
            if let Some(t) = candidate {
                if self.candidate_valid(labels, t) {
                    self.target = Some(t.clone());
                    self.phase = Phase::Choose;
                }
            } else if self.scrolls == 10 && self.request.known_list(labels)? {
                self.phase = Phase::Unavailable;
            }
        }
        let action = match self.phase {
            Phase::Back => AgentMissionAction::Back,
            Phase::Type => AgentMissionAction::ChooseType,
            Phase::Search if self.scrolls < 10 => AgentMissionAction::Scroll(self.scrolls),
            Phase::Choose => AgentMissionAction::ChooseMission,
            _ => return Ok(None),
        };
        self.validate()?;
        Ok(self.page_allows(action, v).then_some(action))
    }
    pub fn prepare(&mut self, a: AgentMissionAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some() || !self.phase_allows(a) {
            return Err("代理人选择没有新的待保存动作");
        }
        self.pending = Some((a, Receipt::Intent));
        self.validate()
    }
    pub fn allows(&self, a: AgentMissionAction, v: &AgentMissionView) -> bool {
        self.validate().is_ok()
            && self.pending == Some((a, Receipt::Intent))
            && self.page_allows(a, v)
    }
    pub fn posted(&mut self, a: AgentMissionAction) -> Result<(), &'static str> {
        if self.pending != Some((a, Receipt::Intent)) {
            return Err("代理人选择投递回执不匹配");
        }
        match a {
            AgentMissionAction::Back | AgentMissionAction::ChooseType => {
                self.pending = Some((a, Receipt::Posted))
            }
            AgentMissionAction::Scroll(_) => {
                self.scrolls += 1;
                self.pending = None;
            }
            AgentMissionAction::ChooseMission => {
                self.phase = Phase::Readback;
                self.pending = None;
            }
        }
        self.validate()
    }
    pub fn not_posted(&mut self, a: AgentMissionAction) -> Result<(), &'static str> {
        if self.pending != Some((a, Receipt::Intent)) {
            return Err("已投递代理人选择不能撤销");
        }
        self.pending = None;
        if a == AgentMissionAction::ChooseMission {
            self.target = None;
            self.phase = Phase::Search;
        }
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some_and(|(_, r)| r == Receipt::Intent) {
            return Err("代理人选择输入结果不确定，不能重放");
        }
        if self.completed() {
            self.receipt = None;
            self.phase = Phase::Readback;
        }
        if self.unavailable() {
            self.phase = Phase::Search;
        }
        self.validate()
    }
}
