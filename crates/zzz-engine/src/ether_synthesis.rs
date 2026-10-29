//! 父副本调用的电池合成，不终结共享控制句柄或写入运行次数。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, ether_synthesis::SynthesisInputEnd};
use od_core::{
    ether_synthesis::{SynthesisFlow, SynthesisRequest, SynthesisResult},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthesisEnd {
    Completed,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesisSnapshot {
    pub flow: SynthesisFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct SynthesisRun {
    id: uuid::Uuid,
    execution_id: uuid::Uuid,
    snapshot: SynthesisSnapshot,
    control: PipelineControl,
    end: Option<SynthesisEnd>,
    steps: u32,
}
impl SynthesisRun {
    pub fn new(
        id: uuid::Uuid,
        request: SynthesisRequest,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(!id.is_nil(), "合成缺少父任务身份");
        let flow = SynthesisFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            id,
            execution_id: uuid::Uuid::new_v4(),
            snapshot: SynthesisSnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
            },
            control,
            end: None,
            steps: 0,
        })
    }
    pub fn snapshot(&self) -> SynthesisSnapshot {
        self.snapshot.clone()
    }
    pub fn id(&self) -> uuid::Uuid {
        self.id
    }
    pub fn execution_id(&self) -> uuid::Uuid {
        self.execution_id
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn proof(&self) -> Result<SynthesisResult> {
        ensure!(
            self.control.requested().is_none(),
            "合成存在控制请求，不能交接本次结果"
        );
        self.result_for_parent()
    }
    /// 父任务转换只读取已保存结果；并发控制请求由父任务统一收尾，不能将暂停误报失败。
    pub(crate) fn result_for_parent(&self) -> Result<SynthesisResult> {
        ensure!(
            self.end == Some(SynthesisEnd::Completed) && self.snapshot.inputs_released,
            "合成结果尚未保存、释放或存在控制请求"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        self.snapshot
            .flow
            .result()
            .ok_or_else(|| anyhow::anyhow!("合成没有本次结果"))
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(self.end == Some(SynthesisEnd::Paused), "电池合成尚未暂停");
        self.resume_after_parent_pause()
    }
    /// 父流程在子流程完成之后才暂停时，重新读取合成页资源。
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(SynthesisEnd::Paused | SynthesisEnd::Completed)
            ) && self.snapshot.inputs_released,
            "电池合成尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<SynthesisEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => SynthesisEnd::Paused,
            Interrupt::Stop => SynthesisEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(SynthesisSnapshot) -> Result<()>,
    ) -> Result<SynthesisEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(SynthesisEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "电池合成子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<SynthesisEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_ether_synthesis()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(SynthesisEnd::TimedOut);
                }
                if self.snapshot.flow.result().is_some() {
                    return Ok(SynthesisEnd::Completed);
                }
                ensure!(self.steps < 4096, "电池合成步骤达到上限");
                self.steps += 1;
                let read = session.read_ether_synthesis(&self.control.cancellation(), remaining());
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "电池合成读帧后尚未释放输入");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(SynthesisEnd::TimedOut);
                }
                let Some(view) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let action = self.snapshot.flow.next(view).map_err(anyhow::Error::msg)?;
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
                    return Ok(self.interrupted().unwrap_or(SynthesisEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_ether_synthesis(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "电池合成输入后尚未释放输入");
                match input {
                    Ok(SynthesisInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(SynthesisInputEnd::NotPosted | SynthesisInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(SynthesisInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(SynthesisEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(SynthesisEnd::TimedOut);
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
            self.end = Some(SynthesisEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "电池合成收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(SynthesisEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = SynthesisEnd::TimedOut;
        }
        if end == SynthesisEnd::Paused && !self.control.acknowledge_pause() {
            end = SynthesisEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
