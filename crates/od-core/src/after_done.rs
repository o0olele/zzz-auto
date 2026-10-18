use crate::{RunRecord, RunState, Snapshot, group_run::GroupStepState, run_plan::RunContext};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AfterDoneAction {
    #[default]
    None,
    CloseGame,
    Shutdown,
    Hibernate,
    Sleep,
}

impl AfterDoneAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "无",
            Self::CloseGame => "关闭游戏",
            Self::Shutdown => "关机",
            Self::Hibernate => "休眠",
            Self::Sleep => "睡眠",
        }
    }

    pub fn options() -> serde_json::Value {
        serde_json::Value::Array(
            [
                Self::None,
                Self::CloseGame,
                Self::Shutdown,
                Self::Hibernate,
                Self::Sleep,
            ]
            .into_iter()
            .map(|value| serde_json::json!({"value": value, "label": value.label()}))
            .collect(),
        )
    }

    pub fn from_legacy(value: &str) -> Option<Self> {
        Some(match value {
            "无" => Self::None,
            "关闭游戏" => Self::CloseGame,
            "关机" => Self::Shutdown,
            "休眠" => Self::Hibernate,
            "睡眠" => Self::Sleep,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AfterDoneState {
    Watching,
    CountingDown,
    Executing,
    Succeeded,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AfterDoneStatus {
    pub id: Uuid,
    pub run_id: Uuid,
    pub action: AfterDoneAction,
    pub state: AfterDoneState,
    pub remaining_seconds: u32,
    pub message: String,
}

#[derive(Default)]
pub struct AfterDoneTracker {
    status: Option<AfterDoneStatus>,
    session: Option<Uuid>,
    context: Option<RunContext>,
    deadline: Option<Instant>,
    observed_at: Option<Instant>,
}

impl AfterDoneTracker {
    pub const COUNTDOWN: Duration = Duration::from_secs(60);

    pub fn arm(
        &mut self,
        session: Uuid,
        action: AfterDoneAction,
        run: &RunRecord,
        now: Instant,
    ) -> Result<(), &'static str> {
        if self.busy() {
            return Err("结束后动作正在倒计时或执行，请先取消并等待完成");
        }
        *self = Self::default();
        if action == AfterDoneAction::None {
            return Ok(());
        }
        if run.state != RunState::Running || !run.matches_account_context() || run.group.is_none() {
            return Err("只能为本次启动的应用组登记结束后动作");
        }
        self.session = Some(session);
        self.context = run.context.clone();
        self.observed_at = Some(now);
        self.status = Some(AfterDoneStatus {
            id: Uuid::new_v4(),
            run_id: run.id,
            action,
            state: AfterDoneState::Watching,
            remaining_seconds: 0,
            message: "等待应用组正常完成".into(),
        });
        Ok(())
    }

    pub fn busy(&self) -> bool {
        self.status.as_ref().is_some_and(|s| {
            matches!(
                s.state,
                AfterDoneState::CountingDown | AfterDoneState::Executing
            )
        })
    }

    pub fn watching(&self) -> bool {
        self.status.as_ref().is_some_and(|s| {
            matches!(
                s.state,
                AfterDoneState::Watching | AfterDoneState::CountingDown
            )
        })
    }

    pub fn snapshot(&self, now: Instant) -> Option<AfterDoneStatus> {
        let mut status = self.status.clone()?;
        if status.state == AfterDoneState::CountingDown {
            status.remaining_seconds = self.deadline.map_or(0, |at| {
                at.saturating_duration_since(now).as_millis().div_ceil(1000) as u32
            });
        }
        Some(status)
    }

    pub fn cancel(&mut self, id: Uuid) -> Result<(), &'static str> {
        let status = self.status.as_ref().ok_or("没有待取消的结束后动作")?;
        if status.id != id {
            return Err("结束后动作已变化，请读取最新状态");
        }
        if status.state == AfterDoneState::Executing {
            return Err("动作已经提交，无法撤销，请检查 Windows 状态");
        }
        self.invalidate("已取消本次结束后动作");
        Ok(())
    }

    pub fn invalidate(&mut self, message: &str) {
        if self.watching() {
            let status = self.status.as_mut().expect("已登记动作");
            status.state = AfterDoneState::Cancelled;
            status.remaining_seconds = 0;
            status.message = message.into();
            self.deadline = None;
        }
    }

    pub fn observe(&mut self, session: Uuid, snapshot: &Snapshot, now: Instant) {
        if !self.watching() {
            return;
        }
        let id = self.status.as_ref().expect("已登记动作").run_id;
        let Some(run) = snapshot.run.as_ref().filter(|run| {
            run.id == id && run.context == self.context && self.session == Some(session)
        }) else {
            self.invalidate("会话或任务已变化，结束后动作已取消");
            return;
        };
        if self
            .observed_at
            .is_some_and(|at| now.saturating_duration_since(at) > Duration::from_secs(5))
        {
            self.invalidate("状态监测中断，结束后动作已取消");
            return;
        }
        self.observed_at = Some(now);
        if snapshot.record_error.is_some() {
            self.invalidate("运行记录保存失败，结束后动作已取消");
            return;
        }
        if !run.state.is_terminal() {
            if self.busy() {
                self.invalidate("已完成任务状态改变，结束后动作已取消");
            }
            return;
        }
        let complete = run.state == RunState::Succeeded
            && run.finished_at.is_some_and(|at| at >= run.started_at)
            && run.matches_account_context()
            && run.inputs_released()
            && run.group.as_ref().is_some_and(|g| {
                g.state == RunState::Succeeded
                    && g.current_index.is_none()
                    && !g.steps.is_empty()
                    && g.steps.iter().any(|s| s.state == GroupStepState::Succeeded)
                    && g.steps.iter().all(|s| {
                        matches!(s.state, GroupStepState::Succeeded | GroupStepState::Skipped)
                    })
            })
            && run
                .mail
                .as_ref()
                .is_none_or(|p| p.state == crate::mail::MailState::Succeeded)
            && run
                .coffee
                .as_ref()
                .is_none_or(|p| p.state == crate::coffee_run::CoffeeState::Succeeded);
        if !complete {
            self.invalidate("应用组未正常完成或输入未释放，结束后动作已取消");
            return;
        }
        let status = self.status.as_mut().expect("已登记动作");
        if status.state == AfterDoneState::Watching {
            status.state = AfterDoneState::CountingDown;
            status.remaining_seconds = Self::COUNTDOWN.as_secs() as u32;
            status.message = "应用组已完成，可取消本次结束后动作".into();
            self.deadline = Some(now + Self::COUNTDOWN);
        }
    }

    pub fn take_due(&mut self, now: Instant) -> Option<AfterDoneStatus> {
        if self.deadline.is_none_or(|at| now < at)
            || self.status.as_ref()?.state != AfterDoneState::CountingDown
        {
            return None;
        }
        let status = self.status.as_mut()?;
        status.state = AfterDoneState::Executing;
        status.remaining_seconds = 0;
        status.message = "正在提交结束后动作".into();
        self.deadline = None;
        Some(status.clone())
    }

    pub fn finish(&mut self, id: Uuid, result: Result<(), String>) {
        if let Some(status) = &mut self.status
            && status.id == id
            && status.state == AfterDoneState::Executing
        {
            match result {
                Ok(()) => {
                    status.state = AfterDoneState::Succeeded;
                    status.message = "结束后动作已提交".into();
                }
                Err(error) => {
                    status.state = AfterDoneState::Failed;
                    status.message = error;
                }
            }
        }
    }
}
