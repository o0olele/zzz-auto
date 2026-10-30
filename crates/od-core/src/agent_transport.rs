//! 代理人培养目标由同次手册标记解析；传送回执不提供选中或消费许可。
pub use crate::charge_transport::TrainingTransportAction as AgentTransportAction;
use crate::{
    charge_plan::{ChargePlanItem, catalog},
    charge_transport::{TrainingArrival, canonical_name},
    simulation_cards::SimulationView,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTransportRequest {
    pub plan: ChargePlanItem,
    pub timeout_ms: u32,
}
impl AgentTransportRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.plan.validate()?;
        if !(1000..=900_000).contains(&self.timeout_ms)
            || self.plan.mission_type_name != "代理人方案培养"
            || !["实战模拟室", "区域巡防", "专业挑战室", "恶名狩猎"]
                .contains(&self.plan.category_name.as_str())
            || self.plan.category_name != "恶名狩猎" && self.plan.level != "默认等级"
        {
            return Err("代理人传送需要支持分类的原始培养计划和有效时限");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTransportTarget {
    pub mission_type: String,
    pub marker_sha256: String,
}
impl AgentTransportTarget {
    pub fn validate(&self, category: &str) -> Result<(), &'static str> {
        if !valid_mission(category, &self.mission_type)
            || self.marker_sha256.len() != 64
            || !self
                .marker_sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("代理人目标缺少完整副本名称或标记像素指纹");
        }
        Ok(())
    }
}
pub fn valid_mission(category: &str, name: &str) -> bool {
    name != "代理人方案培养" && name != "自定义模板" && canonical_name(category, name) == Some(name)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTransportArrival {
    Simulation(SimulationView),
    Training(TrainingArrival),
}
impl AgentTransportArrival {
    fn accepts(&self, category: &str, target: &AgentTransportTarget) -> bool {
        let valid_charge =
            |current, required| current <= 1_000_000 && (1..=1000).contains(&required);
        match self {
            Self::Simulation(v) => {
                category == "实战模拟室" && crate::compendium::TransportFlow::arrival(v)
            }
            Self::Training(TrainingArrival::PatrolDetail { current, required }) => {
                category == "区域巡防" && valid_charge(*current, *required)
            }
            Self::Training(TrainingArrival::ExpertDetail { current, required }) => {
                category == "专业挑战室" && valid_charge(*current, *required)
            }
            Self::Training(TrainingArrival::HuntList) => category == "恶名狩猎",
            Self::Training(TrainingArrival::HuntDetail { mission_type }) => {
                category == "恶名狩猎"
                    && canonical_name(category, mission_type) == Some(target.mission_type.as_str())
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTransportView {
    Unknown,
    Blocked,
    World,
    Handbook {
        training: bool,
        category_visible: bool,
        category_selected: bool,
        types: Vec<String>,
        candidate: Option<AgentTransportTarget>,
    },
    Confirm,
    Arrival(AgentTransportArrival),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTransportReceipt {
    pub plan: ChargePlanItem,
    pub target: AgentTransportTarget,
    pub arrival: AgentTransportArrival,
    pub selection_verified: bool,
}
impl AgentTransportReceipt {
    /// 只解析本次具体目标；调用方仍须核对真实导航对象及控制边界。
    pub fn resolved_plan(&self) -> Result<ChargePlanItem, &'static str> {
        self.validate()?;
        if self.plan.category_name == "实战模拟室" {
            return Err("模拟室代理人关卡仍须完成独立目标选择");
        }
        let mut plan = self.plan.clone();
        plan.mission_type_name = self.target.mission_type.clone();
        plan.mission_name = None;
        plan.validate()?;
        Ok(plan)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        AgentTransportRequest {
            plan: self.plan.clone(),
            timeout_ms: 1000,
        }
        .validate()?;
        self.target.validate(&self.plan.category_name)?;
        if self.selection_verified || !self.arrival.accepts(&self.plan.category_name, &self.target)
        {
            return Err("代理人传送回执不能证明选中，且须保留同次目标到达");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Inspect,
    Open,
    Training,
    Category,
    Search,
    Go,
    Confirm,
    Arrival,
    Done,
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
pub struct AgentTransportFlow {
    request: AgentTransportRequest,
    phase: Phase,
    pending: Option<(AgentTransportAction, Receipt)>,
    scrolls: u8,
    target: Option<AgentTransportTarget>,
    receipt: Option<AgentTransportReceipt>,
}
impl AgentTransportFlow {
    pub fn new(request: AgentTransportRequest) -> Result<Self, &'static str> {
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
    pub fn request(&self) -> &AgentTransportRequest {
        &self.request
    }
    pub fn target(&self) -> Option<&AgentTransportTarget> {
        self.target.as_ref()
    }
    pub fn receipt(&self) -> Option<&AgentTransportReceipt> {
        self.receipt.as_ref()
    }
    pub fn completed(&self) -> bool {
        self.phase == Phase::Done
    }
    pub fn unavailable(&self) -> bool {
        self.phase == Phase::Unavailable
    }
    pub fn awaiting_arrival(&self) -> bool {
        matches!(self.phase, Phase::Arrival | Phase::Done)
            || self
                .pending
                .is_some_and(|(a, _)| a == AgentTransportAction::Confirm)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if let Some(t) = &self.target {
            t.validate(&self.request.plan.category_name)?;
        }
        if let Some(r) = &self.receipt {
            r.validate()?;
        }
        if self.scrolls > 10
            || self.completed() != self.receipt.is_some()
            || self.target.is_some()
                != matches!(
                    self.phase,
                    Phase::Go | Phase::Confirm | Phase::Arrival | Phase::Done
                )
            || self.pending.is_some_and(|(a, _)| !self.phase_allows(a))
            || self.unavailable() && (self.scrolls != 10 || self.pending.is_some())
            || self.receipt.as_ref().is_some_and(|r| {
                r.plan != self.request.plan || Some(&r.target) != self.target.as_ref()
            })
        {
            return Err("代理人传送检查点与目标、意图或到达不一致");
        }
        Ok(())
    }
    fn phase_allows(&self, a: AgentTransportAction) -> bool {
        use AgentTransportAction as A;
        matches!(
            (self.phase, a),
            (Phase::Open, A::Open)
                | (Phase::Training, A::Training)
                | (Phase::Category, A::Category)
                | (Phase::Go, A::Go)
                | (Phase::Confirm | Phase::Arrival, A::Confirm)
        ) || matches!((self.phase,a),(Phase::Search,A::Scroll(n)) if n == self.scrolls && n < 10)
    }
    fn valid_types(&self, types: &[String]) -> bool {
        let Some(cat) = catalog()
            .iter()
            .find(|c| c.category_name == self.request.plan.category_name)
        else {
            return false;
        };
        !types.is_empty()
            && types.len() <= cat.mission_type_list.len()
            && types.iter().collect::<BTreeSet<_>>().len() == types.len()
            && types
                .iter()
                .all(|t| valid_mission(&self.request.plan.category_name, t))
    }
    fn candidate_valid(&self, types: &[String], candidate: &AgentTransportTarget) -> bool {
        self.valid_types(types)
            && candidate.validate(&self.request.plan.category_name).is_ok()
            && types.contains(&candidate.mission_type)
    }
    fn page_allows(&self, a: AgentTransportAction, v: &AgentTransportView) -> bool {
        use AgentTransportAction as A;
        use AgentTransportView as V;
        match (a, v) {
            (A::Open, V::World) | (A::Confirm, V::Confirm) => true,
            (
                A::Training,
                V::Handbook {
                    training: false, ..
                },
            ) => true,
            (
                A::Category,
                V::Handbook {
                    training: true,
                    category_visible: true,
                    category_selected: false,
                    ..
                },
            ) => true,
            (
                A::Go,
                V::Handbook {
                    training: true,
                    category_selected: true,
                    types,
                    candidate: Some(c),
                    ..
                },
            ) => self.candidate_valid(types, c) && self.target.as_ref() == Some(c),
            (
                A::Scroll(_),
                V::Handbook {
                    training: true,
                    category_selected: true,
                    types,
                    candidate: None,
                    ..
                },
            ) => self.valid_types(types),
            _ => false,
        }
    }
    pub fn next(
        &mut self,
        v: &AgentTransportView,
    ) -> Result<Option<AgentTransportAction>, &'static str> {
        self.validate()?;
        use AgentTransportAction as A;
        use AgentTransportView as V;
        if self.completed() || self.unavailable() {
            return Ok(None);
        }
        if *v == V::Blocked {
            return Err("代理人传送出现不支持的弹窗或画面");
        }
        if self.phase == Phase::Inspect {
            self.phase = match v {
                V::World => Phase::Open,
                V::Handbook { .. } => Phase::Training,
                V::Confirm | V::Arrival(_) => return Err("不能采用既有代理人传送确认或副本入口"),
                _ => return Ok(None),
            };
        }
        if let Some((a, receipt)) = self.pending {
            if receipt != Receipt::Posted {
                return Err("代理人传送尚未确认输入投递");
            }
            let next = match (a, v) {
                (A::Open, V::Handbook { .. }) => Some(Phase::Training),
                (A::Training, V::Handbook { training: true, .. }) => Some(Phase::Category),
                (
                    A::Category,
                    V::Handbook {
                        training: true,
                        category_selected: true,
                        types,
                        ..
                    },
                ) if self.valid_types(types) => Some(Phase::Search),
                (A::Go, V::Confirm) => Some(Phase::Confirm),
                (A::Confirm, V::Arrival(arrival)) => {
                    let target = self.target.as_ref().ok_or("本次代理人目标缺失")?;
                    if !arrival.accepts(&self.request.plan.category_name, target) {
                        return Err("代理人传送到达与本次分类或具体目标不符");
                    }
                    self.receipt = Some(AgentTransportReceipt {
                        plan: self.request.plan.clone(),
                        target: target.clone(),
                        arrival: arrival.clone(),
                        selection_verified: false,
                    });
                    Some(Phase::Done)
                }
                _ => None,
            };
            let Some(next) = next else {
                return Ok(None);
            };
            self.phase = next;
            self.pending = None;
        }
        if self.completed() {
            self.validate()?;
            return Ok(None);
        }
        if self.phase == Phase::Training && matches!(v, V::Handbook { training: true, .. }) {
            self.phase = Phase::Category;
        }
        if self.phase == Phase::Category
            && matches!(v,V::Handbook{training:true,category_selected:true,types,..} if self.valid_types(types))
        {
            self.phase = Phase::Search;
        }
        if self.phase == Phase::Search {
            if let V::Handbook {
                training: true,
                category_selected: true,
                types,
                candidate: Some(c),
                ..
            } = v
                && self.candidate_valid(types, c)
            {
                self.target = Some(c.clone());
                self.phase = Phase::Go;
            }
            if self.phase == Phase::Search && self.scrolls == 10 {
                if matches!(v,V::Handbook{training:true,category_selected:true,types,candidate:None,..} if self.valid_types(types))
                {
                    self.phase = Phase::Unavailable;
                }
                self.validate()?;
                return Ok(None);
            }
        }
        let a = match self.phase {
            Phase::Open => A::Open,
            Phase::Training => A::Training,
            Phase::Category => A::Category,
            Phase::Search => A::Scroll(self.scrolls),
            Phase::Go => A::Go,
            Phase::Confirm => A::Confirm,
            _ => return Ok(None),
        };
        self.validate()?;
        Ok(self.page_allows(a, v).then_some(a))
    }
    pub fn prepare(&mut self, a: AgentTransportAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some() || !self.phase_allows(a) || self.phase == Phase::Arrival {
            return Err("代理人传送没有待保存的新动作");
        }
        self.pending = Some((a, Receipt::Intent));
        self.validate()
    }
    pub fn allows(&self, a: AgentTransportAction, v: &AgentTransportView) -> bool {
        self.validate().is_ok()
            && self.pending == Some((a, Receipt::Intent))
            && self.page_allows(a, v)
    }
    pub fn posted(&mut self, a: AgentTransportAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending != Some((a, Receipt::Intent)) {
            return Err("代理人传送投递回执不匹配");
        }
        match a {
            AgentTransportAction::Scroll(_) => {
                self.scrolls += 1;
                self.pending = None;
            }
            AgentTransportAction::Confirm => {
                self.phase = Phase::Arrival;
                self.pending = Some((a, Receipt::Posted));
            }
            _ => self.pending = Some((a, Receipt::Posted)),
        }
        self.validate()
    }
    pub fn not_posted(&mut self, a: AgentTransportAction) -> Result<(), &'static str> {
        if self.pending != Some((a, Receipt::Intent)) {
            return Err("已投递的代理人传送动作不能撤销");
        }
        self.pending = None;
        if a == AgentTransportAction::Go {
            self.target = None;
            self.phase = Phase::Search;
        }
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some_and(|(_, r)| r == Receipt::Intent) {
            return Err("代理人传送输入结果未确认，不能继续或重复投递");
        }
        if self.completed() {
            self.receipt = None;
            self.phase = Phase::Arrival;
            self.pending = Some((AgentTransportAction::Confirm, Receipt::Posted));
        }
        self.validate()
    }
}
