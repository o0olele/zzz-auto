//! 体力父流程使用的导航子流程。终态保存在外层回调后才提供到达回执。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, charge_navigation::ChargeNavInputEnd};
use od_core::{
    charge_navigation::{ChargeNavFlow, ChargeNavTarget},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeNavEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeNavSnapshot {
    pub flow: ChargeNavFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct ChargeNavRun {
    snapshot: ChargeNavSnapshot,
    control: PipelineControl,
    end: Option<ChargeNavEnd>,
    steps: u32,
}
impl ChargeNavRun {
    pub fn new(target: ChargeNavTarget, timeout_ms: u32, control: PipelineControl) -> Result<Self> {
        Ok(Self {
            snapshot: ChargeNavSnapshot {
                flow: ChargeNavFlow::new(target, timeout_ms).map_err(anyhow::Error::msg)?,
                active_ms: 0,
                inputs_released: true,
            },
            control,
            end: None,
            steps: 0,
        })
    }
    pub fn snapshot(&self) -> ChargeNavSnapshot {
        self.snapshot.clone()
    }
    pub fn ready(&self) -> bool {
        self.end == Some(ChargeNavEnd::Ready)
            && self.snapshot.flow.ready()
            && self.snapshot.inputs_released
            && self.control.requested().is_none()
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(self.end == Some(ChargeNavEnd::Paused), "体力导航尚未暂停");
        self.resume_after_parent_pause()
    }
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(self.end, Some(ChargeNavEnd::Paused | ChargeNavEnd::Ready))
                && self.snapshot.inputs_released,
            "体力导航尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<ChargeNavEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => ChargeNavEnd::Paused,
            Interrupt::Stop => ChargeNavEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(ChargeNavSnapshot) -> Result<()>,
    ) -> Result<ChargeNavEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(ChargeNavEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "体力导航不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.timeout_ms()).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<ChargeNavEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_charge_navigation()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(ChargeNavEnd::TimedOut);
                }
                if self.snapshot.flow.ready() {
                    return Ok(ChargeNavEnd::Ready);
                }
                ensure!(self.steps < 4096, "体力导航读帧达到上限");
                self.steps += 1;
                let read =
                    session.read_charge_navigation(&self.control.cancellation(), remaining());
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "体力导航读帧后尚未释放输入");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(ChargeNavEnd::TimedOut);
                }
                let Some(view) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let action = self
                    .snapshot
                    .flow
                    .next(view.view())
                    .map_err(anyhow::Error::msg)?;
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
                    .prepare(action, view.view())
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
                    return Ok(self.interrupted().unwrap_or(ChargeNavEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_charge_navigation(
                    &self.snapshot.flow,
                    action,
                    &view,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "体力导航输入后尚未释放输入");
                match input {
                    Ok(ChargeNavInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(ChargeNavInputEnd::NotPosted | ChargeNavInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(ChargeNavInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(ChargeNavEnd::TimedOut);
                    }
                    Err(error) => {
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(ChargeNavEnd::TimedOut);
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
            self.end = Some(ChargeNavEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "体力导航收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(ChargeNavEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = ChargeNavEnd::TimedOut;
        }
        if end == ChargeNavEnd::Paused && !self.control.acknowledge_pause() {
            end = ChargeNavEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
