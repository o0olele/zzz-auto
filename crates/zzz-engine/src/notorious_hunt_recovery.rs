//! 周常本次失败后的撤退、退出和两帧原免费次数核对，共用父控制器和预算。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, deep_hunt::HuntInputEnd};
use od_core::{
    notorious_hunt_recovery::WeeklyRecoveryFlow,
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeeklyRecoveryEnd {
    Returned,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyRecoverySnapshot {
    pub flow: WeeklyRecoveryFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct WeeklyRecoveryRun {
    pub snapshot: WeeklyRecoverySnapshot,
    control: PipelineControl,
    end: Option<WeeklyRecoveryEnd>,
    steps: u32,
}
impl WeeklyRecoveryRun {
    pub(crate) fn after_timeout_exit(
        timeout: &crate::notorious_hunt_timeout::WeeklyTimeoutRun,
        control: PipelineControl,
    ) -> Result<Self> {
        let flow = timeout
            .completed_for_parent(&control)
            .ok_or_else(|| anyhow::anyhow!("周常超时退出尚无同次完成并释放输入的结果"))?;
        Self::new(
            WeeklyRecoveryFlow::after_timeout(flow.clone()).map_err(anyhow::Error::msg)?,
            control,
        )
    }
    pub fn after_combat(
        result: crate::combat::CombatSegmentEnd,
        inputs_released: bool,
        timeout_ms: u32,
        mission_type: String,
        rewards_before: u8,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(inputs_released, "战斗输入未释放，不能交接周常失败收尾");
        let crate::combat::CombatSegmentEnd::Result(result) = result else {
            anyhow::bail!("战斗未正常结束，不能交接周常失败收尾");
        };
        Self::new(
            WeeklyRecoveryFlow::new(result, timeout_ms, mission_type, rewards_before)
                .map_err(anyhow::Error::msg)?,
            control,
        )
    }
    pub fn new(flow: WeeklyRecoveryFlow, control: PipelineControl) -> Result<Self> {
        flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(!flow.completed(), "已完成周常失败收尾不能重新创建任务");
        Ok(Self {
            snapshot: WeeklyRecoverySnapshot {
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
            matches!(
                self.end,
                Some(WeeklyRecoveryEnd::Paused | WeeklyRecoveryEnd::Returned)
            ) && self.snapshot.inputs_released
                && self.control.requested() == Some(Interrupt::Pause),
            "周常失败收尾尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止不能继续");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<WeeklyRecoveryEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => WeeklyRecoveryEnd::Paused,
            Interrupt::Stop => WeeklyRecoveryEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        n: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(WeeklyRecoverySnapshot) -> Result<()>,
    ) -> Result<WeeklyRecoveryEnd> {
        use WeeklyRecoveryEnd as E;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "周常失败收尾不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.timeout_ms()).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<E> {
            n.require_deep_hunt()?;
            loop {
                if let Some(e) = self.interrupted() {
                    return Ok(e);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(E::Returned);
                }
                ensure!(self.steps < 4096, "周常失败收尾步骤达到上限");
                self.steps += 1;
                let read = n.read_weekly_hunt_recovery(
                    self.snapshot.flow.mission_type(),
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = n.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "周常失败收尾读帧后尚未释放输入"
                );
                if let Some(e) = self.interrupted() {
                    return Ok(e);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
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
                if let Err(e) = publish(self.snapshot.clone()) {
                    self.snapshot.flow = before;
                    return Err(e);
                }
                if remaining().is_zero() || self.interrupted().is_some() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(E::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = n.act_weekly_hunt_recovery(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = n.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "周常失败收尾输入后尚未释放输入"
                );
                match input {
                    Ok(HuntInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(
                        HuntInputEnd::NotPosted | HuntInputEnd::Changed | HuntInputEnd::TimedOut,
                    ) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Err(e) => return Err(e),
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.snapshot.inputs_released = n.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        self.end = Some(E::Failed);
        ensure!(self.snapshot.inputs_released, "周常失败收尾收尾未释放输入");
        let mut end = result?;
        publish(self.snapshot.clone())?;
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(e) = self.interrupted() {
            end = e;
        } else if remaining().is_zero() {
            end = E::TimedOut;
        }
        if end == E::Paused && !self.control.acknowledge_pause() {
            end = E::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
