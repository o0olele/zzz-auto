//! 售卖子流程共用外层控制与原生会话；保存提交意图后才发送 Maa 输入。
pub use crate::suibian_temple::HostingEnd as SalesEnd;
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, suibian_sales::SalesInputEnd};
use od_core::{
    pipeline::{Interrupt, PipelineControl},
    suibian_sales::TempleSales,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct SalesSnapshot {
    pub flow: TempleSales,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct SalesRun {
    pub snapshot: SalesSnapshot,
    control: PipelineControl,
    end: Option<SalesEnd>,
    steps: u32,
}
impl SalesRun {
    pub fn new(flow: TempleSales, control: PipelineControl) -> Result<Self> {
        flow.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: SalesSnapshot {
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
            self.end == Some(SalesEnd::Paused) && self.snapshot.inputs_released,
            "售卖尚未安全暂停"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<SalesEnd> {
        match self.control.requested() {
            Some(Interrupt::Stop) => Some(SalesEnd::Stopped),
            Some(Interrupt::Pause) => Some(SalesEnd::Paused),
            None => None,
        }
    }
    pub fn execute(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        publish: impl FnMut(SalesSnapshot) -> Result<()>,
    ) -> Result<SalesEnd> {
        self.execute_with_budget(session, game, Duration::MAX, publish)
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(SalesSnapshot) -> Result<()>,
    ) -> Result<SalesEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(SalesEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "售卖子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial_ms = self.snapshot.active_ms;
        let budget = u64::from(self.snapshot.flow.config.timeout_ms);
        let remaining = || {
            let elapsed = started.elapsed();
            Duration::from_millis(
                budget.saturating_sub(initial_ms.saturating_add(elapsed.as_millis() as u64)),
            )
            .min(parent_remaining.saturating_sub(elapsed))
        };
        let result = (|| -> Result<SalesEnd> {
            session.require_suibian_sales()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if self.snapshot.flow.completed() {
                    return Ok(SalesEnd::Completed);
                }
                if remaining().is_zero() {
                    return Ok(SalesEnd::TimedOut);
                }
                ensure!(self.steps < 4096, "售卖步骤达到上限");
                self.steps += 1;
                let read = session.read_suibian_sales(&self.control.cancellation(), remaining());
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "售卖读帧后尚未释放输入");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(SalesEnd::TimedOut);
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
                        .not_posted()
                        .map_err(anyhow::Error::msg)?;
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(SalesEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_suibian_sales(
                    &self.snapshot.flow,
                    &action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "售卖输入后尚未释放输入");
                match input {
                    Ok(SalesInputEnd::Posted) => {}
                    Ok(SalesInputEnd::NotPosted | SalesInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted()
                        .map_err(anyhow::Error::msg)?,
                    Ok(SalesInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted()
                            .map_err(anyhow::Error::msg)?;
                        publish(self.snapshot.clone())?;
                        return Ok(SalesEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不能证明未投递；保留意图，不能用重试猜测是否生效。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(SalesEnd::TimedOut);
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
            self.end = Some(SalesEnd::Failed);
        }
        ensure!(self.snapshot.inputs_released, "售卖收尾无法确认释放输入");
        let mut end = result?;
        if let Some(requested) = self.interrupted() {
            end = requested;
        }
        if end == SalesEnd::Paused && !self.control.acknowledge_pause() {
            end = SalesEnd::Stopped;
        }
        self.end = Some(end);
        publish(self.snapshot.clone())?;
        // 不终结共享控制句柄，可选购买等后继仍需接受暂停和停止。
        Ok(end)
    }
}
