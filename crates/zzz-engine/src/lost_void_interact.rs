//! 迷失之地抽奖与路径交互子流程，共用父控制、活动预算和输入收尾，只交回本次交互的返回页面。
use crate::lost_void_choice::{LostVoidChoiceEnd, LostVoidChoiceRun, LostVoidChoiceSnapshot};
use anyhow::{Result, ensure};
use maa_adapter::{
    NativeSession, lost_void_interact::InteractInputEnd as LostVoidInteractInputEnd,
};
use od_core::lost_void_interact::InteractStep;
use od_core::{
    lost_void_interact::{LostVoidInteractFlow, LostVoidInteractRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidInteractEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidInteractSnapshot {
    pub flow: LostVoidInteractFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
    pub choice: Option<LostVoidChoiceSnapshot>,
}
pub struct LostVoidInteractRun {
    snapshot: LostVoidInteractSnapshot,
    control: PipelineControl,
    end: Option<LostVoidInteractEnd>,
    steps: u32,
    child: Option<Box<LostVoidChoiceRun>>,
    child_started: bool,
}
impl LostVoidInteractRun {
    pub fn snapshot(&self) -> LostVoidInteractSnapshot {
        self.snapshot.clone()
    }
    /// 保存、释放及控制边界均正常之后才交回；不能作为挑战消费许可。
    pub fn receipt(&self) -> Option<&od_core::lost_void_interact::LostVoidInteractReceipt> {
        if self.control.requested().is_some() {
            return None;
        }
        self.receipt_for_parent()
    }
    pub(crate) fn receipt_for_parent(
        &self,
    ) -> Option<&od_core::lost_void_interact::LostVoidInteractReceipt> {
        if self.end != Some(LostVoidInteractEnd::Ready) || !self.snapshot.inputs_released {
            return None;
        }
        self.snapshot.flow.receipt()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn new(request: LostVoidInteractRequest, control: PipelineControl) -> Result<Self> {
        let flow = LostVoidInteractFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: LostVoidInteractSnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
                choice: None,
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
            self.end == Some(LostVoidInteractEnd::Paused),
            "迷失之地抽奖与路径交互准备尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    /// 父流程在子流程完成之后才暂停时，同样撤销旧到达证明。
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidInteractEnd::Paused | LostVoidInteractEnd::Ready)
            ) && self.snapshot.inputs_released,
            "迷失之地抽奖与路径交互准备尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        if self.child_started {
            self.child
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("迷失之地抽奖暂停子流程缺失"))?
                .resume_after_parent_pause()?;
        } else {
            ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        }
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidInteractEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidInteractEnd::Paused,
            Interrupt::Stop => LostVoidInteractEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidInteractSnapshot) -> Result<()>,
    ) -> Result<LostVoidInteractEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(LostVoidInteractEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "迷失之地抽奖与路径交互子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<LostVoidInteractEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_lost_void_scene()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidInteractEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(LostVoidInteractEnd::Ready);
                }
                ensure!(self.steps < 4096, "迷失之地抽奖与路径交互准备步骤达到上限");
                self.steps += 1;
                if let Some(child) = &mut self.child {
                    self.child_started = true;
                    let snapshot = &mut self.snapshot;
                    let end = child.execute_with_budget(session, game, remaining(), |s| {
                        snapshot.inputs_released = s.inputs_released;
                        snapshot.choice = Some(s);
                        snapshot.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        publish(snapshot.clone())
                    })?;
                    self.snapshot.choice = Some(child.snapshot());
                    self.snapshot.inputs_released = child.snapshot().inputs_released;
                    match end {
                        LostVoidChoiceEnd::Ready => {
                            let receipt = child
                                .receipt_for_parent()
                                .ok_or_else(|| anyhow::anyhow!("迷失之地抽奖选择尚未正常回交"))?;
                            self.snapshot
                                .flow
                                .accept_choice(receipt)
                                .map_err(anyhow::Error::msg)?;
                            self.child = None;
                            self.child_started = false;
                            publish(self.snapshot.clone())?;
                        }
                        LostVoidChoiceEnd::Paused => return Ok(LostVoidInteractEnd::Paused),
                        LostVoidChoiceEnd::Stopped => return Ok(LostVoidInteractEnd::Stopped),
                        LostVoidChoiceEnd::TimedOut => return Ok(LostVoidInteractEnd::TimedOut),
                        LostVoidChoiceEnd::Failed => return Ok(LostVoidInteractEnd::Failed),
                    }
                    continue;
                }
                let read = session.read_lost_void_interact(
                    &self.snapshot.flow,
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "迷失之地抽奖与路径交互读帧后尚未释放输入"
                );
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidInteractEnd::TimedOut);
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
                    InteractStep::Choice => {
                        let child = LostVoidChoiceRun::new(
                            self.snapshot
                                .flow
                                .choice_request()
                                .map_err(anyhow::Error::msg)?,
                            self.control.clone(),
                        )?;
                        ensure!(
                            child.uses_control(&self.control),
                            "迷失之地抽奖选择必须共用父控制"
                        );
                        self.snapshot.choice = Some(child.snapshot());
                        self.child = Some(Box::new(child));
                        publish(self.snapshot.clone())?;
                        continue;
                    }
                    InteractStep::Input(action) => action,
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
                    return Ok(self.interrupted().unwrap_or(LostVoidInteractEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_lost_void_interact(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "迷失之地抽奖与路径交互输入后尚未释放输入"
                );
                match input {
                    Ok(LostVoidInteractInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(LostVoidInteractInputEnd::NotPosted | LostVoidInteractInputEnd::Changed) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?
                    }
                    Ok(LostVoidInteractInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(LostVoidInteractEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(LostVoidInteractEnd::TimedOut);
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
            self.end = Some(LostVoidInteractEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "迷失之地抽奖与路径交互收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(LostVoidInteractEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = LostVoidInteractEnd::TimedOut;
        }
        if end == LostVoidInteractEnd::Paused && !self.control.acknowledge_pause() {
            end = LostVoidInteractEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
