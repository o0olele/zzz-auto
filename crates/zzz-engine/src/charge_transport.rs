//! 区域巡防、专业挑战与恶名狩猎的传送，不交接消费许可。
use anyhow::{Context, Result, ensure};
use maa_adapter::{NativeSession, charge_transport::TrainingTransportInputEnd};
use od_core::{
    charge_transport::{TrainingTransportFlow, TrainingTransportRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingTransportEnd {
    AwaitingVerification,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingTransportSnapshot {
    pub flow: TrainingTransportFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct TrainingTransportRun {
    snapshot: TrainingTransportSnapshot,
    control: PipelineControl,
    end: Option<TrainingTransportEnd>,
    steps: u32,
}
pub(crate) struct AgentRouteHandoff<'a> {
    pub route: &'a crate::agent_transport::AgentTransportRun,
    pub goal_bundle: std::path::PathBuf,
}
impl TrainingTransportRun {
    pub(crate) fn from_agent_route(
        request: TrainingTransportRequest,
        agent: &crate::agent_transport::AgentTransportRun,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(
            agent.uses_control(&control),
            "代理人导航与挑战不是同一控制句柄"
        );
        let snapshot = agent.snapshot();
        ensure!(
            snapshot.inputs_released
                && snapshot.flow.completed()
                && snapshot.flow.request().timeout_ms == request.timeout_ms,
            "代理人导航尚未保存、释放或预算不一致"
        );
        let receipt = agent
            .receipt_for_parent()
            .context("尚无本次原生代理人导航回执")?;
        let flow = TrainingTransportFlow::from_agent_route(request, receipt)
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: TrainingTransportSnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
            },
            control,
            end: Some(TrainingTransportEnd::AwaitingVerification),
            steps: 0,
        })
    }
    pub fn snapshot(&self) -> TrainingTransportSnapshot {
        self.snapshot.clone()
    }
    /// 保存、释放及控制边界均正常之后才交回；不能作为挑战消费许可。
    pub fn receipt(&self) -> Option<&od_core::charge_transport::TrainingTransportReceipt> {
        if self.control.requested().is_some() {
            return None;
        }
        self.receipt_for_parent()
    }
    pub(crate) fn receipt_for_parent(
        &self,
    ) -> Option<&od_core::charge_transport::TrainingTransportReceipt> {
        if self.end != Some(TrainingTransportEnd::AwaitingVerification)
            || !self.snapshot.inputs_released
        {
            return None;
        }
        self.snapshot.flow.receipt()
    }
    pub(crate) fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn new(request: TrainingTransportRequest, control: PipelineControl) -> Result<Self> {
        let flow = TrainingTransportFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: TrainingTransportSnapshot {
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
            self.end == Some(TrainingTransportEnd::Paused),
            "体力副本传送准备尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    /// 父流程在子流程完成之后才暂停时，同样撤销旧到达证明。
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(TrainingTransportEnd::Paused | TrainingTransportEnd::AwaitingVerification)
            ) && self.snapshot.inputs_released,
            "体力副本传送准备尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<TrainingTransportEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => TrainingTransportEnd::Paused,
            Interrupt::Stop => TrainingTransportEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(TrainingTransportSnapshot) -> Result<()>,
    ) -> Result<TrainingTransportEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(TrainingTransportEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "体力副本传送子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<TrainingTransportEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_charge_transport()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(TrainingTransportEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(TrainingTransportEnd::AwaitingVerification);
                }
                ensure!(self.steps < 4096, "体力副本传送准备步骤达到上限");
                self.steps += 1;
                let read = session.read_charge_transport(
                    &self.snapshot.flow,
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "体力副本传送读帧后尚未释放输入"
                );
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(TrainingTransportEnd::TimedOut);
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
                    return Ok(self.interrupted().unwrap_or(TrainingTransportEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_charge_transport(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "体力副本传送输入后尚未释放输入"
                );
                match input {
                    Ok(TrainingTransportInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(
                        TrainingTransportInputEnd::NotPosted | TrainingTransportInputEnd::Changed,
                    ) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(TrainingTransportInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(TrainingTransportEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(TrainingTransportEnd::TimedOut);
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
            self.end = Some(TrainingTransportEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "体力副本传送收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(TrainingTransportEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = TrainingTransportEnd::TimedOut;
        }
        if end == TrainingTransportEnd::Paused && !self.control.acknowledge_pause() {
            end = TrainingTransportEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
