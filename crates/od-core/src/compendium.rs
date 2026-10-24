//! 实战模拟室的快捷手册传送。到达不代表关卡、卡片或战斗已经完成。
use crate::{charge_plan::catalog, simulation_cards::SimulationView};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportRequest {
    pub mission_type: String,
    pub timeout_ms: u32,
}
impl TransportRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1000..=900_000).contains(&self.timeout_ms)
            || !catalog()[0]
                .mission_type_list
                .iter()
                .any(|m| m.mission_type_name == self.mission_type)
            || self.mission_type == "代理人方案培养"
        {
            return Err("模拟室传送类型或超时无效；代理人方案培养尚未接入");
        }
        Ok(())
    }
    pub fn transport_type(&self) -> &str {
        if self.mission_type == "自定义模板" {
            "基础材料"
        } else {
            &self.mission_type
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportView {
    Unknown,
    Blocked,
    World,
    Handbook {
        training: bool,
        simulation_category: bool,
        types: Vec<String>,
        target_go: bool,
    },
    Confirm,
    Simulation(SimulationView),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportAction {
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportFlow {
    request: TransportRequest,
    phase: Phase,
    pending: Option<TransportAction>,
    scrolls: u8,
    destination: Option<SimulationView>,
}
impl TransportFlow {
    pub fn new(request: TransportRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            phase: Phase::Inspect,
            pending: None,
            scrolls: 0,
            destination: None,
        })
    }
    pub fn request(&self) -> &TransportRequest {
        &self.request
    }
    pub fn completed(&self) -> bool {
        self.phase == Phase::Done
    }
    pub fn destination(&self) -> Option<&SimulationView> {
        self.destination.as_ref()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.scrolls > 20
            || self.completed() != self.destination.is_some()
            || self.destination.as_ref().is_some_and(|v| !Self::arrival(v))
            || self.pending.is_some_and(|a| !self.phase_allows(a))
        {
            return Err("快捷手册传送检查点不一致");
        }
        Ok(())
    }
    fn arrival(view: &SimulationView) -> bool {
        match view {
            SimulationView::Challenge {
                current_charge,
                required_charge,
            } => crate::simulation_cards::CardsCharge {
                current_charge: *current_charge,
                required_charge: *required_charge,
            }
            .validate()
            .is_ok(),
            SimulationView::Categories { names } => {
                let known = &catalog()[0].mission_type_list;
                names.len() >= 3
                    && names.len() <= known.len()
                    && names
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == names.len()
                    && names
                        .iter()
                        .all(|n| known.iter().any(|m| m.mission_type_name == *n))
            }
            _ => false,
        }
    }
    fn phase_allows(&self, a: TransportAction) -> bool {
        matches!(
            (self.phase, a),
            (Phase::Open, TransportAction::Open)
                | (Phase::Training, TransportAction::Training)
                | (Phase::Category, TransportAction::Category)
                | (Phase::Go, TransportAction::Go)
                | (Phase::Confirm, TransportAction::Confirm)
        ) || matches!((self.phase, a), (Phase::Search, TransportAction::Scroll(n)) if n == self.scrolls && n < 20)
    }
    pub fn next(&mut self, view: &TransportView) -> Result<Option<TransportAction>, &'static str> {
        self.validate()?;
        // 只凭已登记的传送确认，才能将目的地作为父流程的到达证明。
        if self.phase == Phase::Arrival {
            if let TransportView::Simulation(v) = view {
                if Self::arrival(v) {
                    self.destination = Some(v.clone());
                    self.phase = Phase::Done;
                    self.pending = None;
                } else if !matches!(v, SimulationView::Unknown | SimulationView::Blocked) {
                    return Err("传送后不是可交接的模拟室入口");
                }
            }
            return Ok(None);
        }
        if self.completed() {
            return Ok(None);
        }
        if self.phase == Phase::Inspect {
            match view {
                TransportView::World => self.phase = Phase::Open,
                TransportView::Handbook { .. } => self.phase = Phase::Training,
                TransportView::Confirm | TransportView::Simulation(_) => {
                    return Err("缺少本次传送意图，不能采用已有弹窗或副本");
                }
                _ => return Ok(None),
            }
        }
        // 打开和切页的回执不等于页面已改变；只在下一张图上推进。
        if self.pending == Some(TransportAction::Open)
            && matches!(view, TransportView::Handbook { .. })
        {
            self.pending = None;
            self.phase = Phase::Training;
        }
        if self.pending == Some(TransportAction::Training)
            && matches!(view, TransportView::Handbook { training: true, .. })
        {
            self.pending = None;
            self.phase = Phase::Category;
        }
        if self.pending == Some(TransportAction::Go) && *view == TransportView::Confirm {
            self.pending = None;
            self.phase = Phase::Confirm;
        }
        if self.pending.is_some() {
            return Ok(None);
        }
        if self.phase == Phase::Search {
            if let TransportView::Handbook {
                training: true,
                target_go: true,
                ..
            } = view
            {
                self.phase = Phase::Go;
            } else if self.scrolls == 20 {
                return Err("快捷手册查找已达到 20 次滚动上限");
            }
        }
        let a = match self.phase {
            Phase::Open => TransportAction::Open,
            Phase::Training => TransportAction::Training,
            Phase::Category => TransportAction::Category,
            Phase::Search => TransportAction::Scroll(self.scrolls),
            Phase::Go => TransportAction::Go,
            Phase::Confirm => TransportAction::Confirm,
            _ => return Ok(None),
        };
        Ok(self.page_allows(a, view).then_some(a))
    }
    fn page_allows(&self, a: TransportAction, v: &TransportView) -> bool {
        match (a, v) {
            (TransportAction::Open, TransportView::World)
            | (TransportAction::Confirm, TransportView::Confirm) => true,
            (TransportAction::Training, TransportView::Handbook { .. }) => true,
            (
                TransportAction::Category,
                TransportView::Handbook {
                    training: true,
                    simulation_category: true,
                    ..
                },
            ) => true,
            (
                TransportAction::Go,
                TransportView::Handbook {
                    training: true,
                    target_go: true,
                    ..
                },
            ) => true,
            (
                TransportAction::Scroll(_),
                TransportView::Handbook {
                    training: true,
                    types,
                    ..
                },
            ) => !types.is_empty(),
            _ => false,
        }
    }
    pub fn prepare(&mut self, a: TransportAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some() || !self.phase_allows(a) {
            return Err("传送动作不是当前步骤");
        }
        self.pending = Some(a);
        Ok(())
    }
    pub fn allows(&self, a: TransportAction, v: &TransportView) -> bool {
        self.validate().is_ok() && self.pending == Some(a) && self.page_allows(a, v)
    }
    pub fn posted(&mut self, a: TransportAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending != Some(a) {
            return Err("传送投递回执不匹配");
        }
        match a {
            TransportAction::Open | TransportAction::Training | TransportAction::Go => {
                return Ok(());
            }
            TransportAction::Category => self.phase = Phase::Search,
            TransportAction::Scroll(_) => self.scrolls += 1,
            TransportAction::Confirm => self.phase = Phase::Arrival,
        }
        self.pending = None;
        self.validate()
    }
    pub fn not_posted(&mut self, a: TransportAction) -> Result<(), &'static str> {
        if self.pending != Some(a) {
            return Err("传送未投递回执不匹配");
        }
        self.pending = None;
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        match self.pending {
            Some(TransportAction::Confirm) => {
                self.phase = Phase::Arrival;
                self.pending = None;
            }
            Some(TransportAction::Category | TransportAction::Scroll(_)) => {
                self.phase = Phase::Category;
                self.pending = None;
            }
            _ => {}
        }
        if self.pending.is_none() && matches!(self.phase, Phase::Search | Phase::Go) {
            self.phase = Phase::Category;
        }
        if self.completed() {
            self.phase = Phase::Arrival;
            self.destination = None;
        }
        self.validate()
    }
}
