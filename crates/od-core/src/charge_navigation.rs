//! 体力循环的已知页面导航；动作意图不能替代到达证明。
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeNavTarget {
    World,
    Handbook,
    SimulationCategory,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeNavView {
    Unknown,
    Blocked,
    World,
    Handbook {
        training: bool,
        simulation_category: bool,
        simulation_ready: bool,
    },
    SynthesisPage {
        back: bool,
    },
    SynthesisDetail {
        close: bool,
    },
    SimulationPage {
        back: bool,
    },
    PatrolPage {
        back: bool,
    },
    ExpertPage {
        back: bool,
    },
    HuntPage {
        back: bool,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeNavAction {
    OpenHandbook,
    ExitHandbook,
    Training,
    SimulationCategory,
    BackSynthesis,
    CloseSynthesisDetail,
    BackSimulation,
    BackPatrol,
    BackExpert,
    BackHunt,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Receipt {
    Intent,
    Posted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    action: ChargeNavAction,
    source: ChargeNavView,
    receipt: Receipt,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeNavFlow {
    target: ChargeNavTarget,
    timeout_ms: u32,
    attempts: u8,
    pending: Option<Pending>,
    ready: bool,
}
impl ChargeNavFlow {
    pub fn new(target: ChargeNavTarget, timeout_ms: u32) -> Result<Self, &'static str> {
        if !(1000..=300_000).contains(&timeout_ms) {
            return Err("体力导航时限无效");
        }
        Ok(Self {
            target,
            timeout_ms,
            attempts: 0,
            pending: None,
            ready: false,
        })
    }
    pub fn target(&self) -> ChargeNavTarget {
        self.target
    }
    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }
    pub fn ready(&self) -> bool {
        self.ready
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1000..=300_000).contains(&self.timeout_ms)
            || self.attempts > 32
            || self
                .pending
                .is_some_and(|p| self.attempts == 0 || !Self::allowed(p.action, p.source))
            || (self.ready && self.pending.is_some())
        {
            return Err("体力导航检查点无效");
        }
        Ok(())
    }
    fn reached(&self, v: ChargeNavView) -> bool {
        matches!(
            (self.target, v),
            (ChargeNavTarget::World, ChargeNavView::World)
                | (ChargeNavTarget::Handbook, ChargeNavView::Handbook { .. })
                | (
                    ChargeNavTarget::SimulationCategory,
                    ChargeNavView::Handbook {
                        training: true,
                        simulation_ready: true,
                        ..
                    },
                )
        )
    }
    fn same_page(a: ChargeNavView, b: ChargeNavView) -> bool {
        std::mem::discriminant(&a) == std::mem::discriminant(&b)
    }
    fn progressed(p: Pending, v: ChargeNavView) -> bool {
        match p.action {
            ChargeNavAction::Training => {
                matches!(v, ChargeNavView::Handbook { training: true, .. })
            }
            ChargeNavAction::SimulationCategory => matches!(
                v,
                ChargeNavView::Handbook {
                    training: true,
                    simulation_ready: true,
                    ..
                }
            ),
            _ => {
                !matches!(v, ChargeNavView::Unknown | ChargeNavView::Blocked)
                    && !Self::same_page(p.source, v)
            }
        }
    }
    pub fn next(&mut self, v: ChargeNavView) -> Result<Option<ChargeNavAction>, &'static str> {
        self.validate()?;
        if self.ready {
            return Ok(None);
        }
        if v == ChargeNavView::Blocked {
            return Err("体力导航出现未支持的弹窗或战斗画面");
        }
        if let Some(p) = self.pending {
            if !Self::progressed(p, v) {
                return Ok(None);
            }
            self.pending = None;
        }
        if self.reached(v) {
            self.ready = true;
            return Ok(None);
        }
        if self.attempts >= 32 {
            return Err("体力导航达到 32 次动作上限");
        }
        Ok(match v {
            ChargeNavView::World => Some(ChargeNavAction::OpenHandbook),
            ChargeNavView::Handbook { .. } if self.target == ChargeNavTarget::World => {
                Some(ChargeNavAction::ExitHandbook)
            }
            ChargeNavView::Handbook {
                training: false, ..
            } => Some(ChargeNavAction::Training),
            ChargeNavView::Handbook {
                training: true,
                simulation_category: true,
                simulation_ready: false,
            } => Some(ChargeNavAction::SimulationCategory),
            ChargeNavView::SynthesisPage { back: true } => Some(ChargeNavAction::BackSynthesis),
            ChargeNavView::SynthesisDetail { close: true } => {
                Some(ChargeNavAction::CloseSynthesisDetail)
            }
            ChargeNavView::SimulationPage { back: true } => Some(ChargeNavAction::BackSimulation),
            ChargeNavView::PatrolPage { back: true } => Some(ChargeNavAction::BackPatrol),
            ChargeNavView::ExpertPage { back: true } => Some(ChargeNavAction::BackExpert),
            ChargeNavView::HuntPage { back: true } => Some(ChargeNavAction::BackHunt),
            _ => None,
        })
    }
    fn allowed(a: ChargeNavAction, v: ChargeNavView) -> bool {
        matches!(
            (a, v),
            (ChargeNavAction::OpenHandbook, ChargeNavView::World)
                | (
                    ChargeNavAction::ExitHandbook,
                    ChargeNavView::Handbook { .. }
                )
                | (
                    ChargeNavAction::Training,
                    ChargeNavView::Handbook {
                        training: false,
                        ..
                    }
                )
                | (
                    ChargeNavAction::SimulationCategory,
                    ChargeNavView::Handbook {
                        training: true,
                        simulation_category: true,
                        simulation_ready: false
                    }
                )
                | (
                    ChargeNavAction::BackSynthesis,
                    ChargeNavView::SynthesisPage { back: true }
                )
                | (
                    ChargeNavAction::CloseSynthesisDetail,
                    ChargeNavView::SynthesisDetail { close: true }
                )
                | (
                    ChargeNavAction::BackSimulation,
                    ChargeNavView::SimulationPage { back: true }
                )
                | (
                    ChargeNavAction::BackPatrol,
                    ChargeNavView::PatrolPage { back: true }
                )
                | (
                    ChargeNavAction::BackExpert,
                    ChargeNavView::ExpertPage { back: true }
                )
                | (
                    ChargeNavAction::BackHunt,
                    ChargeNavView::HuntPage { back: true }
                )
        )
    }
    pub fn prepare(&mut self, a: ChargeNavAction, v: ChargeNavView) -> Result<(), &'static str> {
        self.validate()?;
        let mut expected = self.clone();
        if self.ready || self.pending.is_some() || expected.next(v)? != Some(a) {
            return Err("体力导航没有待保存的新动作");
        }
        self.attempts += 1;
        self.pending = Some(Pending {
            action: a,
            source: v,
            receipt: Receipt::Intent,
        });
        Ok(())
    }
    pub fn allows(&self, a: ChargeNavAction, v: ChargeNavView) -> bool {
        self.validate().is_ok()
            && !self.ready
            && self.pending.is_some_and(|p| {
                p.action == a && p.receipt == Receipt::Intent && Self::allowed(a, v)
            })
    }
    pub fn posted(&mut self, a: ChargeNavAction) -> Result<(), &'static str> {
        let p = self.pending.as_mut().ok_or("没有导航意图")?;
        if p.action != a || p.receipt != Receipt::Intent {
            return Err("导航投递不匹配");
        }
        p.receipt = Receipt::Posted;
        Ok(())
    }
    pub fn not_posted(&mut self, a: ChargeNavAction) -> Result<(), &'static str> {
        if !self
            .pending
            .is_some_and(|p| p.action == a && p.receipt == Receipt::Intent)
        {
            return Err("已投递导航不能撤销");
        }
        self.pending = None;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.ready = false;
        Ok(())
    }
}
