use crate::{
    RunState,
    run_plan::{GroupPlan, PlanDecision},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub fn recovery_inputs() -> Vec<crate::InputProbeConfig> {
    let mut inputs = crate::mail::MailConfig::recovery_inputs().to_vec();
    for input in crate::coffee_approach::recovery_inputs() {
        if !inputs.contains(&input) {
            inputs.push(input);
        }
    }
    inputs
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GroupStepState {
    Ended,
    Pending,
    Running,
    Skipped,
    Succeeded,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GroupStepProgress {
    pub app_id: String,
    pub state: GroupStepState,
    pub inputs_released: bool,
    #[ts(type = "string | null")]
    pub started_at: Option<DateTime<Utc>>,
    #[ts(type = "string | null")]
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GroupProgress {
    pub checkpoint: u32,
    pub state: RunState,
    pub current_index: Option<usize>,
    pub steps: Vec<GroupStepProgress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppOutcome {
    Ended,
    Succeeded,
    Failed,
    Stopped,
}

// 与一次投递关联，检查点变化后旧的投递凭据不可复用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupStepPermit {
    index: usize,
    checkpoint: u32,
}

#[derive(Debug)]
pub struct GroupSchedule {
    progress: GroupProgress,
    checkpoint: u32,
    saved_checkpoint: u32,
}

impl GroupSchedule {
    pub fn new(plan: &GroupPlan) -> Result<Self, &'static str> {
        plan.context.validate()?;
        if !plan.blockers.is_empty() || (!plan.executable && !plan.all_done) {
            return Err("应用组计划未通过预检");
        }
        let enabled: Vec<_> = plan.items.iter().filter(|item| item.enabled).collect();
        if enabled
            .iter()
            .map(|item| &item.app_id)
            .ne(plan.context.app_ids.iter())
            || enabled.iter().any(|item| {
                !matches!(
                    item.decision,
                    PlanDecision::Run | PlanDecision::AlreadyDone | PlanDecision::NoWork
                )
            })
            || plan
                .items
                .iter()
                .any(|item| !item.enabled && item.decision != PlanDecision::Disabled)
        {
            return Err("应用组分项与保存计划不符");
        }
        let steps: Vec<_> = enabled
            .into_iter()
            .map(|item| GroupStepProgress {
                app_id: item.app_id.clone(),
                state: if matches!(
                    item.decision,
                    PlanDecision::AlreadyDone | PlanDecision::NoWork
                ) {
                    GroupStepState::Skipped
                } else {
                    GroupStepState::Pending
                },
                inputs_released: true,
                started_at: None,
                finished_at: None,
            })
            .collect();
        let all_done = steps
            .iter()
            .all(|item| item.state == GroupStepState::Skipped);
        if all_done != plan.all_done || plan.executable == all_done {
            return Err("应用组完成状态与分项不符");
        }
        Ok(Self {
            progress: GroupProgress {
                checkpoint: 0,
                state: if all_done {
                    RunState::Succeeded
                } else {
                    RunState::Running
                },
                current_index: None,
                steps,
            },
            checkpoint: 0,
            saved_checkpoint: 0,
        })
    }

    pub fn progress(&self) -> &GroupProgress {
        &self.progress
    }
    pub fn checkpoint(&self) -> u32 {
        self.checkpoint
    }
    pub fn checkpoint_pending(&self) -> bool {
        self.saved_checkpoint != self.checkpoint
    }

    fn changed(&mut self) {
        self.checkpoint += 1;
        self.progress.checkpoint = self.checkpoint;
    }

    pub fn confirm_saved(&mut self, checkpoint: u32) -> Result<(), &'static str> {
        if checkpoint != self.checkpoint {
            return Err("保存回执不属于当前检查点");
        }
        self.saved_checkpoint = checkpoint;
        Ok(())
    }

    pub fn start_next(
        &mut self,
        now: DateTime<Utc>,
    ) -> Result<Option<GroupStepPermit>, &'static str> {
        if self.checkpoint_pending() {
            return Err("上一检查点尚未完整保存");
        }
        if self.progress.state.is_terminal() {
            return Ok(None);
        }
        if self.progress.state != RunState::Running || self.progress.current_index.is_some() {
            return Err("当前应用或控制请求尚未收尾");
        }
        let Some(index) = self
            .progress
            .steps
            .iter()
            .position(|item| item.state == GroupStepState::Pending)
        else {
            return Err("没有可开始的应用");
        };
        let step = &mut self.progress.steps[index];
        step.state = GroupStepState::Running;
        step.started_at = Some(now);
        step.inputs_released = false;
        self.progress.current_index = Some(index);
        self.changed();
        Ok(Some(GroupStepPermit {
            index,
            checkpoint: self.checkpoint,
        }))
    }

    pub fn permitted_app(&self, permit: &GroupStepPermit) -> Option<&str> {
        (self.progress.state == RunState::Running
            && !self.checkpoint_pending()
            && permit.checkpoint == self.checkpoint
            && self.progress.current_index == Some(permit.index))
        .then(|| self.progress.steps[permit.index].app_id.as_str())
    }

    pub fn current_app(&self) -> Option<&str> {
        self.progress
            .current_index
            .map(|index| self.progress.steps[index].app_id.as_str())
    }

    pub fn request_pause(&mut self) -> Result<(), &'static str> {
        if !matches!(self.progress.state, RunState::Running | RunState::Pausing) {
            return Err("当前应用组不能暂停");
        }
        self.progress.state = if self.progress.current_index.is_some() {
            RunState::Pausing
        } else {
            RunState::Paused
        };
        Ok(())
    }

    pub fn acknowledge_pause(&mut self, inputs_released: bool) -> Result<(), &'static str> {
        if self.progress.state != RunState::Pausing || !inputs_released {
            return Err("尚未确认暂停与输入释放");
        }
        if let Some(index) = self.progress.current_index {
            self.progress.steps[index].inputs_released = true;
        }
        self.progress.state = RunState::Paused;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), &'static str> {
        if self.progress.state != RunState::Paused {
            return Err("应用组尚未确认暂停");
        }
        self.progress.state = RunState::Running;
        if let Some(index) = self.progress.current_index {
            self.progress.steps[index].inputs_released = false;
        }
        Ok(())
    }

    pub fn request_stop(&mut self) {
        if !self.progress.state.is_terminal() {
            self.progress.state = if self.progress.current_index.is_some() {
                RunState::Stopping
            } else {
                RunState::Stopped
            };
        }
    }

    pub fn complete_current(
        &mut self,
        app_id: &str,
        outcome: AppOutcome,
        released: bool,
        now: DateTime<Utc>,
    ) -> Result<(), &'static str> {
        let index = self.progress.current_index.ok_or("当前没有执行中的应用")?;
        let step = &mut self.progress.steps[index];
        if step.app_id != app_id || now < step.started_at.ok_or("应用缺少开始时间")? {
            return Err("应用结果或结束时间不匹配");
        }
        let outcome = if !released {
            AppOutcome::Failed
        } else if self.progress.state == RunState::Stopping {
            AppOutcome::Stopped
        } else {
            outcome
        };
        step.state = match outcome {
            AppOutcome::Ended => GroupStepState::Ended,
            AppOutcome::Succeeded => GroupStepState::Succeeded,
            AppOutcome::Failed => GroupStepState::Failed,
            AppOutcome::Stopped => GroupStepState::Stopped,
        };
        step.inputs_released = released;
        step.finished_at = Some(now);
        self.progress.current_index = None;
        self.progress.state = match outcome {
            AppOutcome::Failed => RunState::Failed,
            AppOutcome::Stopped => RunState::Stopped,
            AppOutcome::Succeeded | AppOutcome::Ended
                if self
                    .progress
                    .steps
                    .iter()
                    .any(|item| item.state == GroupStepState::Pending) =>
            {
                if matches!(self.progress.state, RunState::Pausing | RunState::Paused) {
                    RunState::Paused
                } else {
                    RunState::Running
                }
            }
            AppOutcome::Succeeded | AppOutcome::Ended => {
                if self
                    .progress
                    .steps
                    .iter()
                    .any(|s| s.state == GroupStepState::Ended)
                {
                    RunState::Failed
                } else {
                    RunState::Succeeded
                }
            }
        };
        self.changed();
        Ok(())
    }
}
