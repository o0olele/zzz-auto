//! 抽奖机与路径迭换：抽奖后态独立于点击次数，路径迭换只返回大世界。
use crate::{
    lost_void::{ArtifactSelection, LostVoidChallenge},
    lost_void_choice::{LostVoidChoiceReceipt, LostVoidChoiceRequest},
    lost_void_scene::{LostVoidObservation as V, LostVoidScreen as S, LotteryTickets},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionKind {
    Lottery,
    RouteChange,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidInteractRequest {
    pub run_id: Uuid,
    pub interaction_id: Uuid,
    pub kind: InteractionKind,
    pub challenge: LostVoidChallenge,
    pub options: ArtifactSelection,
    pub timeout_ms: u32,
}
impl LostVoidInteractRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        LostVoidChoiceRequest {
            run_id: self.run_id,
            interaction_id: self.interaction_id,
            challenge: self.challenge.clone(),
            options: self.options.clone(),
            return_to: vec![S::Lottery],
            timeout_ms: self.timeout_ms,
        }
        .validate()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractAction {
    Start,
    Back,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractStep {
    Input(InteractAction),
    Choice,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidInteractReceipt {
    pub run_id: Uuid,
    pub interaction_id: Uuid,
    pub returned_to: S,
    pub kind: InteractionKind,
    pub draws: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidInteractFlow {
    request: LostVoidInteractRequest,
    pending: Option<(InteractAction, bool)>,
    ready: Option<InteractAction>,
    tickets: Option<LotteryTickets>,
    child: Option<Uuid>,
    child_returned: bool,
    draws: u8,
    receipt: Option<LostVoidInteractReceipt>,
}
impl LostVoidInteractFlow {
    pub fn new(request: LostVoidInteractRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            pending: None,
            ready: None,
            tickets: None,
            child: None,
            child_returned: false,
            draws: 0,
            receipt: None,
        })
    }
    pub fn request(&self) -> &LostVoidInteractRequest {
        &self.request
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn receipt(&self) -> Option<&LostVoidInteractReceipt> {
        self.receipt.as_ref()
    }
    fn screen(&self) -> S {
        match self.request.kind {
            InteractionKind::Lottery => S::Lottery,
            InteractionKind::RouteChange => S::RouteChange,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.draws > 32
            || self
                .tickets
                .is_some_and(|t| t.total == 0 || t.total > 32 || t.remaining > t.total)
            || (self.pending.is_some() && self.ready.is_some())
            || (self.request.kind == InteractionKind::RouteChange
                && (self.tickets.is_some()
                    || self.draws != 0
                    || self.child.is_some()
                    || self
                        .pending
                        .is_some_and(|(a, _)| a == InteractAction::Start)
                    || self.ready == Some(InteractAction::Start)))
            || (self.child_returned && self.child.is_none())
            || self.child.is_some_and(|id| {
                id.is_nil()
                    || !self
                        .pending
                        .is_some_and(|(a, _)| a == InteractAction::Start)
            })
            || self.pending.is_some_and(|(a, _)| {
                a == InteractAction::Start && self.tickets.is_none_or(|t| t.remaining == 0)
            })
            || (self.request.kind == InteractionKind::Lottery
                && (self.pending.is_some_and(|(a, _)| a == InteractAction::Back)
                    || self.ready == Some(InteractAction::Back))
                && self.tickets.is_none_or(|t| t.remaining != 0))
            || self.tickets.is_some_and(|t| {
                u16::from(self.draws) + u16::from(t.remaining) > u16::from(t.total)
            })
            || self.receipt.as_ref().is_some_and(|r| {
                r.run_id != self.request.run_id
                    || r.interaction_id != self.request.interaction_id
                    || ![S::World, S::Event].contains(&r.returned_to)
                    || r.kind != self.request.kind
                    || r.draws != self.draws
                    || !self.pending.is_some_and(|(a, _)| a == InteractAction::Back)
            })
        {
            return Err("迷失之地局内交互检查点不一致");
        }
        Ok(())
    }
    pub fn next(&mut self, v: &V) -> Result<Option<InteractStep>, &'static str> {
        self.validate()?;
        self.ready = None;
        if self.completed() {
            return Ok(None);
        }
        if v.screen == S::Unknown {
            return Ok(None);
        }
        if [S::World, S::Event].contains(&v.screen) {
            if self.pending.is_some_and(|(a, _)| a == InteractAction::Back) {
                self.receipt = Some(LostVoidInteractReceipt {
                    run_id: self.request.run_id,
                    interaction_id: self.request.interaction_id,
                    returned_to: v.screen,
                    kind: self.request.kind,
                    draws: self.draws,
                });
            }
            return Ok(None);
        }
        if v.screen == S::CommonChoice {
            if self.request.kind != InteractionKind::Lottery
                || !self
                    .pending
                    .is_some_and(|(a, _)| a == InteractAction::Start)
                || self.child_returned
            {
                return Err("迷失之地抽奖没有本次领取意图");
            }
            if self.child.is_none() {
                self.child = Some(Uuid::new_v4());
            }
            return Ok(Some(InteractStep::Choice));
        }
        if v.screen != self.screen() {
            return Err("迷失之地局内交互遇到非预期页面");
        }
        if self.pending.is_some_and(|(a, _)| a == InteractAction::Back) {
            return Ok(None);
        }
        if self.request.kind == InteractionKind::RouteChange {
            self.ready = Some(InteractAction::Back);
            return Ok(Some(InteractStep::Input(InteractAction::Back)));
        }
        let Some(tickets) = v.lottery else {
            return Ok(None);
        };
        if tickets.total == 0 || tickets.total > 32 || tickets.remaining > tickets.total {
            return Err("迷失之地抽奖次数无效");
        }
        if self
            .pending
            .is_some_and(|(a, _)| a == InteractAction::Start)
        {
            let old = self.tickets.ok_or("迷失之地抽奖缺少消费前次数")?;
            if old.total != tickets.total
                || tickets.remaining > old.remaining
                || old.remaining - tickets.remaining > 1
            {
                return Err("迷失之地抽奖次数后态异常");
            }
            if tickets.remaining == old.remaining || (self.child.is_some() && !self.child_returned)
            {
                return Ok(None);
            }
            self.pending = None;
            self.child = None;
            self.child_returned = false;
            self.draws += 1;
        }
        self.tickets = Some(tickets);
        let action = if tickets.remaining == 0 {
            InteractAction::Back
        } else {
            if self.draws >= 32 {
                return Err("迷失之地抽奖达到次数上限");
            }
            InteractAction::Start
        };
        self.ready = Some(action);
        Ok(Some(InteractStep::Input(action)))
    }
    pub fn choice_request(&self) -> Result<LostVoidChoiceRequest, &'static str> {
        self.validate()?;
        Ok(LostVoidChoiceRequest {
            run_id: self.request.run_id,
            interaction_id: self.child.ok_or("迷失之地抽奖选择身份缺失")?,
            challenge: self.request.challenge.clone(),
            options: self.request.options.clone(),
            return_to: vec![S::Lottery],
            timeout_ms: self.request.timeout_ms,
        })
    }
    pub fn accept_choice(&mut self, r: &LostVoidChoiceReceipt) -> Result<(), &'static str> {
        self.validate()?;
        if self.child != Some(r.interaction_id)
            || r.run_id != self.request.run_id
            || r.returned_to != S::Lottery
            || !(1..=16).contains(&r.confirmed_pages)
            || self.child_returned
        {
            return Err("迷失之地抽奖选择回执不属于本次交互");
        }
        self.child_returned = true;
        Ok(())
    }
    pub fn prepare(&mut self, a: InteractAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.ready != Some(a) || self.pending.is_some() || self.completed() {
            return Err("迷失之地没有待保存的交互输入");
        }
        self.ready = None;
        self.pending = Some((a, false));
        Ok(())
    }
    pub fn allows(&self, a: InteractAction, v: &V) -> bool {
        self.validate().is_ok()
            && self.pending == Some((a, false))
            && v.screen == self.screen()
            && match self.request.kind {
                InteractionKind::RouteChange => a == InteractAction::Back,
                InteractionKind::Lottery => {
                    v.lottery == self.tickets
                        && self.tickets.is_some_and(|t| match a {
                            InteractAction::Start => t.remaining > 0,
                            InteractAction::Back => t.remaining == 0,
                        })
                }
            }
    }
    pub fn posted(&mut self, a: InteractAction) -> Result<(), &'static str> {
        if self.pending != Some((a, false)) {
            return Err("迷失之地交互投递回执不匹配");
        }
        self.pending = Some((a, true));
        self.validate()
    }
    pub fn not_posted(&mut self, a: InteractAction) -> Result<(), &'static str> {
        if self.pending != Some((a, false)) {
            return Err("迷失之地已投递交互不能撤销");
        }
        self.pending = None;
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.receipt = None;
        self.ready = None;
        Ok(())
    }
}
