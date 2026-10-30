//! 迷失之地事件子流程，共用父控制、活动预算和输入收尾，只交回本次交互的返回页面。
use crate::lost_void_choice::{LostVoidChoiceEnd, LostVoidChoiceRun, LostVoidChoiceSnapshot};
use crate::{
    lost_void_gear::{LostVoidGearEnd, LostVoidGearRun, LostVoidGearSnapshot},
    lost_void_interact::{LostVoidInteractEnd, LostVoidInteractRun, LostVoidInteractSnapshot},
    lost_void_store::{LostVoidStoreEnd, LostVoidStoreRun, LostVoidStoreSnapshot},
};
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, lost_void_event::EventInputEnd as LostVoidEventInputEnd};
use od_core::lost_void_event::EventStep;
use od_core::lost_void_event::{EventChildReceipt, EventChildRequest};
use od_core::{
    lost_void_event::{LostVoidEventFlow, LostVoidEventRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidEventEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "snapshot", rename_all = "snake_case")]
pub enum EventChildSnapshot {
    Choice(Box<LostVoidChoiceSnapshot>),
    Gear(Box<LostVoidGearSnapshot>),
    Store(Box<LostVoidStoreSnapshot>),
    Interact(Box<LostVoidInteractSnapshot>),
}
impl EventChildSnapshot {
    fn inputs_released(&self) -> bool {
        match self {
            Self::Choice(s) => s.inputs_released,
            Self::Gear(s) => s.inputs_released,
            Self::Store(s) => s.inputs_released,
            Self::Interact(s) => s.inputs_released,
        }
    }
}
enum EventChildRun {
    Choice(Box<LostVoidChoiceRun>),
    Gear(Box<LostVoidGearRun>),
    Store(Box<LostVoidStoreRun>),
    Interact(Box<LostVoidInteractRun>),
}
macro_rules! child_match {
    ($self:expr,$r:ident,$body:expr) => {
        match $self {
            EventChildRun::Choice($r) => $body,
            EventChildRun::Gear($r) => $body,
            EventChildRun::Store($r) => $body,
            EventChildRun::Interact($r) => $body,
        }
    };
}
impl EventChildRun {
    fn new(request: EventChildRequest, control: PipelineControl) -> Result<Self> {
        Ok(match request {
            EventChildRequest::Choice(r) => {
                Self::Choice(Box::new(LostVoidChoiceRun::new(r, control)?))
            }
            EventChildRequest::Gear(r) => Self::Gear(Box::new(LostVoidGearRun::new(r, control)?)),
            EventChildRequest::Store(r) => {
                Self::Store(Box::new(LostVoidStoreRun::new(r, control)?))
            }
            EventChildRequest::Interact(r) => {
                Self::Interact(Box::new(LostVoidInteractRun::new(r, control)?))
            }
        })
    }
    fn uses_control(&self, c: &PipelineControl) -> bool {
        child_match!(self, r, r.uses_control(c))
    }
    fn inputs_released(&self) -> bool {
        child_match!(self, r, r.snapshot().inputs_released)
    }
    fn snapshot(&self) -> EventChildSnapshot {
        match self {
            Self::Choice(r) => EventChildSnapshot::Choice(Box::new(r.snapshot())),
            Self::Gear(r) => EventChildSnapshot::Gear(Box::new(r.snapshot())),
            Self::Store(r) => EventChildSnapshot::Store(Box::new(r.snapshot())),
            Self::Interact(r) => EventChildSnapshot::Interact(Box::new(r.snapshot())),
        }
    }
    fn receipt_for_parent(&self) -> Option<EventChildReceipt> {
        match self {
            Self::Choice(r) => r
                .receipt_for_parent()
                .cloned()
                .map(EventChildReceipt::Choice),
            Self::Gear(r) => r.receipt_for_parent().cloned().map(EventChildReceipt::Gear),
            Self::Store(r) => r
                .receipt_for_parent()
                .cloned()
                .map(EventChildReceipt::Store),
            Self::Interact(r) => r
                .receipt_for_parent()
                .cloned()
                .map(EventChildReceipt::Interact),
        }
    }
    fn resume_after_parent_pause(&mut self) -> Result<()> {
        child_match!(self, r, r.resume_after_parent_pause())
    }
    fn execute_with_budget(
        &mut self,
        n: &mut NativeSession,
        game: bool,
        remaining: Duration,
        mut publish: impl FnMut(EventChildSnapshot) -> Result<()>,
    ) -> Result<LostVoidEventEnd> {
        macro_rules! execute {
            ($r:expr,$kind:ident,$end:ident) => {
                $r.execute_with_budget(n, game, remaining, |s| {
                    publish(EventChildSnapshot::$kind(Box::new(s)))
                })
                .map(|end| match end {
                    $end::Ready => LostVoidEventEnd::Ready,
                    $end::Paused => LostVoidEventEnd::Paused,
                    $end::Stopped => LostVoidEventEnd::Stopped,
                    $end::TimedOut => LostVoidEventEnd::TimedOut,
                    $end::Failed => LostVoidEventEnd::Failed,
                })
            };
        }
        match self {
            Self::Choice(r) => execute!(r, Choice, LostVoidChoiceEnd),
            Self::Gear(r) => execute!(r, Gear, LostVoidGearEnd),
            Self::Store(r) => execute!(r, Store, LostVoidStoreEnd),
            Self::Interact(r) => execute!(r, Interact, LostVoidInteractEnd),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidEventSnapshot {
    pub flow: LostVoidEventFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
    pub child: Option<EventChildSnapshot>,
}
pub struct LostVoidEventRun {
    snapshot: LostVoidEventSnapshot,
    control: PipelineControl,
    end: Option<LostVoidEventEnd>,
    steps: u32,
    child: Option<Box<EventChildRun>>,
    child_started: bool,
}
impl LostVoidEventRun {
    pub fn snapshot(&self) -> LostVoidEventSnapshot {
        self.snapshot.clone()
    }
    /// 保存、释放及控制边界均正常之后才交回；不能作为挑战消费许可。
    pub fn receipt(&self) -> Option<&od_core::lost_void_event::LostVoidEventReceipt> {
        if self.control.requested().is_some() {
            return None;
        }
        self.receipt_for_parent()
    }
    pub(crate) fn receipt_for_parent(
        &self,
    ) -> Option<&od_core::lost_void_event::LostVoidEventReceipt> {
        if self.end != Some(LostVoidEventEnd::Ready) || !self.snapshot.inputs_released {
            return None;
        }
        self.snapshot.flow.receipt()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn new(request: LostVoidEventRequest, control: PipelineControl) -> Result<Self> {
        let flow = LostVoidEventFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: LostVoidEventSnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
                child: None,
            },
            control,
            end: None,
            steps: 0,
            child: None,
            child_started: false,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidEventEnd::Paused),
            "迷失之地事件准备尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    /// 同一次进门后再次弹出自动事件时继续观察，保留已消费选项及子动作历史。
    pub(crate) fn continue_observing(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidEventEnd::Ready)
                && self.snapshot.inputs_released
                && self.control.requested().is_none()
                && !self.child_started
                && self.child.is_none(),
            "同次事件尚未安全交回"
        );
        self.snapshot.flow.resume().map_err(anyhow::Error::msg)?;
        self.end = None;
        Ok(())
    }
    /// 父流程在子流程完成之后才暂停时，同样撤销旧到达证明。
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidEventEnd::Paused | LostVoidEventEnd::Ready)
            ) && self.snapshot.inputs_released,
            "迷失之地事件准备尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        if self.child_started {
            self.child
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("迷失之地事件暂停子流程缺失"))?
                .resume_after_parent_pause()?;
        } else {
            ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        }
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidEventEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidEventEnd::Paused,
            Interrupt::Stop => LostVoidEventEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidEventSnapshot) -> Result<()>,
    ) -> Result<LostVoidEventEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(LostVoidEventEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "迷失之地事件子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<LostVoidEventEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_lost_void_scene()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidEventEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(LostVoidEventEnd::Ready);
                }
                ensure!(self.steps < 4096, "迷失之地事件准备步骤达到上限");
                self.steps += 1;
                if let Some(child) = &mut self.child {
                    self.child_started = true;
                    let snapshot = &mut self.snapshot;
                    let end = child.execute_with_budget(session, game, remaining(), |s| {
                        snapshot.inputs_released = s.inputs_released();
                        snapshot.child = Some(s);
                        snapshot.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        publish(snapshot.clone())
                    })?;
                    self.snapshot.child = Some(child.snapshot());
                    self.snapshot.inputs_released = child.inputs_released();
                    match end {
                        LostVoidEventEnd::Ready => {
                            let receipt = child
                                .receipt_for_parent()
                                .ok_or_else(|| anyhow::anyhow!("迷失之地事件选择尚未正常回交"))?;
                            self.snapshot
                                .flow
                                .accept_child(&receipt)
                                .map_err(anyhow::Error::msg)?;
                            self.child = None;
                            self.child_started = false;
                            self.snapshot.child = None;
                            publish(self.snapshot.clone())?;
                        }
                        LostVoidEventEnd::Paused => return Ok(LostVoidEventEnd::Paused),
                        LostVoidEventEnd::Stopped => return Ok(LostVoidEventEnd::Stopped),
                        LostVoidEventEnd::TimedOut => return Ok(LostVoidEventEnd::TimedOut),
                        LostVoidEventEnd::Failed => return Ok(LostVoidEventEnd::Failed),
                    }
                    continue;
                }
                let read = session.read_lost_void_event(
                    &self.snapshot.flow,
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "迷失之地事件读帧后尚未释放输入"
                );
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidEventEnd::TimedOut);
                }
                let Some(view) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let action = self.snapshot.flow.next(&view).map_err(anyhow::Error::msg)?;
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                let Some(step) = action else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let action = match step {
                    EventStep::Child => {
                        let child = EventChildRun::new(
                            self.snapshot
                                .flow
                                .child_request()
                                .map_err(anyhow::Error::msg)?,
                            self.control.clone(),
                        )?;
                        ensure!(
                            child.uses_control(&self.control),
                            "迷失之地事件选择必须共用父控制"
                        );
                        self.snapshot.child = Some(child.snapshot());
                        self.child = Some(Box::new(child));
                        publish(self.snapshot.clone())?;
                        continue;
                    }
                    EventStep::Input(action) => action,
                };
                let before = self.snapshot.flow.clone();
                self.snapshot
                    .flow
                    .prepare(action)
                    .map_err(anyhow::Error::msg)?;
                if let Err(error) = publish(self.snapshot.clone()) {
                    self.snapshot.flow = before;
                    return Err(error);
                }
                if remaining().is_zero() || self.interrupted().is_some() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(LostVoidEventEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_lost_void_event(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "迷失之地事件输入后尚未释放输入"
                );
                match input {
                    Ok(LostVoidEventInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(LostVoidEventInputEnd::NotPosted | LostVoidEventInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(LostVoidEventInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(LostVoidEventEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(LostVoidEventEnd::TimedOut);
                        }
                        return Err(error);
                    }
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() || !self.snapshot.inputs_released {
            self.end = Some(LostVoidEventEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "迷失之地事件收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(LostVoidEventEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = LostVoidEventEnd::TimedOut;
        }
        if end == LostVoidEventEnd::Paused && !self.control.acknowledge_pause() {
            end = LostVoidEventEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
