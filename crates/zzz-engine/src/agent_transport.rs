//! 四种训练分类的代理人培养传送，不交接消费许可。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, agent_transport::AgentTransportInputEnd};
use od_core::{
    agent_transport::{AgentTransportFlow, AgentTransportRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTransportEnd {
    Unavailable,
    AwaitingVerification,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTransportSnapshot {
    pub flow: AgentTransportFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct AgentTransportRun {
    snapshot: AgentTransportSnapshot,
    control: PipelineControl,
    end: Option<AgentTransportEnd>,
    steps: u32,
}
impl AgentTransportRun {
    pub fn snapshot(&self) -> AgentTransportSnapshot {
        self.snapshot.clone()
    }
    /// 保存、释放及控制边界均正常之后才交回；不能作为挑战消费许可。
    pub fn receipt(&self) -> Option<&od_core::agent_transport::AgentTransportReceipt> {
        if self.control.requested().is_some() {
            return None;
        }
        self.receipt_for_parent()
    }
    pub(crate) fn receipt_for_parent(
        &self,
    ) -> Option<&od_core::agent_transport::AgentTransportReceipt> {
        if self.end != Some(AgentTransportEnd::AwaitingVerification)
            || !self.snapshot.inputs_released
        {
            return None;
        }
        self.snapshot.flow.receipt()
    }
    /// 供父流程核对共享控制句柄，不提供页面或执行证明。
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn new(request: AgentTransportRequest, control: PipelineControl) -> Result<Self> {
        let flow = AgentTransportFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: AgentTransportSnapshot {
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
            self.end == Some(AgentTransportEnd::Paused),
            "代理人副本传送准备尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    /// 父流程在子流程完成之后才暂停时，同样撤销旧到达证明。
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(AgentTransportEnd::Paused | AgentTransportEnd::AwaitingVerification)
            ) && self.snapshot.inputs_released,
            "代理人副本传送准备尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<AgentTransportEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => AgentTransportEnd::Paused,
            Interrupt::Stop => AgentTransportEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(AgentTransportSnapshot) -> Result<()>,
    ) -> Result<AgentTransportEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(AgentTransportEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "代理人副本传送子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<AgentTransportEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_agent_transport()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(AgentTransportEnd::TimedOut);
                }
                if self.snapshot.flow.unavailable() {
                    return Ok(AgentTransportEnd::Unavailable);
                }
                if self.snapshot.flow.completed() {
                    return Ok(AgentTransportEnd::AwaitingVerification);
                }
                ensure!(self.steps < 4096, "代理人副本传送准备步骤达到上限");
                self.steps += 1;
                let read = session.read_agent_transport(
                    &self.snapshot.flow,
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "代理人副本传送读帧后尚未释放输入"
                );
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(AgentTransportEnd::TimedOut);
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
                    return Ok(self.interrupted().unwrap_or(AgentTransportEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_agent_transport(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "代理人副本传送输入后尚未释放输入"
                );
                match input {
                    Ok(AgentTransportInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(AgentTransportInputEnd::NotPosted | AgentTransportInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(AgentTransportInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(AgentTransportEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(AgentTransportEnd::TimedOut);
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
            self.end = Some(AgentTransportEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "代理人副本传送收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(AgentTransportEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = AgentTransportEnd::TimedOut;
        }
        if end == AgentTransportEnd::Paused && !self.control.acknowledge_pause() {
            end = AgentTransportEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
