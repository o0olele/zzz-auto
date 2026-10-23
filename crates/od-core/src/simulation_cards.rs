//! 实战模拟室卡片准备；只证明卡片数量，不证明目标副本或战斗结果。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationView {
    Unknown,
    Blocked,
    Categories {
        names: Vec<String>,
    },
    Challenge {
        current_charge: u32,
        required_charge: u32,
    },
    Cards,
    Deployment,
    Settlement,
    Reward,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardsRequest {
    pub card_num: Option<u8>,
    pub timeout_ms: u32,
}
impl CardsRequest {
    pub fn validate(self) -> Result<(), &'static str> {
        if self.card_num.is_some_and(|n| !(1..=5).contains(&n))
            || !(1000..=900_000).contains(&self.timeout_ms)
        {
            return Err("卡片数量或准备超时无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardsAction {
    Open,
    Clear(u8),
    Add(u8),
    Save,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Inspect,
    Open,
    Edit,
    Verify,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardsFlow {
    request: CardsRequest,
    phase: Phase,
    cleared: u8,
    added: u8,
    pending: Option<CardsAction>,
    result: Option<CardsCharge>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardsCharge {
    pub current_charge: u32,
    pub required_charge: u32,
}
impl CardsCharge {
    pub fn validate(self) -> Result<(), &'static str> {
        if self.current_charge > 1_000_000
            || !(20..=100).contains(&self.required_charge)
            || !self.required_charge.is_multiple_of(20)
        {
            return Err("实战模拟室实际电量不可用");
        }
        Ok(())
    }
}
impl CardsFlow {
    pub fn new(request: CardsRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            phase: Phase::Inspect,
            cleared: 0,
            added: 0,
            pending: None,
            result: None,
        })
    }
    pub fn request(&self) -> CardsRequest {
        self.request
    }
    pub fn charge(&self) -> Option<CardsCharge> {
        self.result
    }
    pub fn completed(&self) -> bool {
        self.phase == Phase::Done
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.cleared > 5
            || self.added > self.request.card_num.unwrap_or(0)
            || (self.added > 0 && self.cleared != 5)
            || (self.result.is_some() != self.completed())
            || (self.completed() && self.pending.is_some())
            || (matches!(self.phase, Phase::Inspect | Phase::Open)
                && (self.cleared != 0 || self.added != 0))
            || (self.completed()
                && self
                    .request
                    .card_num
                    .is_some_and(|n| self.cleared != 5 || self.added != n))
            || (matches!(self.phase, Phase::Open | Phase::Edit | Phase::Verify)
                && self.request.card_num.is_none())
            || (self.phase == Phase::Verify
                && (self.cleared != 5 || Some(self.added) != self.request.card_num))
        {
            return Err("卡片准备检查点不一致");
        }
        if let Some(result) = self.result {
            result.validate()?;
            if self
                .request
                .card_num
                .is_some_and(|n| result.required_charge != u32::from(n) * 20)
            {
                return Err("卡片准备结果与冻结数量不符");
            }
        }
        if self.pending.is_some_and(|a| self.expected() != Some(a)) {
            return Err("卡片准备意图与阶段不符");
        }
        Ok(())
    }
    fn expected(&self) -> Option<CardsAction> {
        match self.phase {
            Phase::Open => Some(CardsAction::Open),
            Phase::Edit if self.cleared < 5 => Some(CardsAction::Clear(self.cleared)),
            Phase::Edit if self.added < self.request.card_num.unwrap_or(0) => {
                Some(CardsAction::Add(self.added))
            }
            Phase::Edit => Some(CardsAction::Save),
            _ => None,
        }
    }
    pub fn next(&mut self, view: &SimulationView) -> Result<Option<CardsAction>, &'static str> {
        self.validate()?;
        match (self.phase, view) {
            (
                Phase::Inspect,
                SimulationView::Challenge {
                    current_charge,
                    required_charge,
                },
            ) => {
                let charge = CardsCharge {
                    current_charge: *current_charge,
                    required_charge: *required_charge,
                };
                charge.validate()?;
                if self.request.card_num.is_none() {
                    self.result = Some(charge);
                    self.phase = Phase::Done;
                } else {
                    self.phase = Phase::Open;
                }
            }
            (Phase::Inspect, SimulationView::Cards) if self.request.card_num.is_some() => {
                self.phase = Phase::Edit;
            }
            (Phase::Open, SimulationView::Cards) => {
                self.pending = None;
                self.phase = Phase::Edit;
            }
            (
                Phase::Verify,
                SimulationView::Challenge {
                    current_charge,
                    required_charge,
                },
            ) => {
                let charge = CardsCharge {
                    current_charge: *current_charge,
                    required_charge: *required_charge,
                };
                charge.validate()?;
                if Some((charge.required_charge / 20) as u8) != self.request.card_num {
                    return Err("保存后的实际电量与冻结卡片数量不符");
                }
                self.result = Some(charge);
                self.phase = Phase::Done;
            }
            _ => {}
        }
        self.validate()?;
        Ok(self
            .expected()
            .filter(|a| self.pending.is_none() && self.page_allows(*a, view)))
    }
    fn page_allows(&self, action: CardsAction, view: &SimulationView) -> bool {
        match (action, view) {
            (
                CardsAction::Open,
                SimulationView::Challenge {
                    current_charge,
                    required_charge,
                },
            ) => CardsCharge {
                current_charge: *current_charge,
                required_charge: *required_charge,
            }
            .validate()
            .is_ok(),
            (
                CardsAction::Clear(_) | CardsAction::Add(_) | CardsAction::Save,
                SimulationView::Cards,
            ) => true,
            _ => false,
        }
    }
    pub fn prepare(&mut self, action: CardsAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some() || self.expected() != Some(action) {
            return Err("卡片动作不是当前步骤");
        }
        self.pending = Some(action);
        Ok(())
    }
    pub fn allows(&self, action: CardsAction, view: &SimulationView) -> bool {
        self.validate().is_ok() && self.pending == Some(action) && self.page_allows(action, view)
    }
    pub fn posted(&mut self, action: CardsAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending != Some(action) {
            return Err("卡片投递回执不匹配");
        }
        match action {
            // 打开后只等编辑页，不能在相同详情上重复点击。
            CardsAction::Open => return Ok(()),
            CardsAction::Clear(_) => self.cleared += 1,
            CardsAction::Add(_) => self.added += 1,
            CardsAction::Save => self.phase = Phase::Verify,
        }
        self.pending = None;
        self.validate()
    }
    pub fn not_posted(&mut self, action: CardsAction) -> Result<(), &'static str> {
        if self.pending != Some(action) {
            return Err("卡片未投递回执不匹配");
        }
        self.pending = None;
        self.validate()
    }
    /// 编辑操作可重做；暂停后重新清空，禁止续用旧计数。已保存方案继续核对实际电量。
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.completed() {
            // 完成证明尚未交给父流程就收到暂停时，继续也必须重读当前电量。
            self.result = None;
            self.phase = if self.request.card_num.is_some() {
                Phase::Verify
            } else {
                Phase::Inspect
            };
        }
        if !matches!(self.phase, Phase::Verify | Phase::Done) {
            if self.pending == Some(CardsAction::Save) {
                self.phase = Phase::Verify;
            } else {
                self.phase = Phase::Inspect;
                self.cleared = 0;
                self.added = 0;
            }
        }
        self.pending = None;
        self.validate()
    }
}
