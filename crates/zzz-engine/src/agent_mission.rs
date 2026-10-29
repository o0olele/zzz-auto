//! 模拟室培养详情选择，不交接消费许可。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, agent_mission::AgentMissionInputEnd};
use od_core::{
    agent_mission::{AgentMissionFlow, AgentMissionRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMissionEnd {
    Unavailable,
    AwaitingVerification,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMissionSnapshot {
    pub flow: AgentMissionFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct AgentMissionRun {
    snapshot: AgentMissionSnapshot,
    control: PipelineControl,
    end: Option<AgentMissionEnd>,
    steps: u32,
}
impl AgentMissionRun {
    pub fn snapshot(&self) -> AgentMissionSnapshot {
        self.snapshot.clone()
    }
    /// 保存、释放及控制边界均正常之后才交回；不能作为挑战消费许可。
    pub fn receipt(&self) -> Option<&od_core::agent_mission::AgentMissionReceipt> {
        if self.control.requested().is_some() {
            return None;
        }
        self.receipt_for_parent()
    }
    pub(crate) fn receipt_for_parent(
        &self,
    ) -> Option<&od_core::agent_mission::AgentMissionReceipt> {
        if self.end != Some(AgentMissionEnd::AwaitingVerification) || !self.snapshot.inputs_released
        {
            return None;
        }
        self.snapshot.flow.receipt()
    }
    /// 供父流程核对共享控制句柄，不提供页面或执行证明。
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn new(request: AgentMissionRequest, control: PipelineControl) -> Result<Self> {
        let flow = AgentMissionFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: AgentMissionSnapshot {
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
            self.end == Some(AgentMissionEnd::Paused),
            "代理人模拟室选关准备尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    /// 父流程在子流程完成之后才暂停时，同样撤销旧到达证明。
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(AgentMissionEnd::Paused | AgentMissionEnd::AwaitingVerification)
            ) && self.snapshot.inputs_released,
            "代理人模拟室选关准备尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<AgentMissionEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => AgentMissionEnd::Paused,
            Interrupt::Stop => AgentMissionEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(AgentMissionSnapshot) -> Result<()>,
    ) -> Result<AgentMissionEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(AgentMissionEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "代理人模拟室选关子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<AgentMissionEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_agent_mission()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(AgentMissionEnd::TimedOut);
                }
                if self.snapshot.flow.unavailable() {
                    return Ok(AgentMissionEnd::Unavailable);
                }
                if self.snapshot.flow.completed() {
                    return Ok(AgentMissionEnd::AwaitingVerification);
                }
                ensure!(self.steps < 4096, "代理人模拟室选关准备步骤达到上限");
                self.steps += 1;
                let read = session.read_agent_mission(
                    self.snapshot.flow.request(),
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "代理人模拟室选关读帧后尚未释放输入"
                );
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(AgentMissionEnd::TimedOut);
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
                    return Ok(self.interrupted().unwrap_or(AgentMissionEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_agent_mission(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "代理人模拟室选关输入后尚未释放输入"
                );
                match input {
                    Ok(AgentMissionInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(AgentMissionInputEnd::NotPosted | AgentMissionInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(AgentMissionInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(AgentMissionEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(AgentMissionEnd::TimedOut);
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
            self.end = Some(AgentMissionEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "代理人模拟室选关收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(AgentMissionEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = AgentMissionEnd::TimedOut;
        }
        if end == AgentMissionEnd::Paused && !self.control.acknowledge_pause() {
            end = AgentMissionEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
