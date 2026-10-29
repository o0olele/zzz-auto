//! 父副本调用的专业挑战室结算收尾，不终结共享控制句柄或写入运行次数。
use anyhow::{Context, Result, ensure};
use maa_adapter::{NativeSession, expert_challenge::ExpertInputEnd};
use od_core::{
    agent_goal::{AgentFinishState, AgentSettlementObservation},
    simulation_settlement::SettlementAction,
};
use od_core::{
    expert_challenge::ExpertSettlementFlow,
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpertSettlementEnd {
    Collected,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertSettlementSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_goal: Option<AgentFinishState>,
    pub flow: ExpertSettlementFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct ExpertSettlementRun {
    pub snapshot: ExpertSettlementSnapshot,
    control: PipelineControl,
    end: Option<ExpertSettlementEnd>,
    steps: u32,
}
impl ExpertSettlementRun {
    pub(crate) fn enable_agent_goal(&mut self) -> Result<()> {
        ensure!(
            self.end.is_none()
                && self.snapshot.active_ms == 0
                && self.snapshot.agent_goal.is_none(),
            "培养达成读取只能在结算首次执行前启用"
        );
        self.snapshot.agent_goal = Some(AgentFinishState::default());
        Ok(())
    }

    pub fn after_combat(
        result: crate::combat::CombatSegmentEnd,
        inputs_released: bool,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(inputs_released, "战斗输入未释放，不能交接结算");
        let crate::combat::CombatSegmentEnd::Result(result) = result else {
            anyhow::bail!("战斗未正常结束，不能交接结算");
        };
        Self::new(
            ExpertSettlementFlow::new(result, timeout_ms).map_err(anyhow::Error::msg)?,
            control,
        )
    }
    pub fn new(flow: ExpertSettlementFlow, control: PipelineControl) -> Result<Self> {
        flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            !flow.completed(),
            "已完成的结算不能重新创建任务；恢复时须先撤销旧返回证明"
        );
        Ok(Self {
            snapshot: ExpertSettlementSnapshot {
                agent_goal: None,
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
                Some(ExpertSettlementEnd::Paused | ExpertSettlementEnd::Collected)
            ) && self.snapshot.inputs_released
                && self.control.requested() == Some(Interrupt::Pause),
            "专业挑战室结算准备尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<ExpertSettlementEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => ExpertSettlementEnd::Paused,
            Interrupt::Stop => ExpertSettlementEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(ExpertSettlementSnapshot) -> Result<()>,
    ) -> Result<ExpertSettlementEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(ExpertSettlementEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "专业挑战室结算子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.timeout_ms()).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<ExpertSettlementEnd> {
            session.require_expert_challenge()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(ExpertSettlementEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    ensure!(
                        self.snapshot
                            .agent_goal
                            .as_ref()
                            .is_none_or(|g| g.validate().is_ok() && g.observed().is_some()),
                        "培养结算返回缺少本次完成意图和达成读数"
                    );
                    return Ok(ExpertSettlementEnd::Collected);
                }
                ensure!(self.steps < 4096, "专业挑战室结算准备步骤达到上限");
                self.steps += 1;
                let read = if self.snapshot.agent_goal.is_some() {
                    session.read_expert_challenge_agent_settlement(
                        &self.control.cancellation(),
                        remaining(),
                    )
                } else {
                    session
                        .read_expert_challenge_settlement(&self.control.cancellation(), remaining())
                        .map(|o| o.map(|view| AgentSettlementObservation { view, goal: None }))
                };
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "专业挑战室结算读帧后尚未释放输入"
                );
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(ExpertSettlementEnd::TimedOut);
                }
                let Some(observation) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let view = observation.view;
                let action = self.snapshot.flow.next(&view).map_err(anyhow::Error::msg)?;
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                let Some(action) = action else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let before = self.snapshot.flow.clone();
                let before_goal = self.snapshot.agent_goal.clone();
                self.snapshot
                    .flow
                    .prepare(action)
                    .map_err(anyhow::Error::msg)?;
                if action == SettlementAction::Finish
                    && let Some(goal) = self.snapshot.agent_goal.as_mut()
                {
                    goal.prepare(observation.goal.context("本次培养结算缺少达成读取")?)
                        .map_err(anyhow::Error::msg)?;
                }
                if let Err(error) = publish(self.snapshot.clone()) {
                    self.snapshot.flow = before;
                    self.snapshot.agent_goal = before_goal;
                    return Err(error);
                }
                if remaining().is_zero() || self.interrupted().is_some() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    if action == SettlementAction::Finish
                        && let Some(goal) = self.snapshot.agent_goal.as_mut()
                    {
                        goal.not_posted().map_err(anyhow::Error::msg)?;
                    }
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(ExpertSettlementEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input =
                    if action == SettlementAction::Finish && self.snapshot.agent_goal.is_some() {
                        session.act_expert_challenge_agent_finish(
                            &self.snapshot.flow,
                            self.snapshot
                                .agent_goal
                                .as_ref()
                                .and_then(AgentFinishState::expected)
                                .context("培养完成意图缺少读数")?,
                            &self.control.cancellation(),
                            remaining(),
                            game,
                        )
                    } else {
                        session.act_expert_challenge_settlement(
                            &self.snapshot.flow,
                            action,
                            &self.control.cancellation(),
                            remaining(),
                            game,
                        )
                    };
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "专业挑战室结算输入后尚未释放输入"
                );
                if action == SettlementAction::Finish
                    && let Some(goal) = self.snapshot.agent_goal.as_mut()
                {
                    match &input {
                        Ok(ExpertInputEnd::Posted) => goal.posted().map_err(anyhow::Error::msg)?,
                        Ok(
                            ExpertInputEnd::NotPosted
                            | ExpertInputEnd::Changed
                            | ExpertInputEnd::TimedOut,
                        ) => goal.not_posted().map_err(anyhow::Error::msg)?,
                        Err(_) => (),
                    }
                }
                match input {
                    Ok(ExpertInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(ExpertInputEnd::NotPosted | ExpertInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(ExpertInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(ExpertSettlementEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(ExpertSettlementEnd::TimedOut);
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
            self.end = Some(ExpertSettlementEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "专业挑战室结算收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(ExpertSettlementEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = ExpertSettlementEnd::TimedOut;
        }
        if end == ExpertSettlementEnd::Paused && !self.control.acknowledge_pause() {
            end = ExpertSettlementEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
