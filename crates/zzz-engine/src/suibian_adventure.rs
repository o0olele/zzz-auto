//! 游历子流程共用外层控制与原生会话；保存提交意图后才发送 Maa 输入。
pub use crate::suibian_temple::HostingEnd as AdventureEnd;
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, suibian_adventure::AdventureInputEnd};
use od_core::{
    pipeline::{Interrupt, PipelineControl},
    suibian_adventure::TempleAdventure,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct AdventureSnapshot {
    pub flow: TempleAdventure,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct AdventureRun {
    pub snapshot: AdventureSnapshot,
    control: PipelineControl,
    end: Option<AdventureEnd>,
    steps: u32,
}
impl AdventureRun {
    pub fn new(flow: TempleAdventure, control: PipelineControl) -> Result<Self> {
        flow.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: AdventureSnapshot {
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
            self.end == Some(AdventureEnd::Paused) && self.snapshot.inputs_released,
            "游历尚未安全暂停"
        );
        self.snapshot.flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<AdventureEnd> {
        match self.control.requested() {
            Some(Interrupt::Stop) => Some(AdventureEnd::Stopped),
            Some(Interrupt::Pause) => Some(AdventureEnd::Paused),
            None => None,
        }
    }
    pub fn execute(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        publish: impl FnMut(AdventureSnapshot) -> Result<()>,
    ) -> Result<AdventureEnd> {
        self.execute_with_budget(session, game, Duration::MAX, publish)
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(AdventureSnapshot) -> Result<()>,
    ) -> Result<AdventureEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(AdventureEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "游历子流程不能直接重跑"
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
        let result = (|| -> Result<AdventureEnd> {
            session.require_suibian_adventure()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if self.snapshot.flow.completed() {
                    return Ok(AdventureEnd::Completed);
                }
                if remaining().is_zero() {
                    return Ok(AdventureEnd::TimedOut);
                }
                ensure!(self.steps < 4096, "游历步骤达到上限");
                self.steps += 1;
                let read =
                    session.read_suibian_adventure(&self.control.cancellation(), remaining());
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "游历读帧后尚未释放输入");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(AdventureEnd::TimedOut);
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
                    return Ok(self.interrupted().unwrap_or(AdventureEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_suibian_adventure(
                    &self.snapshot.flow,
                    &action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "游历输入后尚未释放输入");
                match input {
                    Ok(AdventureInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted_navigation(&action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(AdventureInputEnd::NotPosted | AdventureInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted()
                        .map_err(anyhow::Error::msg)?,
                    Ok(AdventureInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted()
                            .map_err(anyhow::Error::msg)?;
                        publish(self.snapshot.clone())?;
                        return Ok(AdventureEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不能证明未投递；保留意图，不能用重试猜测是否生效。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(AdventureEnd::TimedOut);
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
            self.end = Some(AdventureEnd::Failed);
        }
        ensure!(self.snapshot.inputs_released, "游历收尾无法确认释放输入");
        let mut end = result?;
        if let Some(requested) = self.interrupted() {
            end = requested;
        }
        if end == AdventureEnd::Paused && !self.control.acknowledge_pause() {
            end = AdventureEnd::Stopped;
        }
        self.end = Some(end);
        publish(self.snapshot.clone())?;
        // 不终结共享控制句柄，饮茶仙 / 制造等后继仍需接受暂停和停止。
        Ok(end)
    }
}
