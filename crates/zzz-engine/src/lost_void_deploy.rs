//! 迷失之地本次出战到真实局内页面，共用父控制、剩余预算和保存边界。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, lost_void_deploy::DeployInputEnd};
use od_core::{
    lost_void_deploy::{LostVoidDeployFlow, LostVoidDeployReceipt, LostVoidDeployRequest},
    lost_void_matrix::LostVoidMatrixFlow,
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidDeployEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidDeploySnapshot {
    pub flow: LostVoidDeployFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct LostVoidDeployRun {
    snapshot: LostVoidDeploySnapshot,
    control: PipelineControl,
    end: Option<LostVoidDeployEnd>,
    steps: u32,
    matrix: Option<LostVoidMatrixFlow>,
}
impl LostVoidDeployRun {
    pub fn new(
        request: LostVoidDeployRequest,
        matrix: Option<LostVoidMatrixFlow>,
        control: PipelineControl,
    ) -> Result<Self> {
        match (&request.party, &matrix) {
            (od_core::lost_void_deploy::DeploymentParty::Matrix { receipt }, Some(m)) => {
                m.validate().map_err(anyhow::Error::msg)?;
                ensure!(m.receipt() == Some(receipt), "出战缺少同次矩阵编队检查点");
            }
            (od_core::lost_void_deploy::DeploymentParty::Classic { .. }, None) => {}
            _ => anyhow::bail!("出战与矩阵编队检查点不一致"),
        }
        Ok(Self {
            snapshot: LostVoidDeploySnapshot {
                flow: LostVoidDeployFlow::new(request).map_err(anyhow::Error::msg)?,
                active_ms: 0,
                inputs_released: true,
            },
            control,
            end: None,
            steps: 0,
            matrix,
        })
    }
    pub fn snapshot(&self) -> LostVoidDeploySnapshot {
        self.snapshot.clone()
    }
    pub fn request(&self) -> &LostVoidDeployRequest {
        self.snapshot.flow.request()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn receipt(&self) -> Option<&LostVoidDeployReceipt> {
        if self.control.requested().is_some() {
            return None;
        }
        self.receipt_for_parent()
    }
    pub(crate) fn receipt_for_parent(&self) -> Option<&LostVoidDeployReceipt> {
        if self.end != Some(LostVoidDeployEnd::Ready) || !self.snapshot.inputs_released {
            None
        } else {
            self.snapshot.flow.receipt()
        }
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidDeployEnd::Paused),
            "迷失之地出战子流程尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidDeployEnd::Paused | LostVoidDeployEnd::Ready)
            ) && self.snapshot.inputs_released,
            "迷失之地出战尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidDeployEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidDeployEnd::Paused,
            Interrupt::Stop => LostVoidDeployEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidDeploySnapshot) -> Result<()>,
    ) -> Result<LostVoidDeployEnd> {
        ensure!(
            self.end.is_none()
                || self.end == Some(LostVoidDeployEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop),
            "迷失之地出战子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget =
            Duration::from_millis(u64::from(self.request().timeout_ms).saturating_sub(initial))
                .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<LostVoidDeployEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_lost_void_scene()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidDeployEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(LostVoidDeployEnd::Ready);
                }
                ensure!(self.steps < 4096, "迷失之地出战观察达到上限");
                self.steps += 1;
                let read = session.read_lost_void_deploy(
                    &self.snapshot.flow,
                    self.matrix.as_ref(),
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "迷失之地出战读帧后未释放输入"
                );
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidDeployEnd::TimedOut);
                }
                let Some(view) = read? else {
                    self.snapshot.flow.invalidate_observation();
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
                if self.interrupted().is_some() || remaining().is_zero() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(LostVoidDeployEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_lost_void_deploy(
                    &self.snapshot.flow,
                    action,
                    self.matrix.as_ref(),
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "迷失之地出战输入后未释放输入"
                );
                match input {
                    Ok(DeployInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(DeployInputEnd::NotPosted | DeployInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(DeployInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(LostVoidDeployEnd::TimedOut);
                    }
                    Err(error) => {
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(LostVoidDeployEnd::TimedOut);
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
            self.end = Some(LostVoidDeployEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "迷失之地出战收尾无法确认释放输入"
        );
        let mut end = result?;
        let saved = publish(self.snapshot.clone());
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Err(error) = saved {
            self.end = Some(LostVoidDeployEnd::Failed);
            return Err(error);
        }
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = LostVoidDeployEnd::TimedOut;
        }
        if end == LostVoidDeployEnd::Paused && !self.control.acknowledge_pause() {
            end = LostVoidDeployEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
