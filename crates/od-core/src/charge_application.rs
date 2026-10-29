//! 应用历史仅保存紧凑摘要；完整成功仍须复验本次体力日志。
use crate::{
    RunRecord, RunSlot, RunState,
    charge_plan::ChargeResources,
    charge_run::{ChargeRunContext, ChargeSchedule, ChargeSelection, MAX_ATTEMPTS},
    run_plan::RunContext,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChargeAppOutcome {
    Completed,
    NoPlans,
    InsufficientCharge,
    UnavailableMaterials,
    WeeklyRewardsAvailable,
    AgentTargetsUnavailable,
    NeedsVerification,
    AwaitingSave,
    Stopped,
    TimedOut,
    Failed,
}
impl ChargeAppOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Completed => "计划完成",
            Self::NoPlans => "无启用计划",
            Self::InsufficientCharge => "电量不足",
            Self::UnavailableMaterials => "合成素材不足",
            Self::WeeklyRewardsAvailable => "周常奖励未耗尽",
            Self::AgentTargetsUnavailable => "未找到培养目标",
            Self::NeedsVerification => "关卡或读数待验证",
            Self::AwaitingSave => "结果待提交",
            Self::Stopped => "已停止",
            Self::TimedOut => "运行超时",
            Self::Failed => "运行失败",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChargeAppSummary {
    pub records: u32,
    pub successful_runs: u32,
    pub journal_sha256: String,
}
impl ChargeAppSummary {
    pub fn from_schedule(schedule: &ChargeSchedule) -> Result<Self, &'static str> {
        schedule.validate()?;
        Ok(Self {
            records: schedule.records().len() as u32,
            successful_runs: schedule.progress()?.successful_runs,
            journal_sha256: hex::encode(Sha256::digest(
                serde_json::to_vec(schedule).map_err(|_| "体力摘要无法序列化")?,
            )),
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChargeAppProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub config_revision_after: Option<u32>,
    pub context: ChargeRunContext,
    pub state: RunState,
    pub stage: String,
    pub outcome: Option<ChargeAppOutcome>,
    pub summary: Option<ChargeAppSummary>,
    pub resources: Option<ChargeResources>,
    #[ts(type = "number")]
    pub active_ms: u64,
    pub inputs_released: bool,
    pub world_returned: bool,
    pub counts_committed: bool,
}
impl ChargeAppProgress {
    pub fn new(context: ChargeRunContext) -> Result<Self, &'static str> {
        context.validate()?;
        Ok(Self {
            config_revision_after: None,
            context,
            state: RunState::Running,
            stage: "World".into(),
            outcome: None,
            summary: None,
            resources: None,
            active_ms: 0,
            inputs_released: true,
            world_returned: false,
            counts_committed: false,
        })
    }
    pub fn completed(&self) -> bool {
        self.state == RunState::Succeeded
            && self.outcome == Some(ChargeAppOutcome::Completed)
            && self.summary.is_some()
            && self.world_returned
            && self.inputs_released
            && self.counts_committed
            && self.validate().is_ok()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.context.validate()?;
        if self.stage.is_empty()
            || self
                .config_revision_after
                .is_some_and(|r| r < self.context.config_revision || !self.counts_committed)
            || self.stage.len() > 64
            || self.stage.chars().any(char::is_control)
            || self.summary.as_ref().is_some_and(|s| {
                s.records as usize > MAX_ATTEMPTS
                    || s.successful_runs > s.records
                    || s.journal_sha256.len() != 64
                    || !s
                        .journal_sha256
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            })
            || self.resources.is_some_and(|r| r.validate().is_err())
        {
            return Err("体力应用摘要、阶段或资源无效");
        }
        let valid = match self.outcome {
            Some(ChargeAppOutcome::Completed) => {
                self.state == RunState::Succeeded
                    && self.summary.is_some()
                    && self.inputs_released
                    && self.world_returned
                    && self.counts_committed
            }
            Some(ChargeAppOutcome::NoPlans) => {
                self.state == RunState::Succeeded
                    && self.summary.is_none()
                    && self.inputs_released
                    && self.world_returned
            }
            Some(ChargeAppOutcome::AwaitingSave) => {
                matches!(self.state, RunState::Paused | RunState::Stopping)
                    && self.inputs_released
                    && !self.counts_committed
            }
            Some(ChargeAppOutcome::Stopped) => self.state == RunState::Stopped,
            Some(_) => self.state == RunState::Failed,
            None => {
                !self.state.is_terminal()
                    && (self.state != RunState::Paused || self.inputs_released)
            }
        };
        if !valid {
            return Err("体力应用状态与完成证明不符");
        }
        Ok(())
    }
    pub fn matches_record(&self, record: &RunRecord) -> bool {
        self.validate().is_ok()
            && matches!(record.app_id.as_str(), "charge_plan" | "application_group")
            && record.context.as_ref().is_some_and(|c| {
                self.context.run_id == record.id
                    && self.context.instance_id == c.instance_id
                    && self.context.group_id == c.group_id
                    && self.context.config_revision
                        == start_revision(record).unwrap_or(c.config_revision)
                    && c.game_day.as_ref() == Some(&self.context.game_day)
                    && c.app_ids.iter().any(|id| id == "charge_plan")
                    && (record.group.is_some()
                        || (record.app_id == "charge_plan" && record.state == self.state))
            })
    }
    pub fn verify_schedule(&self, schedule: &ChargeSchedule) -> Result<(), &'static str> {
        self.validate()?;
        if schedule.request().context != self.context
            || self.summary.as_ref() != Some(&ChargeAppSummary::from_schedule(schedule)?)
            || (self.completed() && schedule.final_selection() != Some(&ChargeSelection::Completed))
        {
            return Err("体力应用摘要与原始日志或完整结束选择不符");
        }
        Ok(())
    }
}
/// 只有本组更早的完整编队扫描可以形成新的体力起始修订。
/// 原应用组身份仍保留启动修订，任意配置修改不能借此重新绑定。
pub(crate) fn start_revision(record: &RunRecord) -> Option<u32> {
    let group = record.group.as_ref()?;
    let charge = group.steps.iter().position(|s| s.app_id == "charge_plan")?;
    let team = group.steps.iter().position(|s| {
        s.app_id == "predefined_team_checker"
            && s.state == crate::group_run::GroupStepState::Succeeded
    })?;
    if team >= charge {
        return None;
    }
    let progress = record.daily.iter().find(|p| {
        p.app_id == "predefined_team_checker"
            && p.state == crate::mail::MailState::Succeeded
            && p.completed()
    })?;
    progress.team_check.as_ref()?.saved_revision.filter(|r| {
        *r > record
            .context
            .as_ref()
            .map_or(u32::MAX, |c| c.config_revision)
    })
}
impl RunSlot {
    pub fn begin_charge(
        &mut self,
        context: RunContext,
        progress: ChargeAppProgress,
    ) -> Result<RunRecord, &'static str> {
        context.validate()?;
        progress.validate()?;
        if context.app_ids != ["charge_plan"]
            || progress.state != RunState::Running
            || progress.summary.is_some()
            || progress.outcome.is_some()
            || progress.context.instance_id != context.instance_id
            || progress.context.group_id != context.group_id
            || progress.context.config_revision != context.config_revision
            || context.game_day.as_ref() != Some(&progress.context.game_day)
        {
            return Err("体力独立记录必须从本次未执行状态开始");
        }
        let mut record = self.begin("charge_plan")?;
        record.id = progress.context.run_id;
        record.context = Some(context);
        record.charge = Some(Box::new(progress));
        debug_assert!(record.matches_account_context());
        self.current = Some(record.clone());
        Ok(record)
    }
    pub fn update_charge(
        &mut self,
        id: uuid::Uuid,
        mut progress: ChargeAppProgress,
        message: String,
    ) -> Result<RunRecord, &'static str> {
        progress.validate()?;
        let record = self.current.as_mut().ok_or("当前没有体力任务")?;
        if record.id != id
            || record.state.is_terminal()
            || record
                .charge
                .as_ref()
                .is_none_or(|old| old.context != progress.context)
        {
            return Err("体力运行 ID、应用或冻结身份不符");
        }
        if self
            .group_schedule
            .as_ref()
            .is_some_and(|s| s.current_app() != Some("charge_plan"))
        {
            return Err("体力不是当前分项");
        }
        if record.state == RunState::Stopping && progress.state == RunState::Succeeded {
            progress.state = RunState::Stopped;
            progress.outcome = Some(ChargeAppOutcome::Stopped);
        }
        if progress.state == RunState::Running
            && matches!(record.state, RunState::Pausing | RunState::Stopping)
        {
            progress.state = record.state;
        }
        if let Some(schedule) = &mut self.group_schedule {
            if progress.state == RunState::Paused && record.state != RunState::Stopping {
                if schedule.progress().state == RunState::Running {
                    schedule.request_pause()?;
                }
                if schedule.progress().state == RunState::Pausing {
                    schedule.acknowledge_pause(progress.inputs_released)?;
                }
            } else if progress.state.is_terminal() {
                use crate::group_run::AppOutcome;
                let outcome = if progress.completed()
                    || progress.outcome == Some(ChargeAppOutcome::NoPlans)
                {
                    AppOutcome::Succeeded
                } else if progress.state == RunState::Stopped {
                    AppOutcome::Stopped
                } else if progress.world_returned
                    && progress.inputs_released
                    && matches!(
                        progress.outcome,
                        Some(
                            ChargeAppOutcome::InsufficientCharge
                                | ChargeAppOutcome::UnavailableMaterials
                                | ChargeAppOutcome::WeeklyRewardsAvailable
                                | ChargeAppOutcome::AgentTargetsUnavailable
                        )
                    )
                {
                    AppOutcome::Ended
                } else {
                    AppOutcome::Failed
                };
                schedule.complete_current(
                    "charge_plan",
                    outcome,
                    progress.inputs_released,
                    chrono::Utc::now(),
                )?;
            }
            record.group = Some(schedule.progress().clone());
            record.state = schedule.progress().state;
        } else {
            record.state = progress.state;
        }
        record.charge = Some(Box::new(progress));
        record.message = message;
        record.finished_at = record.state.is_terminal().then(chrono::Utc::now);
        Ok(record.clone())
    }
}
