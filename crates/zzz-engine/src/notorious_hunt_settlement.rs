//! 周常单战完成、奖励确认和两帧免费余量核对，共用父控制器和预算。
use anyhow::{Context, Result, ensure};
use maa_adapter::{NativeSession, deep_hunt::HuntInputEnd};
use od_core::{
    agent_goal::AgentFinishState,
    notorious_hunt_challenge::WeeklySettlementFlow,
    pipeline::{Interrupt, PipelineControl},
    simulation_settlement::SettlementAction,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeeklySettlementEnd {
    Collected,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklySettlementSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_goal: Option<AgentFinishState>,
    pub flow: WeeklySettlementFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct WeeklySettlementRun {
    pub snapshot: WeeklySettlementSnapshot,
    control: PipelineControl,
    end: Option<WeeklySettlementEnd>,
    steps: u32,
}
impl WeeklySettlementRun {
    pub(crate) fn enable_agent_goal(&mut self) -> Result<()> {
        ensure!(
            self.end.is_none()
                && self.snapshot.active_ms == 0
                && self.snapshot.agent_goal.is_none(),
            "周常培养读数只能在首次结算前启用"
        );
        self.snapshot.agent_goal = Some(AgentFinishState::default());
        Ok(())
    }
    pub fn after_combat(
        result: crate::combat::CombatSegmentEnd,
        inputs_released: bool,
        timeout_ms: u32,
        mission_type: String,
        rewards_before: u8,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(inputs_released, "战斗输入未释放，不能交接周常结算");
        let crate::combat::CombatSegmentEnd::Result(result) = result else {
            anyhow::bail!("战斗未正常结束，不能交接周常结算");
        };
        Self::new(
            WeeklySettlementFlow::new(result, timeout_ms, mission_type, rewards_before)
                .map_err(anyhow::Error::msg)?,
            control,
        )
    }
    pub fn new(flow: WeeklySettlementFlow, control: PipelineControl) -> Result<Self> {
        flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(!flow.completed(), "已完成周常结算不能重新创建任务");
        Ok(Self {
            snapshot: WeeklySettlementSnapshot {
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
                Some(WeeklySettlementEnd::Paused | WeeklySettlementEnd::Collected)
            ) && self.snapshot.inputs_released
                && self.control.requested() == Some(Interrupt::Pause),
            "周常结算尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止不能继续");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<WeeklySettlementEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => WeeklySettlementEnd::Paused,
            Interrupt::Stop => WeeklySettlementEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        n: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(WeeklySettlementSnapshot) -> Result<()>,
    ) -> Result<WeeklySettlementEnd> {
        use WeeklySettlementEnd as E;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "周常结算不能直接重跑"
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
                    ensure!(
                        self.snapshot
                            .agent_goal
                            .as_ref()
                            .is_none_or(|g| g.validate().is_ok() && g.observed().is_some()),
                        "周常培养返回缺少本次达成读数"
                    );
                    return Ok(E::Collected);
                }
                ensure!(self.steps < 4096, "周常结算步骤达到上限");
                self.steps += 1;
                let read = if self.snapshot.agent_goal.is_some() {
                    n.read_weekly_hunt_agent_settlement(
                        self.snapshot.flow.mission_type(),
                        &self.control.cancellation(),
                        remaining(),
                    )
                } else {
                    n.read_weekly_hunt_settlement(
                        self.snapshot.flow.mission_type(),
                        &self.control.cancellation(),
                        remaining(),
                    )
                    .map(|o| o.map(|v| (v, None)))
                };
                self.snapshot.inputs_released = n.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "周常结算读帧后尚未释放输入");
                if let Some(e) = self.interrupted() {
                    return Ok(e);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                let Some((view, goal_reading)) = read? else {
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
                let before_goal = self.snapshot.agent_goal.clone();
                self.snapshot
                    .flow
                    .prepare(action)
                    .map_err(anyhow::Error::msg)?;
                if action == SettlementAction::Finish
                    && let Some(goal) = &mut self.snapshot.agent_goal
                {
                    goal.prepare(goal_reading.context("本次周常培养结算缺少达成读数")?)
                        .map_err(anyhow::Error::msg)?;
                }
                if let Err(e) = publish(self.snapshot.clone()) {
                    self.snapshot.flow = before;
                    self.snapshot.agent_goal = before_goal;
                    return Err(e);
                }
                if remaining().is_zero() || self.interrupted().is_some() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    if action == SettlementAction::Finish
                        && let Some(goal) = &mut self.snapshot.agent_goal
                    {
                        goal.not_posted().map_err(anyhow::Error::msg)?;
                    }
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(E::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input =
                    if action == SettlementAction::Finish && self.snapshot.agent_goal.is_some() {
                        n.act_weekly_hunt_agent_finish(
                            &self.snapshot.flow,
                            self.snapshot
                                .agent_goal
                                .as_ref()
                                .and_then(AgentFinishState::expected)
                                .context("周常培养完成意图缺少达成读数")?,
                            &self.control.cancellation(),
                            remaining(),
                            game,
                        )
                    } else {
                        n.act_weekly_hunt_settlement(
                            &self.snapshot.flow,
                            action,
                            &self.control.cancellation(),
                            remaining(),
                            game,
                        )
                    };
                self.snapshot.inputs_released = n.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "周常结算输入后尚未释放输入");
                if action == SettlementAction::Finish
                    && let Some(goal) = &mut self.snapshot.agent_goal
                {
                    match &input {
                        Ok(HuntInputEnd::Posted) => goal.posted().map_err(anyhow::Error::msg)?,
                        Ok(
                            HuntInputEnd::NotPosted
                            | HuntInputEnd::Changed
                            | HuntInputEnd::TimedOut,
                        ) => goal.not_posted().map_err(anyhow::Error::msg)?,
                        Err(_) => (),
                    }
                }
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
        ensure!(self.snapshot.inputs_released, "周常结算收尾未释放输入");
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
