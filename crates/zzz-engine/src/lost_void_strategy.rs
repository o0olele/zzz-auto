//! 迷失之地战略子流程，共用父控制、活动预算和输入收尾，不出战或记次数。
use anyhow::{Result, ensure};
use maa_adapter::{
    NativeSession, lost_void_strategy::StrategyInputEnd as LostVoidStrategyInputEnd,
};
use od_core::{
    lost_void_strategy::{LostVoidStrategyFlow, LostVoidStrategyRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidStrategyEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidStrategySnapshot {
    pub flow: LostVoidStrategyFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
    pub up_observed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub up_observation_started_at: Option<chrono::DateTime<chrono::Utc>>,
}
pub struct LostVoidStrategyRun {
    snapshot: LostVoidStrategySnapshot,
    control: PipelineControl,
    end: Option<LostVoidStrategyEnd>,
    steps: u32,
}
impl LostVoidStrategyRun {
    pub fn snapshot(&self) -> LostVoidStrategySnapshot {
        self.snapshot.clone()
    }
    /// 保存、释放及控制边界均正常之后才交回；不能作为挑战消费许可。
    pub fn receipt(&self) -> Option<&od_core::lost_void_strategy::LostVoidStrategyReceipt> {
        if self.control.requested().is_some() {
            return None;
        }
        self.receipt_for_parent()
    }
    pub(crate) fn receipt_for_parent(
        &self,
    ) -> Option<&od_core::lost_void_strategy::LostVoidStrategyReceipt> {
        if self.end != Some(LostVoidStrategyEnd::Ready) || !self.snapshot.inputs_released {
            return None;
        }
        self.snapshot.flow.receipt()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn new(request: LostVoidStrategyRequest, control: PipelineControl) -> Result<Self> {
        let flow = LostVoidStrategyFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: LostVoidStrategySnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
                up_observed_at: None,
                up_observation_started_at: None,
            },
            control,
            end: None,
            steps: 0,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidStrategyEnd::Paused),
            "迷失之地战略准备尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    /// 父流程在子流程完成之后才暂停时，同样撤销旧到达证明。
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidStrategyEnd::Paused | LostVoidStrategyEnd::Ready)
            ) && self.snapshot.inputs_released,
            "迷失之地战略准备尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidStrategyEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidStrategyEnd::Paused,
            Interrupt::Stop => LostVoidStrategyEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidStrategySnapshot) -> Result<()>,
    ) -> Result<LostVoidStrategyEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(LostVoidStrategyEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "迷失之地战略子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<LostVoidStrategyEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_lost_void_scene()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidStrategyEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(LostVoidStrategyEnd::Ready);
                }
                ensure!(self.steps < 4096, "迷失之地战略准备步骤达到上限");
                self.steps += 1;
                let observation_started_at = chrono::Utc::now();
                let read = session.read_lost_void_strategy(
                    &self.snapshot.flow,
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "迷失之地战略读帧后尚未释放输入"
                );
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidStrategyEnd::TimedOut);
                }
                let Some(view) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let first_up = self.snapshot.flow.up_agents().is_none();
                let observed_at = chrono::Utc::now();
                let action = self.snapshot.flow.next(&view).map_err(anyhow::Error::msg)?;
                if first_up && self.snapshot.flow.up_agents().is_some() {
                    self.snapshot.up_observed_at = Some(observed_at);
                    self.snapshot.up_observation_started_at = Some(observation_started_at);
                }
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
                    return Ok(self.interrupted().unwrap_or(LostVoidStrategyEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_lost_void_strategy(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "迷失之地战略输入后尚未释放输入"
                );
                match input {
                    Ok(LostVoidStrategyInputEnd::PartialScroll) => self
                        .snapshot
                        .flow
                        .partial_scroll(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(LostVoidStrategyInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(LostVoidStrategyInputEnd::NotPosted | LostVoidStrategyInputEnd::Changed) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?
                    }
                    Ok(LostVoidStrategyInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(LostVoidStrategyEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(LostVoidStrategyEnd::TimedOut);
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
            self.end = Some(LostVoidStrategyEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "迷失之地战略收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(LostVoidStrategyEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = LostVoidStrategyEnd::TimedOut;
        }
        if end == LostVoidStrategyEnd::Paused && !self.control.acknowledge_pause() {
            end = LostVoidStrategyEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
