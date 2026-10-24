//! 父副本调用的手册传送准备，不终结共享控制句柄或写入运行次数。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, compendium::TransportInputEnd};
use od_core::{
    compendium::TransportFlow,
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportEnd {
    Arrived,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportSnapshot {
    pub flow: TransportFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct TransportRun {
    pub snapshot: TransportSnapshot,
    control: PipelineControl,
    end: Option<TransportEnd>,
    steps: u32,
}
impl TransportRun {
    pub fn new(flow: TransportFlow, control: PipelineControl) -> Result<Self> {
        flow.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: TransportSnapshot {
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
            self.end == Some(TransportEnd::Paused) && self.snapshot.inputs_released,
            "手册传送准备尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<TransportEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => TransportEnd::Paused,
            Interrupt::Stop => TransportEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(TransportSnapshot) -> Result<()>,
    ) -> Result<TransportEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(TransportEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "手册传送子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<TransportEnd> {
            session.require_compendium()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(TransportEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(TransportEnd::Arrived);
                }
                ensure!(self.steps < 4096, "手册传送准备步骤达到上限");
                self.steps += 1;
                let read = session.read_compendium(
                    self.snapshot.flow.request(),
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "手册传送读帧后尚未释放输入");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(TransportEnd::TimedOut);
                }
                let Some(view) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let action = self.snapshot.flow.next(&view).map_err(anyhow::Error::msg)?;
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                let Some(action) = action else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
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
                    return Ok(self.interrupted().unwrap_or(TransportEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_compendium(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "手册传送输入后尚未释放输入");
                match input {
                    Ok(TransportInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(TransportInputEnd::NotPosted | TransportInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(TransportInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(TransportEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(TransportEnd::TimedOut);
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
            self.end = Some(TransportEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "手册传送收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(TransportEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = TransportEnd::TimedOut;
        }
        if end == TransportEnd::Paused && !self.control.acknowledge_pause() {
            end = TransportEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
