//! 完整 HUD 队伍及移动前台准备，共用父控制、剩余预算和保存边界。
use anyhow::{Result, ensure};
use maa_adapter::{
    NativeSession,
    lost_void_roster::{LostVoidHudReader, RosterInputEnd},
};
use od_core::{
    lost_void_roster::{RosterFlow, RosterReceipt, RosterRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidRosterEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
    ObservedPage(od_core::lost_void_scene::LostVoidScreen),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidRosterSnapshot {
    pub flow: RosterFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
    #[serde(default)]
    pub observed_page: Option<od_core::lost_void_scene::LostVoidScreen>,
}
pub struct LostVoidRosterRun {
    snapshot: LostVoidRosterSnapshot,
    control: PipelineControl,
    end: Option<LostVoidRosterEnd>,
    steps: u32,
    page_handoff: bool,
    page_reads: Option<(od_core::lost_void_scene::LostVoidScreen, u8)>,
}
impl LostVoidRosterRun {
    pub fn new(request: RosterRequest, control: PipelineControl) -> Result<Self> {
        Ok(Self {
            snapshot: LostVoidRosterSnapshot {
                flow: RosterFlow::new(request).map_err(anyhow::Error::msg)?,
                active_ms: 0,
                inputs_released: true,
                observed_page: None,
            },
            control,
            end: None,
            steps: 0,
            page_handoff: false,
            page_reads: None,
        })
    }
    pub fn snapshot(&self) -> LostVoidRosterSnapshot {
        self.snapshot.clone()
    }
    pub fn with_page_handoff(mut self) -> Result<Self> {
        ensure!(
            self.steps == 0 && self.end.is_none(),
            "队伍运行后不能更改页面交接策略"
        );
        self.page_handoff = true;
        Ok(self)
    }
    pub(crate) fn continue_after_page(&mut self) -> Result<()> {
        ensure!(
            matches!(self.end, Some(LostVoidRosterEnd::ObservedPage(_)))
                && self.snapshot.inputs_released
                && self.control.requested().is_none(),
            "队伍尚未安全交回内容页"
        );
        self.snapshot.flow.resume().map_err(anyhow::Error::msg)?;
        self.snapshot.observed_page = None;
        self.page_reads = None;
        self.end = None;
        Ok(())
    }
    pub fn request(&self) -> &RosterRequest {
        self.snapshot.flow.request()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn receipt(&self) -> Option<&RosterReceipt> {
        if self.end != Some(LostVoidRosterEnd::Ready)
            || !self.snapshot.inputs_released
            || self.control.requested().is_some()
        {
            None
        } else {
            self.snapshot.flow.receipt()
        }
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidRosterEnd::Paused),
            "实际队伍子流程尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(
                    LostVoidRosterEnd::Paused
                        | LostVoidRosterEnd::Ready
                        | LostVoidRosterEnd::ObservedPage(_)
                )
            ) && self.snapshot.inputs_released,
            "实际队伍尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.snapshot.observed_page = None;
        self.page_reads = None;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidRosterEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidRosterEnd::Paused,
            Interrupt::Stop => LostVoidRosterEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        reader: &mut LostVoidHudReader,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidRosterSnapshot) -> Result<()>,
    ) -> Result<LostVoidRosterEnd> {
        ensure!(
            self.end.is_none()
                || self.end == Some(LostVoidRosterEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop),
            "实际队伍子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget =
            Duration::from_millis(u64::from(self.request().timeout_ms).saturating_sub(initial))
                .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<LostVoidRosterEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_lost_void_roster(reader)?;
            reader.reset();
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidRosterEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(LostVoidRosterEnd::Ready);
                }
                ensure!(self.steps < 4096, "实际队伍观察达到上限");
                self.steps += 1;
                let read = session.read_lost_void_roster(
                    &self.snapshot.flow,
                    reader,
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "实际队伍读帧后未释放输入");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidRosterEnd::TimedOut);
                }
                let Some(view) = read? else {
                    self.page_reads = None;
                    self.snapshot.flow.invalidate_observation();
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                if self.page_handoff && od_core::lost_void_scene::level_content(view.screen) {
                    self.snapshot.flow.invalidate_observation();
                    let reads = if self.page_reads.is_some_and(|(s, _)| s == view.screen) {
                        (self.page_reads.unwrap().1 + 1).min(2)
                    } else {
                        1
                    };
                    self.page_reads = Some((view.screen, reads));
                    if reads == 2 {
                        self.snapshot.observed_page = Some(view.screen);
                        return Ok(LostVoidRosterEnd::ObservedPage(view.screen));
                    }
                    continue;
                }
                self.page_reads = None;
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
                if self.interrupted().is_some() || remaining().is_zero() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(LostVoidRosterEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_lost_void_roster(
                    &self.snapshot.flow,
                    action,
                    reader,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "实际队伍输入后未释放输入");
                match input {
                    Ok(RosterInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(RosterInputEnd::NotPosted | RosterInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(RosterInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(LostVoidRosterEnd::TimedOut);
                    }
                    Err(error) => {
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(LostVoidRosterEnd::TimedOut);
                        }
                        return Err(error);
                    }
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                let settle = Instant::now();
                while settle.elapsed() < Duration::from_millis(200)
                    && self.interrupted().is_none()
                    && !remaining().is_zero()
                {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        })();
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() || !self.snapshot.inputs_released {
            self.end = Some(LostVoidRosterEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "实际队伍收尾无法确认释放输入"
        );
        let mut end = result?;
        let saved = publish(self.snapshot.clone());
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Err(error) = saved {
            self.end = Some(LostVoidRosterEnd::Failed);
            return Err(error);
        }
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = LostVoidRosterEnd::TimedOut;
        }
        if end == LostVoidRosterEnd::Paused && !self.control.acknowledge_pause() {
            end = LostVoidRosterEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
