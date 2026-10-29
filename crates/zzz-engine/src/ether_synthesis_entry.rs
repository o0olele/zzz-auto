//! 父副本调用的合成入口导航，不终结共享控制句柄或写入运行次数。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, ether_synthesis_entry::SynthesisEntryInputEnd};
use od_core::ether_synthesis::SynthesisRequest;
use od_core::{
    ether_synthesis_entry::SynthesisEntryFlow,
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthesisEntryEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesisEntrySnapshot {
    pub flow: SynthesisEntryFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
    pub page_wait_started_ms: Option<u64>,
}
pub struct SynthesisEntryRun {
    snapshot: SynthesisEntrySnapshot,
    control: PipelineControl,
    end: Option<SynthesisEntryEnd>,
    steps: u32,
}
impl SynthesisEntryRun {
    pub fn new(request: SynthesisRequest, control: PipelineControl) -> Result<Self> {
        let flow = SynthesisEntryFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: SynthesisEntrySnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
                page_wait_started_ms: None,
            },
            control,
            end: None,
            steps: 0,
        })
    }
    pub fn snapshot(&self) -> SynthesisEntrySnapshot {
        self.snapshot.clone()
    }
    fn page_wait_started(&self) -> Option<u64> {
        self.snapshot
            .flow
            .awaiting_page()
            .then_some(self.snapshot.page_wait_started_ms)
            .flatten()
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(SynthesisEntryEnd::Paused),
            "合成入口导航尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    /// 父流程在子流程完成之后才暂停时，撤销旧回执并重新读取详情。
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(SynthesisEntryEnd::Paused | SynthesisEntryEnd::Ready)
            ) && self.snapshot.inputs_released,
            "合成入口导航尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<SynthesisEntryEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => SynthesisEntryEnd::Paused,
            Interrupt::Stop => SynthesisEntryEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(SynthesisEntrySnapshot) -> Result<()>,
    ) -> Result<SynthesisEntryEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(SynthesisEntryEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "合成入口导航子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = |page_wait_started: Option<u64>| {
            let total = budget.saturating_sub(started.elapsed());
            page_wait_started.map_or(total, |wait_start| {
                total.min(
                    Duration::from_millis(20_000).saturating_sub(Duration::from_millis(
                        initial
                            .saturating_add(started.elapsed().as_millis() as u64)
                            .saturating_sub(wait_start),
                    )),
                )
            })
        };
        let result = (|| -> Result<SynthesisEntryEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_ether_synthesis_entry()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining(self.page_wait_started()).is_zero() {
                    return Ok(SynthesisEntryEnd::TimedOut);
                }
                if self.snapshot.flow.ready().is_some() {
                    return Ok(SynthesisEntryEnd::Ready);
                }
                ensure!(self.steps < 4096, "合成入口导航步骤达到上限");
                self.steps += 1;
                let read = session.read_ether_synthesis_entry(
                    &self.snapshot.flow,
                    &self.control.cancellation(),
                    remaining(self.page_wait_started()),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "合成入口导航读帧后尚未释放输入"
                );
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining(self.page_wait_started()).is_zero() {
                    return Ok(SynthesisEntryEnd::TimedOut);
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
                let previous_wait = self.snapshot.page_wait_started_ms;
                self.snapshot
                    .flow
                    .prepare(action)
                    .map_err(anyhow::Error::msg)?;
                if action == od_core::ether_synthesis_entry::SynthesisEntryAction::SelectRecipe {
                    self.snapshot.page_wait_started_ms =
                        Some(initial.saturating_add(started.elapsed().as_millis() as u64));
                }
                if let Err(error) = publish(self.snapshot.clone()) {
                    self.snapshot.flow = before;
                    self.snapshot.page_wait_started_ms = previous_wait;
                    return Err(error);
                }
                if remaining(self.page_wait_started()).is_zero() || self.interrupted().is_some() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(SynthesisEntryEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_ether_synthesis_entry(
                    &self.snapshot.flow,
                    action,
                    Some(&view),
                    &self.control.cancellation(),
                    remaining(self.page_wait_started()),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "合成入口导航输入后尚未释放输入"
                );
                match input {
                    Ok(SynthesisEntryInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(SynthesisEntryInputEnd::NotPosted | SynthesisEntryInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(SynthesisEntryInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(SynthesisEntryEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining(self.page_wait_started()).is_zero() {
                            return Ok(SynthesisEntryEnd::TimedOut);
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
            self.end = Some(SynthesisEntryEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "合成入口导航收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(SynthesisEntryEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining(self.page_wait_started()).is_zero() {
            end = SynthesisEntryEnd::TimedOut;
        }
        if end == SynthesisEntryEnd::Paused && !self.control.acknowledge_pause() {
            end = SynthesisEntryEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
