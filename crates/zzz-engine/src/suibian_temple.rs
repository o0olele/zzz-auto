//! 随便观托管子执行器。借用外层已有的原生会话和控制句柄，不另开运行槽。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, PipelineEnd};
use od_core::{
    pipeline::{Interrupt, PipelineControl},
    suibian_temple_auto::TempleHostingProgress,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostingEnd {
    Completed,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}

#[derive(Debug, Clone)]
pub struct HostingSnapshot {
    pub progress: TempleHostingProgress,
    pub active_ms: u64,
    pub inputs_released: bool,
}

pub struct HostingRun {
    pub snapshot: HostingSnapshot,
    timeout_ms: u32,
    control: PipelineControl,
    end: Option<HostingEnd>,
    steps: u32,
}
impl HostingRun {
    pub fn new(timeout_ms: u32, control: PipelineControl) -> Result<Self> {
        ensure!((1000..=3_600_000).contains(&timeout_ms), "托管超时超出范围");
        Ok(Self {
            snapshot: HostingSnapshot {
                progress: Default::default(),
                active_ms: 0,
                inputs_released: true,
            },
            timeout_ms,
            control,
            end: None,
            steps: 0,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(HostingEnd::Paused) && self.snapshot.inputs_released,
            "托管尚未安全暂停"
        );
        let mut next = self.snapshot.progress.clone();
        next.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.progress = next;
        self.end = None;
        Ok(())
    }
    fn interrupt(&mut self) -> Option<HostingEnd> {
        match self.control.requested() {
            Some(Interrupt::Stop) => Some(HostingEnd::Stopped),
            Some(Interrupt::Pause) => {
                if self.control.acknowledge_pause() {
                    Some(HostingEnd::Paused)
                } else {
                    self.interrupt()
                }
            }
            None => None,
        }
    }
    pub fn execute(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        publish: impl FnMut(HostingSnapshot) -> Result<()>,
    ) -> Result<HostingEnd> {
        self.execute_with_budget(session, game, Duration::MAX, publish)
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(HostingSnapshot) -> Result<()>,
    ) -> Result<HostingEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(HostingEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "托管子流程不能直接重跑"
        );
        self.snapshot
            .progress
            .validate()
            .map_err(anyhow::Error::msg)?;
        let initial_ms = self.snapshot.active_ms;
        let started = Instant::now();
        let result = (|| -> Result<HostingEnd> {
            loop {
                if let Some(end) = self.interrupt() {
                    return Ok(end);
                }
                if self.snapshot.progress.completed() {
                    return Ok(HostingEnd::Completed);
                }
                let used = initial_ms.saturating_add(started.elapsed().as_millis() as u64);
                let remaining = u64::from(self.timeout_ms).saturating_sub(used).min(
                    parent_remaining
                        .saturating_sub(started.elapsed())
                        .as_millis()
                        .min(u64::MAX as u128) as u64,
                );
                if remaining == 0 {
                    return Ok(HostingEnd::TimedOut);
                }
                ensure!(self.steps < 4096, "托管步骤达到上限");
                let before = self.snapshot.progress.clone();
                self.snapshot
                    .progress
                    .before_step()
                    .map_err(anyhow::Error::msg)?;
                self.snapshot.active_ms = used;
                // 所有输入先保存意图，失败没有 Tasker 投递，保留上一个安全检查点。
                if let Err(error) = publish(self.snapshot.clone()) {
                    self.snapshot.progress = before;
                    return Err(error);
                }
                // 保存可能耗时，重新扣减父预算；耗尽后不再投递。
                let remaining = Duration::from_millis(u64::from(self.timeout_ms).saturating_sub(
                    initial_ms.saturating_add(started.elapsed().as_millis() as u64),
                ))
                .min(parent_remaining.saturating_sub(started.elapsed()));
                if remaining.is_zero() {
                    self.snapshot
                        .progress
                        .not_posted(&before)
                        .map_err(anyhow::Error::msg)?;
                    return Ok(HostingEnd::TimedOut);
                }
                self.snapshot.inputs_released = false;
                let execution = session.suibian_temple_hosting_step(
                    &self.snapshot.progress,
                    self.control.cancellation(),
                    remaining,
                    game,
                )?;
                self.steps += 1;
                self.snapshot.inputs_released = execution.report.released;
                ensure!(self.snapshot.inputs_released, "托管步骤尚未确认释放输入");
                if execution.task_id.is_none() {
                    self.snapshot
                        .progress
                        .not_posted(&before)
                        .map_err(anyhow::Error::msg)?;
                }
                match execution.report.end {
                    PipelineEnd::Succeeded => {
                        let node = execution
                            .nodes
                            .iter()
                            .rev()
                            .find(|node| node.completed)
                            .ok_or_else(|| anyhow::anyhow!("托管步骤缺少完成节点"))?;
                        self.snapshot
                            .progress
                            .observe(&node.name)
                            .map_err(anyhow::Error::msg)?;
                    }
                    PipelineEnd::Cancelled => {
                        // 即使只看到动作开始，恢复也必须检查后态，不能重放已经投递的动作。
                        if execution.task_id.is_some() {
                            self.snapshot
                                .progress
                                .resume()
                                .map_err(anyhow::Error::msg)?;
                        }
                        ensure!(self.control.requested().is_some(), "托管收到未登记取消");
                    }
                    PipelineEnd::TimedOut => {
                        self.snapshot
                            .progress
                            .resume()
                            .map_err(anyhow::Error::msg)?;
                        return Ok(HostingEnd::TimedOut);
                    }
                    PipelineEnd::Failed => {
                        self.snapshot
                            .progress
                            .resume()
                            .map_err(anyhow::Error::msg)?;
                        anyhow::bail!("托管 Pipeline 执行失败");
                    }
                }
                self.snapshot.active_ms =
                    initial_ms.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.snapshot.active_ms = initial_ms.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() {
            self.end = Some(HostingEnd::Failed);
        }
        let released = session.release_inputs();
        self.snapshot.inputs_released = released.is_ok();
        if let Err(error) = released {
            self.end = Some(HostingEnd::Failed);
            return Err(error);
        }
        let end = result?;
        // 子流程不能 finish 共享句柄，后继游历、购买与返回仍需接受停止。
        let end = self.interrupt().unwrap_or(end);
        self.end = Some(end);
        publish(self.snapshot.clone())?;
        Ok(end)
    }
}
