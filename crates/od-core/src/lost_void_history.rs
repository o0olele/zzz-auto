//! 迷失之地历史只核验与补交记录，不恢复游戏执行器。
use super::*;
use crate::{
    group_run::GroupStepState,
    lost_void_record_store::{LostVoidRecordError, LostVoidRecordStore},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidHistoryReplay {
    pub scanned: u32,
    pub unreadable: u32,
    pub records_with_pending: u32,
    pub settlements_changed: u32,
    pub claims_changed: u32,
}
fn record_error(error: LostVoidRecordError) -> HistoryError {
    match error {
        LostVoidRecordError::Io(error) => HistoryError::Io(error),
        LostVoidRecordError::Invalid(message) => HistoryError::Invalid(message),
        LostVoidRecordError::Locked => HistoryError::Invalid("迷失之地记录正在被其他任务更新"),
    }
}
fn store(runs: &Path) -> Result<LostVoidRecordStore, HistoryError> {
    LostVoidRecordStore::new(
        runs.parent()
            .ok_or(HistoryError::Invalid("迷失之地历史数据目录缺失"))?,
    )
    .map_err(record_error)
}
pub(super) fn verify(record: &RunRecord, runs: &Path) -> Result<(), HistoryError> {
    if let Some(progress) = &record.lost_void {
        if !progress.matches_record(record)
            || record.state.is_terminal() != record.finished_at.is_some()
            || record
                .finished_at
                .is_some_and(|finished| finished < record.started_at)
        {
            return Err(HistoryError::Invalid(
                "迷失之地历史与冻结运行身份或状态不符",
            ));
        }
        if let Some(group) = &record.group {
            let index = group
                .steps
                .iter()
                .position(|step| step.app_id == "lost_void")
                .ok_or(HistoryError::Invalid("迷失之地历史不属于组分项"))?;
            let step = &group.steps[index];
            let valid = match step.state {
                GroupStepState::Succeeded => progress.completed(),
                GroupStepState::Failed => progress.state == RunState::Failed,
                GroupStepState::Stopped => {
                    matches!(progress.state, RunState::Stopped | RunState::Failed)
                }
                GroupStepState::Running => {
                    group.current_index == Some(index)
                        && !record.state.is_terminal()
                        && record.state == progress.state
                }
                GroupStepState::Pending | GroupStepState::Skipped | GroupStepState::Ended => false,
            };
            if !valid
                || group.state != record.state
                || step.state != GroupStepState::Running && group.current_index == Some(index)
            {
                return Err(HistoryError::Invalid("迷失之地摘要与组分项状态不符"));
            }
        }
        progress
            .verify_committed(&store(runs)?)
            .map_err(record_error)?;
    } else if record.app_id == "lost_void"
        || record.group.as_ref().is_some_and(|group| {
            group.steps.iter().any(|step| {
                step.app_id == "lost_void"
                    && !matches!(
                        step.state,
                        GroupStepState::Pending | GroupStepState::Skipped
                    )
            })
        })
    {
        return Err(HistoryError::Invalid("迷失之地历史缺少专用运行摘要"));
    }
    Ok(())
}
pub(super) fn journal_pending(record: &RunRecord, runs: &Path) -> Result<(), HistoryError> {
    if let Some(progress) = &record.lost_void
        && (progress.pending_claim.is_some() || progress.pending_settlement.is_some())
    {
        progress
            .replay_pending(&store(runs)?)
            .map_err(record_error)?;
    }
    Ok(())
}
pub(super) fn verify_replacement(old: &RunRecord, next: &RunRecord) -> Result<(), HistoryError> {
    if old.lost_void.is_some() && next.lost_void.is_none() {
        return Err(HistoryError::Invalid("迷失之地历史不能移除已登记的摘要"));
    }
    if old.lost_void.is_some() || next.lost_void.is_some() {
        if old.state.is_terminal()
            && (old.state != next.state || old.finished_at != next.finished_at)
        {
            return Err(HistoryError::Invalid(
                "迷失之地历史不能改写已结束结果或完成时间",
            ));
        }
        if old.id != next.id
            || old.app_id != next.app_id
            || old.started_at != next.started_at
            || old.context != next.context
        {
            return Err(HistoryError::Invalid("迷失之地历史不能改绑运行身份"));
        }
        if let (Some(old), Some(next)) = (&old.lost_void, &next.lost_void) {
            next.validate_successor(old)
                .map_err(HistoryError::Invalid)?;
        }
    }
    Ok(())
}
impl RunStore {
    pub fn begin_lost_void_record(&self, record: &RunRecord) -> Result<(), HistoryError> {
        if record.app_id != "lost_void"
            || record.state != RunState::Running
            || record.lost_void.as_ref().is_none_or(|progress| {
                progress.end.is_some()
                    || progress.phase != crate::lost_void_application::LostVoidAppPhase::Plan
                    || progress.challenges_started != 0
                    || progress.completed_claim_checks != 0
                    || progress.active_ms != 0
                    || progress.planning.is_some()
                    || progress.pending_claim.is_some()
                    || progress.pending_settlement.is_some()
            })
        {
            return Err(HistoryError::Invalid("首次迷失之地历史必须是未执行状态"));
        }
        self.save_inner(record, true)
    }
    /// 显式恢复历史中的紧凑证据；不改写原历史、不提升成功状态、不创建输入资格。
    pub fn replay_lost_void_pending(
        &self,
        instance_id: u32,
    ) -> Result<LostVoidHistoryReplay, HistoryError> {
        let target = store(&self.directory)?;
        let mut report = LostVoidHistoryReplay {
            scanned: 0,
            unreadable: 0,
            records_with_pending: 0,
            settlements_changed: 0,
            claims_changed: 0,
        };
        let mut error = None;
        let (valid, unreadable) = self.scan_records(instance_id, |record| {
            if error.is_some() {
                return;
            }
            if let Some(progress) = record.lost_void
                && (progress.pending_settlement.is_some() || progress.pending_claim.is_some())
            {
                report.records_with_pending += 1;
                match progress.replay_pending(&target) {
                    Ok(replayed) => {
                        report.settlements_changed += u32::from(replayed.settlement_changed);
                        report.claims_changed += u32::from(replayed.claim_changed);
                    }
                    Err(problem) => error = Some(record_error(problem)),
                }
            }
        })?;
        if let Some(error) = error {
            return Err(error);
        }
        report.scanned = valid as u32;
        report.unreadable = unreadable;
        Ok(report)
    }
}
