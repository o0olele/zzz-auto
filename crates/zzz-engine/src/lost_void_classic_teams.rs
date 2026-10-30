//! 迷失之地普通父任务的选队准备，共用控制器及剩余预算，不终结父控制句柄。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, simulation::CardsInputEnd, team_selection::SelectionEnd};
use od_core::{
    lost_void_classic_teams::{LostVoidClassicTeamsFlow as TeamsFlow, LostVoidClassicTeamsReceipt},
    pipeline::{Interrupt, PipelineControl},
    simulation_teams::{TeamsStage, TeamsView},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidClassicTeamsEnd {
    Prepared,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidClassicTeamsSnapshot {
    pub flow: TeamsFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct LostVoidClassicTeamsRun {
    snapshot: LostVoidClassicTeamsSnapshot,
    control: PipelineControl,
    end: Option<LostVoidClassicTeamsEnd>,
}
impl LostVoidClassicTeamsRun {
    pub fn snapshot(&self) -> LostVoidClassicTeamsSnapshot {
        self.snapshot.clone()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn receipt(&self) -> Option<&LostVoidClassicTeamsReceipt> {
        if self.control.requested().is_some() {
            return None;
        }
        self.receipt_for_parent()
    }
    pub(crate) fn receipt_for_parent(&self) -> Option<&LostVoidClassicTeamsReceipt> {
        (self.end == Some(LostVoidClassicTeamsEnd::Prepared)
            && self.snapshot.inputs_released
            && self.snapshot.flow.validate().is_ok())
        .then(|| self.snapshot.flow.receipt())
        .flatten()
    }
    pub fn new(flow: TeamsFlow, control: PipelineControl) -> Result<Self> {
        flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(!flow.completed(), "已完成选队检查点不能作为新任务");
        Ok(Self {
            snapshot: LostVoidClassicTeamsSnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
            },
            control,
            end: None,
        })
    }
    fn interrupted(&self) -> Option<LostVoidClassicTeamsEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidClassicTeamsEnd::Paused,
            Interrupt::Stop => LostVoidClassicTeamsEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidClassicTeamsEnd::Paused) && self.snapshot.inputs_released,
            "迷失之地普通选队尚未安全暂停"
        );
        self.resume_after_parent_pause()
    }
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidClassicTeamsEnd::Paused | LostVoidClassicTeamsEnd::Prepared)
            ) && self.snapshot.inputs_released,
            "普通选队尚未安全暂停"
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
        mut publish: impl FnMut(LostVoidClassicTeamsSnapshot) -> Result<()>,
    ) -> Result<LostVoidClassicTeamsEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(LostVoidClassicTeamsEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "迷失之地普通选队不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<LostVoidClassicTeamsEnd> {
            session.require_lost_void_classic_teams()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidClassicTeamsEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(LostVoidClassicTeamsEnd::Prepared);
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
                    ensure!(
                        self.snapshot.inputs_released,
                        "迷失之地普通子选队尚未释放输入"
                    );
                    let child = child?;
                    if let Some(end) = self.interrupted() {
                        return Ok(end);
                    }
                    if remaining().is_zero() {
                        return Ok(LostVoidClassicTeamsEnd::TimedOut);
                    }
                    match child {
                        SelectionEnd::Completed => {
                            self.snapshot.flow.selected().map_err(anyhow::Error::msg)?
                        }
                        SelectionEnd::TimedOut => return Ok(LostVoidClassicTeamsEnd::TimedOut),
                        SelectionEnd::Cancelled => anyhow::bail!("选队取消但父控制句柄没有请求"),
                    }
                } else {
                    let read = session.read_lost_void_classic_teams(
                        &self.snapshot.flow.request().teams,
                        &self.control.cancellation(),
                        remaining(),
                    );
                    self.snapshot.inputs_released = session.release_inputs().is_ok();
                    ensure!(
                        self.snapshot.inputs_released,
                        "迷失之地普通选队读帧后尚未释放输入"
                    );
                    if let Some(end) = self.interrupted() {
                        return Ok(end);
                    }
                    if remaining().is_zero() {
                        return Ok(LostVoidClassicTeamsEnd::TimedOut);
                    }
                    let Some(view) = read? else {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    };
                    ensure!(
                        view != TeamsView::Blocked,
                        "弹窗或管理页不能继续迷失之地普通选队"
                    );
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
                            return Ok(self
                                .interrupted()
                                .unwrap_or(LostVoidClassicTeamsEnd::TimedOut));
                        }
                        self.snapshot.inputs_released = false;
                        let input = session.act_lost_void_classic_teams_open(
                            &self.snapshot.flow,
                            &self.control.cancellation(),
                            remaining(),
                            game,
                        );
                        self.snapshot.inputs_released = session.release_inputs().is_ok();
                        ensure!(
                            self.snapshot.inputs_released,
                            "迷失之地普通打开列表尚未释放输入"
                        );
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
                                return Ok(LostVoidClassicTeamsEnd::TimedOut);
                            }
                            Err(error) => {
                                if let Some(end) = self.interrupted() {
                                    return Ok(end);
                                }
                                if remaining().is_zero() {
                                    return Ok(LostVoidClassicTeamsEnd::TimedOut);
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
        self.end = Some(LostVoidClassicTeamsEnd::Failed);
        ensure!(
            self.snapshot.inputs_released,
            "迷失之地普通选队收尾未释放输入"
        );
        let mut end = result?;
        publish(self.snapshot.clone())?;
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = LostVoidClassicTeamsEnd::TimedOut;
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if end == LostVoidClassicTeamsEnd::Paused && !self.control.acknowledge_pause() {
            end = LostVoidClassicTeamsEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
