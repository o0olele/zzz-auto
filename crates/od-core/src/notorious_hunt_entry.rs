//! 免费周常详情与奖励余量的内部交接，不提供消费或完成证明。
use crate::{
    charge_plan::ChargePlanItem,
    charge_transport::{TrainingArrival, TrainingTransportReceipt, TrainingTransportRequest},
    deep_hunt::same_mission,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyEntryRequest {
    pub plan: ChargePlanItem,
    pub timeout_ms: u32,
}
impl WeeklyEntryRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        TrainingTransportRequest {
            plan: self.plan.clone(),
            timeout_ms: self.timeout_ms,
        }
        .validate()?;
        crate::notorious_hunt::NotoriousHuntConfig {
            plan_list: vec![self.plan.clone()],
            ..Default::default()
        }
        .validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyEntryView {
    Unknown,
    Blocked,
    List {
        matches: u8,
    },
    Detail {
        mission_type: String,
        back: bool,
        rewards: Option<u8>,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyEntryAction {
    Back,
    Select,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyEntryStage {
    Inspect,
    AwaitList,
    AwaitDetail,
    Observed,
    AwaitingVerification,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Delivery {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyEntryFlow {
    request: WeeklyEntryRequest,
    route: TrainingTransportReceipt,
    stage: WeeklyEntryStage,
    back: Option<Delivery>,
    select: Option<Delivery>,
    candidate: Option<u8>,
    stable: u8,
    unknown: u8,
}
impl WeeklyEntryFlow {
    pub fn new(
        request: WeeklyEntryRequest,
        route: TrainingTransportReceipt,
    ) -> Result<Self, &'static str> {
        let flow = Self {
            request,
            route,
            stage: WeeklyEntryStage::Inspect,
            back: None,
            select: None,
            candidate: None,
            stable: 0,
            unknown: 0,
        };
        flow.validate()?;
        Ok(flow)
    }
    pub fn request(&self) -> &WeeklyEntryRequest {
        &self.request
    }
    pub fn stage(&self) -> WeeklyEntryStage {
        self.stage
    }
    pub fn rewards(&self) -> Option<u8> {
        (self.stage == WeeklyEntryStage::Observed)
            .then_some(self.candidate)
            .flatten()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        use WeeklyEntryStage as S;
        self.request.validate()?;
        if self.route.category_name != "恶名狩猎"
            || self.route.selection_verified
            || !same_mission(
                &self.request.plan.mission_type_name,
                &self.route.mission_type,
            )
            || !matches!(
                &self.route.arrival,
                TrainingArrival::HuntList | TrainingArrival::HuntDetail { .. }
            )
            || matches!(&self.route.arrival, TrainingArrival::HuntDetail { mission_type } if crate::charge_transport::canonical_name("恶名狩猎", mission_type).is_none())
            || self.candidate.is_some_and(|n| n > 3)
            || self.stable > 2
            || self.unknown > 3
            || self.stable > 0 && self.candidate.is_none()
            || self.stage == S::AwaitList && self.back.is_none()
            || self.stage == S::AwaitDetail && self.select.is_none()
            || self.stage == S::Observed && (self.stable != 2 || self.candidate.is_none())
        {
            return Err("周常入口的目标、路径、意图或连续余量证明无效");
        }
        Ok(())
    }
    fn uncertain(&mut self) {
        self.candidate = None;
        self.stable = 0;
        self.unknown = (self.unknown + 1).min(3);
        if self.unknown == 3 {
            self.stage = WeeklyEntryStage::AwaitingVerification;
        }
    }
    pub fn next(
        &mut self,
        view: &WeeklyEntryView,
    ) -> Result<Option<WeeklyEntryAction>, &'static str> {
        use WeeklyEntryStage as S;
        self.validate()?;
        if matches!(self.stage, S::Observed | S::AwaitingVerification) {
            return Ok(None);
        }
        match view {
            WeeklyEntryView::List { matches: 1 } if self.stage != S::AwaitDetail => {
                self.candidate = None;
                self.stable = 0;
                self.unknown = 0;
                if self.select.is_none() {
                    return Ok(Some(WeeklyEntryAction::Select));
                }
            }
            WeeklyEntryView::Detail {
                mission_type,
                back: true,
                ..
            } if !same_mission(&self.request.plan.mission_type_name, mission_type)
                && crate::charge_transport::canonical_name("恶名狩猎", mission_type).is_some()
                && self.stage == S::Inspect
                && self.back.is_none()
                && self.select.is_none() =>
            {
                self.candidate = None;
                self.stable = 0;
                self.unknown = 0;
                return Ok(Some(WeeklyEntryAction::Back));
            }
            WeeklyEntryView::Detail {
                mission_type,
                rewards: Some(n @ 0..=3),
                ..
            } if same_mission(&self.request.plan.mission_type_name, mission_type)
                && self.stage != S::AwaitList =>
            {
                self.unknown = 0;
                if self.candidate == Some(*n) {
                    self.stable = (self.stable + 1).min(2);
                } else {
                    self.candidate = Some(*n);
                    self.stable = 1;
                }
                if self.stable == 2 {
                    self.stage = S::Observed;
                }
                return Ok(None);
            }
            _ => {}
        }
        self.uncertain();
        Ok(None)
    }
    pub fn prepare(&mut self, action: WeeklyEntryAction) -> Result<(), &'static str> {
        self.validate()?;
        let allowed = match action {
            WeeklyEntryAction::Back => {
                self.stage == WeeklyEntryStage::Inspect
                    && self.back.is_none()
                    && self.select.is_none()
            }
            WeeklyEntryAction::Select => {
                matches!(
                    self.stage,
                    WeeklyEntryStage::Inspect | WeeklyEntryStage::AwaitList
                ) && self.select.is_none()
            }
        };
        if !allowed {
            return Err("周常入口不能重复或越阶段保存输入意图");
        }
        match action {
            WeeklyEntryAction::Back => {
                self.back = Some(Delivery::Intent);
                self.stage = WeeklyEntryStage::AwaitList;
            }
            WeeklyEntryAction::Select => {
                self.select = Some(Delivery::Intent);
                self.stage = WeeklyEntryStage::AwaitDetail;
            }
        }
        self.candidate = None;
        self.stable = 0;
        self.unknown = 0;
        Ok(())
    }
    pub fn allows(&self, action: WeeklyEntryAction, view: &WeeklyEntryView) -> bool {
        if self.validate().is_err() {
            return false;
        }
        match (action, view) {
            (
                WeeklyEntryAction::Back,
                WeeklyEntryView::Detail {
                    mission_type,
                    back: true,
                    ..
                },
            ) => {
                self.stage == WeeklyEntryStage::AwaitList
                    && self.back == Some(Delivery::Intent)
                    && !same_mission(&self.request.plan.mission_type_name, mission_type)
                    && crate::charge_transport::canonical_name("恶名狩猎", mission_type).is_some()
            }
            (WeeklyEntryAction::Select, WeeklyEntryView::List { matches: 1 }) => {
                self.stage == WeeklyEntryStage::AwaitDetail && self.select == Some(Delivery::Intent)
            }
            _ => false,
        }
    }
    pub fn posted(&mut self, action: WeeklyEntryAction) -> Result<(), &'static str> {
        let receipt = match action {
            WeeklyEntryAction::Back => &mut self.back,
            WeeklyEntryAction::Select => &mut self.select,
        };
        if *receipt != Some(Delivery::Intent) {
            return Err("周常入口输入未保存意图");
        }
        *receipt = Some(Delivery::Posted);
        Ok(())
    }
    pub fn not_posted(&mut self, action: WeeklyEntryAction) -> Result<(), &'static str> {
        let receipt = match action {
            WeeklyEntryAction::Back => &mut self.back,
            WeeklyEntryAction::Select => &mut self.select,
        };
        if *receipt != Some(Delivery::Intent) {
            return Err("不能撤销已投递的周常入口动作");
        }
        *receipt = None;
        self.stage = if self.back.is_some() {
            WeeklyEntryStage::AwaitList
        } else {
            WeeklyEntryStage::Inspect
        };
        self.candidate = None;
        self.stable = 0;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.candidate = None;
        self.stable = 0;
        self.unknown = 0;
        if self.stage == WeeklyEntryStage::Observed {
            self.stage = WeeklyEntryStage::Inspect;
        }
        Ok(())
    }
}
