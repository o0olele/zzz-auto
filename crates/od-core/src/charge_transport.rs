//! 区域巡防、专业挑战与恶名狩猎的传送；到达不证明消费准备完成。
use crate::charge_plan::{ChargePlanItem, catalog, execution_mission_type};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};
pub fn normalized_name(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_whitespace() && !['·', '・', '•', '「', '」', '『', '』', '“', '”'].contains(c)
        })
        .collect()
}
pub fn canonical_name(category: &str, text: &str) -> Option<&'static str> {
    type AliasMap = BTreeMap<String, Vec<(String, Vec<String>)>>;
    static ALIASES: OnceLock<AliasMap> = OnceLock::new();
    let aliases = ALIASES.get_or_init(|| {
        #[derive(Deserialize)]
        struct Tab {
            #[serde(default)]
            category_list: Vec<Category>,
        }
        #[derive(Deserialize)]
        struct Category {
            category_name: String,
            #[serde(default)]
            mission_type_list: Vec<Mission>,
        }
        #[derive(Deserialize)]
        struct Mission {
            mission_type_name: String,
            #[serde(default)]
            alias_list: Vec<String>,
        }
        serde_yaml::from_str::<Vec<Tab>>(include_str!(
            "../../../resources/charge-plan/compendium.yml"
        ))
        .expect("固定训练目录有效")
        .into_iter()
        .flat_map(|t| t.category_list)
        .map(|c| {
            (
                c.category_name,
                c.mission_type_list
                    .into_iter()
                    .map(|m| (m.mission_type_name, m.alias_list))
                    .collect(),
            )
        })
        .collect()
    });
    let text = normalized_name(execution_mission_type(category, text));
    let found: Vec<_> = aliases
        .get(category)?
        .iter()
        .filter(|(name, list)| {
            normalized_name(name) == text || list.iter().any(|a| normalized_name(a) == text)
        })
        .collect();
    if found.len() != 1 {
        return None;
    }
    catalog()
        .iter()
        .find(|c| c.category_name == category)?
        .mission_type_list
        .iter()
        .find(|m| m.mission_type_name == found[0].0)
        .map(|m| m.mission_type_name.as_str())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingTransportRequest {
    pub plan: ChargePlanItem,
    pub timeout_ms: u32,
}
impl TrainingTransportRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.plan.validate()?;
        if !(1000..=900_000).contains(&self.timeout_ms)
            || !["区域巡防", "专业挑战室", "恶名狩猎"].contains(&self.plan.category_name.as_str())
            || self.plan.execution_mission_type() == "代理人方案培养"
            || self.plan.mission_name.is_some()
        {
            return Err(
                "普通训练传送仅支持区域巡防、专业挑战室和恶名狩猎；代理人方案培养须独立处理",
            );
        }
        Ok(())
    }
    pub fn mission_type(&self) -> &str {
        self.plan.execution_mission_type()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingArrival {
    PatrolDetail { current: u32, required: u32 },
    ExpertDetail { current: u32, required: u32 },
    HuntList,
    HuntDetail { mission_type: String },
}
impl TrainingArrival {
    fn accepts(&self, r: &TrainingTransportRequest) -> bool {
        match self {
            Self::PatrolDetail { current, required } => {
                r.plan.category_name == "区域巡防"
                    && *current <= 1_000_000
                    && (1..=1000).contains(required)
            }
            Self::ExpertDetail { current, required } => {
                r.plan.category_name == "专业挑战室"
                    && *current <= 1_000_000
                    && (1..=1000).contains(required)
            }
            Self::HuntList => r.plan.category_name == "恶名狩猎",
            Self::HuntDetail { mission_type } => {
                r.plan.category_name == "恶名狩猎"
                    && canonical_name("恶名狩猎", mission_type).is_some()
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingTransportView {
    Unknown,
    Blocked,
    World,
    Handbook {
        training: bool,
        category_visible: bool,
        category_selected: bool,
        types: Vec<String>,
        target_go: bool,
    },
    Confirm,
    Arrival(TrainingArrival),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingTransportAction {
    Open,
    Training,
    Category,
    Scroll(u8),
    Go,
    Confirm,
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
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Receipt {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingTransportReceipt {
    pub category_name: String,
    pub mission_type: String,
    pub arrival: TrainingArrival,
    pub selection_verified: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingTransportFlow {
    request: TrainingTransportRequest,
    phase: Phase,
    pending: Option<(TrainingTransportAction, Receipt)>,
    scrolls: u8,
    receipt: Option<TrainingTransportReceipt>,
}
impl TrainingTransportFlow {
    /// 语义转换本次代理人到达；此纯核心方法本身不证明原生输入所有权。
    pub fn from_agent_route(
        request: TrainingTransportRequest,
        agent: &crate::agent_transport::AgentTransportReceipt,
    ) -> Result<Self, &'static str> {
        request.validate()?;
        agent.validate()?;
        if request.plan != agent.resolved_plan()? {
            return Err("具体挑战目标与原代理人导航不一致");
        }
        let crate::agent_transport::AgentTransportArrival::Training(arrival) = &agent.arrival
        else {
            return Err("普通训练挑战不能采用模拟室到达");
        };
        let receipt = TrainingTransportReceipt {
            category_name: request.plan.category_name.clone(),
            mission_type: request.mission_type().into(),
            arrival: arrival.clone(),
            selection_verified: false,
        };
        let flow = Self {
            request,
            phase: Phase::Done,
            pending: None,
            scrolls: 0,
            receipt: Some(receipt),
        };
        flow.validate()?;
        Ok(flow)
    }
    pub fn new(request: TrainingTransportRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            phase: Phase::Inspect,
            pending: None,
            scrolls: 0,
            receipt: None,
        })
    }
    pub fn request(&self) -> &TrainingTransportRequest {
        &self.request
    }
    pub fn receipt(&self) -> Option<&TrainingTransportReceipt> {
        self.receipt.as_ref()
    }
    pub fn completed(&self) -> bool {
        self.phase == Phase::Done
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.scrolls > 20
            || self.completed() != self.receipt.is_some()
            || self.pending.is_some_and(|(a, _)| !self.phase_allows(a))
            || self.receipt.as_ref().is_some_and(|r| {
                r.selection_verified
                    || r.category_name != self.request.plan.category_name
                    || r.mission_type != self.request.mission_type()
                    || !r.arrival.accepts(&self.request)
            })
        {
            return Err("训练副本传送检查点不一致");
        }
        Ok(())
    }
    fn phase_allows(&self, a: TrainingTransportAction) -> bool {
        use TrainingTransportAction as A;
        matches!(
            (self.phase, a),
            (Phase::Open, A::Open)
                | (Phase::Training, A::Training)
                | (Phase::Category, A::Category)
                | (Phase::Go, A::Go)
                | (Phase::Confirm | Phase::Arrival, A::Confirm)
        ) || matches!((self.phase,a),(Phase::Search,A::Scroll(n)) if n==self.scrolls&&n<20)
    }
    fn valid_types(&self, types: &[String]) -> bool {
        let cat = catalog()
            .iter()
            .find(|c| c.category_name == self.request.plan.category_name)
            .expect("已验证分类");
        !types.is_empty()
            && types.len() <= cat.mission_type_list.len()
            && types.iter().collect::<BTreeSet<_>>().len() == types.len()
            && types.iter().all(|t| {
                cat.mission_type_list
                    .iter()
                    .any(|m| m.mission_type_name == *t)
            })
    }
    fn page_allows(&self, a: TrainingTransportAction, v: &TrainingTransportView) -> bool {
        use TrainingTransportAction as A;
        use TrainingTransportView as V;
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
                    ..
                },
            ) => true,
            (
                A::Go,
                V::Handbook {
                    training: true,
                    category_selected: true,
                    types,
                    target_go: true,
                    ..
                },
            ) => self.valid_types(types) && types.iter().any(|t| t == self.request.mission_type()),
            (
                A::Scroll(_),
                V::Handbook {
                    training: true,
                    category_selected: true,
                    types,
                    target_go: false,
                    ..
                },
            ) => self.valid_types(types),
            _ => false,
        }
    }
    pub fn next(
        &mut self,
        v: &TrainingTransportView,
    ) -> Result<Option<TrainingTransportAction>, &'static str> {
        self.validate()?;
        use TrainingTransportAction as A;
        use TrainingTransportView as V;
        if self.completed() {
            return Ok(None);
        }
        if *v == V::Blocked {
            return Err("训练传送出现不支持的弹窗或战斗画面");
        }
        if self.phase == Phase::Inspect {
            self.phase = match v {
                V::World => Phase::Open,
                V::Handbook { .. } => Phase::Training,
                V::Confirm | V::Arrival(_) => {
                    return Err("缺少本次训练传送意图，不能采用已有弹窗或入口");
                }
                _ => return Ok(None),
            };
        }
        if let Some((action, _)) = self.pending {
            let progress = match (action, v) {
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
                (
                    A::Scroll(_),
                    V::Handbook {
                        training: true,
                        category_selected: true,
                        types,
                        target_go: true,
                        ..
                    },
                ) if self.valid_types(types) => Some(Phase::Search),
                (A::Confirm, V::Arrival(arrival)) => {
                    if !arrival.accepts(&self.request) {
                        return Err("训练传送后出现其他副本入口");
                    }
                    self.receipt = Some(TrainingTransportReceipt {
                        category_name: self.request.plan.category_name.clone(),
                        mission_type: self.request.mission_type().into(),
                        arrival: arrival.clone(),
                        selection_verified: false,
                    });
                    Some(Phase::Done)
                }
                _ => None,
            };
            if let Some(p) = progress {
                self.phase = p;
                self.pending = None;
            } else {
                return Ok(None);
            }
        }
        if self.completed() {
            return Ok(None);
        }
        if self.phase == Phase::Training && matches!(v, V::Handbook { training: true, .. }) {
            self.phase = Phase::Category;
        }
        if self.phase == Phase::Category
            && matches!(v,V::Handbook {training:true,category_selected:true,types,..} if self.valid_types(types))
        {
            self.phase = Phase::Search;
        }
        if self.phase == Phase::Search && self.page_allows(A::Go, v) {
            self.phase = Phase::Go;
        }
        if self.phase == Phase::Search && self.scrolls >= 20 {
            return Err("训练副本查找达到 20 次滚动上限");
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
        Ok(self.page_allows(a, v).then_some(a))
    }
    pub fn prepare(&mut self, a: TrainingTransportAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some() || !self.phase_allows(a) || self.phase == Phase::Arrival {
            return Err("训练传送没有待保存的新动作");
        }
        self.pending = Some((a, Receipt::Intent));
        Ok(())
    }
    pub fn allows(&self, a: TrainingTransportAction, v: &TrainingTransportView) -> bool {
        self.validate().is_ok()
            && self.pending == Some((a, Receipt::Intent))
            && self.page_allows(a, v)
    }
    pub fn posted(&mut self, a: TrainingTransportAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending != Some((a, Receipt::Intent)) {
            return Err("训练传送投递回执不匹配");
        }
        match a {
            TrainingTransportAction::Scroll(_) => {
                self.scrolls += 1;
                self.pending = None;
            }
            TrainingTransportAction::Confirm => {
                self.phase = Phase::Arrival;
                self.pending = Some((a, Receipt::Posted));
            }
            _ => self.pending = Some((a, Receipt::Posted)),
        }
        self.validate()
    }
    pub fn not_posted(&mut self, a: TrainingTransportAction) -> Result<(), &'static str> {
        if self.pending != Some((a, Receipt::Intent)) {
            return Err("已投递训练传送不能撤销");
        }
        self.pending = None;
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.completed() {
            self.receipt = None;
            self.phase = Phase::Arrival;
            self.pending = Some((TrainingTransportAction::Confirm, Receipt::Posted));
        }
        Ok(())
    }
    pub fn awaiting_arrival(&self) -> bool {
        matches!(self.phase, Phase::Arrival | Phase::Done)
            || self
                .pending
                .is_some_and(|(a, _)| a == TrainingTransportAction::Confirm)
    }
}
