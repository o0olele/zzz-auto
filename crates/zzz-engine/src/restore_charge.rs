//! 副本电量恢复与父流程共用控制器、剩余预算和持久化回调。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, RecognitionImage, restore_charge::RestoreInputEnd};
use od_core::{
    pipeline::{Interrupt, PipelineControl},
    restore_charge::{RestoreFlow, RestoreReturn},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreEnd {
    Completed,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreSnapshot {
    pub flow: RestoreFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct RestoreRun {
    pub snapshot: RestoreSnapshot,
    control: PipelineControl,
    end: Option<RestoreEnd>,
    steps: u32,
}
impl RestoreRun {
    pub fn new(flow: RestoreFlow, control: PipelineControl) -> Result<Self> {
        flow.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: RestoreSnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
            },
            control,
            end: None,
            steps: 0,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(RestoreEnd::Paused) && self.snapshot.inputs_released,
            "恢复电量尚未安全暂停"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<RestoreEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => RestoreEnd::Paused,
            Interrupt::Stop => RestoreEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut parent: impl FnMut(
            &NativeSession,
            &RecognitionImage,
            &AtomicBool,
            Duration,
        ) -> Result<Option<RestoreReturn>>,
        mut publish: impl FnMut(RestoreSnapshot) -> Result<()>,
    ) -> Result<RestoreEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(RestoreEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "恢复子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial_ms = self.snapshot.active_ms;
        let budget = u64::from(self.snapshot.flow.request().timeout_ms);
        let remaining =
            || {
                Duration::from_millis(budget.saturating_sub(
                    initial_ms.saturating_add(started.elapsed().as_millis() as u64),
                ))
                .min(parent_remaining.saturating_sub(started.elapsed()))
            };
        let result = (|| -> Result<RestoreEnd> {
            session.require_restore_charge()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(RestoreEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(RestoreEnd::Completed);
                }
                ensure!(self.steps < 4096, "恢复电量步骤达到上限");
                self.steps += 1;
                let read = session.read_restore_charge(
                    &self.control.cancellation(),
                    remaining(),
                    &mut parent,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "恢复读帧后尚未释放输入");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(RestoreEnd::TimedOut);
                }
                let Some(view) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let action = self.snapshot.flow.next(&view).map_err(anyhow::Error::msg)?;
                self.snapshot.active_ms =
                    initial_ms.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                let Some(action) = action else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let before = self.snapshot.flow.clone();
                self.snapshot
                    .flow
                    .prepare(&action)
                    .map_err(anyhow::Error::msg)?;
                if self.snapshot.flow != before
                    && let Err(error) = publish(self.snapshot.clone())
                {
                    self.snapshot.flow = before;
                    return Err(error);
                }
                if remaining().is_zero() || self.interrupted().is_some() {
                    self.snapshot
                        .flow
                        .not_posted(&action)
                        .map_err(anyhow::Error::msg)?;
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(RestoreEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_restore_charge(
                    &self.snapshot.flow,
                    &action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "恢复输入后尚未释放输入");
                match input {
                    Ok(RestoreInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(&action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(RestoreInputEnd::NotPosted | RestoreInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(&action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(RestoreInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(&action)
                            .map_err(anyhow::Error::msg)?;
                        publish(self.snapshot.clone())?;
                        return Ok(RestoreEnd::TimedOut);
                    }
                    Err(error) => {
                        // 错误不证明未投递。保留消费和奖励确认意图，继续后仅检查实际后态。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(RestoreEnd::TimedOut);
                        }
                        return Err(error);
                    }
                }
                self.snapshot.active_ms =
                    initial_ms.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.snapshot.active_ms = initial_ms.saturating_add(started.elapsed().as_millis() as u64);
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        if result.is_err() || !self.snapshot.inputs_released {
            self.end = Some(RestoreEnd::Failed);
        }
        ensure!(self.snapshot.inputs_released, "恢复收尾无法确认释放输入");
        let mut end = result?;
        if let Some(requested) = self.interrupted() {
            end = requested;
        }
        if end == RestoreEnd::Paused && !self.control.acknowledge_pause() {
            end = RestoreEnd::Stopped;
        }
        self.end = Some(end);
        publish(self.snapshot.clone())?;
        // 父副本仍使用这个控制句柄，不在子流程中终结它。
        Ok(end)
    }
}
