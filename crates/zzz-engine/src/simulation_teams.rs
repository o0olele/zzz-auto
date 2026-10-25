//! 模拟室父任务的选队准备，共用控制器及剩余预算，不终结父控制句柄。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, simulation::CardsInputEnd, team_selection::SelectionEnd};
use od_core::{
    pipeline::{Interrupt, PipelineControl},
    simulation_teams::{TeamsFlow, TeamsStage, TeamsView},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamsEnd {
    Prepared,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamsSnapshot {
    pub flow: TeamsFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct TeamsRun {
    pub snapshot: TeamsSnapshot,
    control: PipelineControl,
    end: Option<TeamsEnd>,
}
impl TeamsRun {
    pub fn new(flow: TeamsFlow, control: PipelineControl) -> Result<Self> {
        flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(!flow.completed(), "已完成选队检查点不能作为新任务");
        Ok(Self {
            snapshot: TeamsSnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
            },
            control,
            end: None,
        })
    }
    fn interrupted(&self) -> Option<TeamsEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => TeamsEnd::Paused,
            Interrupt::Stop => TeamsEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(TeamsEnd::Paused) && self.snapshot.inputs_released,
            "模拟室选队尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(TeamsSnapshot) -> Result<()>,
    ) -> Result<TeamsEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(TeamsEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "模拟室选队不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<TeamsEnd> {
            session.require_simulation_teams()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(TeamsEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(TeamsEnd::Prepared);
                }
                if self.snapshot.flow.stage() == TeamsStage::Selecting {
                    let mut selection = self.snapshot.flow.selection().unwrap().clone();
                    let teams = self.snapshot.flow.request().teams.clone();
                    self.snapshot.inputs_released = false;
                    let child = session.select_predefined_teams(
                        &teams,
                        &mut selection,
                        game,
                        self.control.cancellation(),
                        remaining(),
                        |s| {
                            self.snapshot
                                .flow
                                .update_selection(s)
                                .map_err(anyhow::Error::msg)?;
                            self.snapshot.active_ms =
                                initial.saturating_add(started.elapsed().as_millis() as u64);
                            publish(self.snapshot.clone())
                        },
                    );
                    self.snapshot
                        .flow
                        .update_selection(selection)
                        .map_err(anyhow::Error::msg)?;
                    self.snapshot.inputs_released = session.release_inputs().is_ok();
                    ensure!(self.snapshot.inputs_released, "模拟室子选队尚未释放输入");
                    let child = child?;
                    if let Some(end) = self.interrupted() {
                        return Ok(end);
                    }
                    if remaining().is_zero() {
                        return Ok(TeamsEnd::TimedOut);
                    }
                    match child {
                        SelectionEnd::Completed => {
                            self.snapshot.flow.selected().map_err(anyhow::Error::msg)?
                        }
                        SelectionEnd::TimedOut => return Ok(TeamsEnd::TimedOut),
                        SelectionEnd::Cancelled => anyhow::bail!("选队取消但父控制句柄没有请求"),
                    }
                } else {
                    let read = session.read_simulation_teams(
                        &self.snapshot.flow.request().teams,
                        &self.control.cancellation(),
                        remaining(),
                    );
                    self.snapshot.inputs_released = session.release_inputs().is_ok();
                    ensure!(
                        self.snapshot.inputs_released,
                        "模拟室选队读帧后尚未释放输入"
                    );
                    if let Some(end) = self.interrupted() {
                        return Ok(end);
                    }
                    if remaining().is_zero() {
                        return Ok(TeamsEnd::TimedOut);
                    }
                    let Some(view) = read? else {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    };
                    ensure!(view != TeamsView::Blocked, "弹窗或管理页不能继续模拟室选队");
                    let open = self
                        .snapshot
                        .flow
                        .observe(view)
                        .map_err(anyhow::Error::msg)?;
                    if open {
                        let before = self.snapshot.flow.clone();
                        self.snapshot
                            .flow
                            .prepare_open()
                            .map_err(anyhow::Error::msg)?;
                        self.snapshot.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        if let Err(error) = publish(self.snapshot.clone()) {
                            self.snapshot.flow = before;
                            return Err(error);
                        }
                        if self.interrupted().is_some() || remaining().is_zero() {
                            self.snapshot
                                .flow
                                .not_posted_open()
                                .map_err(anyhow::Error::msg)?;
                            return Ok(self.interrupted().unwrap_or(TeamsEnd::TimedOut));
                        }
                        self.snapshot.inputs_released = false;
                        let input = session.act_simulation_teams_open(
                            &self.snapshot.flow,
                            &self.control.cancellation(),
                            remaining(),
                            game,
                        );
                        self.snapshot.inputs_released = session.release_inputs().is_ok();
                        ensure!(self.snapshot.inputs_released, "模拟室打开列表尚未释放输入");
                        match input {
                            Ok(CardsInputEnd::Posted) => self
                                .snapshot
                                .flow
                                .posted_open()
                                .map_err(anyhow::Error::msg)?,
                            Ok(CardsInputEnd::NotPosted | CardsInputEnd::Changed) => self
                                .snapshot
                                .flow
                                .not_posted_open()
                                .map_err(anyhow::Error::msg)?,
                            Ok(CardsInputEnd::TimedOut) => {
                                self.snapshot
                                    .flow
                                    .not_posted_open()
                                    .map_err(anyhow::Error::msg)?;
                                return Ok(TeamsEnd::TimedOut);
                            }
                            Err(error) => {
                                if let Some(end) = self.interrupted() {
                                    return Ok(end);
                                }
                                if remaining().is_zero() {
                                    return Ok(TeamsEnd::TimedOut);
                                }
                                return Err(error);
                            }
                        }
                    }
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        self.end = Some(TeamsEnd::Failed);
        ensure!(self.snapshot.inputs_released, "模拟室选队收尾未释放输入");
        let mut end = result?;
        publish(self.snapshot.clone())?;
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = TeamsEnd::TimedOut;
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if end == TeamsEnd::Paused && !self.control.acknowledge_pause() {
            end = TeamsEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
