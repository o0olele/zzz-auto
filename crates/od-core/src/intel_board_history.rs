use super::{HistoryError, RunStore};
use crate::{
    daily::GameDay,
    intel_board::{IntelHistory, IntelWeek},
};
use chrono::{DateTime, Utc};

impl RunStore {
    /// 保留部分完成、停止和异常运行中已经确认并返回板面的结果。相同运行的
    /// 文件只读取一次；每份 ledger 是累计快照，不能将历史基线当成新增次数。
    pub fn intel_history(
        &self,
        instance_id: u32,
        day: &GameDay,
        now: DateTime<Utc>,
    ) -> Result<IntelHistory, HistoryError> {
        let week = IntelWeek::from_day(day).map_err(HistoryError::Invalid)?;
        let mut snapshots = Vec::new();
        let mut invalid = 0_u32;
        let (_, unreadable) = self.scan_records(instance_id, |record| {
            let Some(context) = record.context.as_ref() else {
                return;
            };
            let Some(saved_day) = context.game_day.as_ref() else {
                return;
            };
            if IntelWeek::from_day(saved_day).ok().as_ref() != Some(&week) {
                return;
            }
            let mut checkpoints = record.daily.iter().filter(|p| p.app_id == "intel_board");
            let Some(progress) = checkpoints.next() else {
                return;
            };
            let Some(ledger) = progress.intel_board.as_ref() else {
                invalid = invalid.saturating_add(1);
                return;
            };
            if checkpoints.next().is_some()
                || !matches!(record.app_id.as_str(), "intel_board" | "application_group")
                || !context.app_ids.iter().any(|id| id == "intel_board")
                || ledger.week != week
                || ledger.validate().is_err()
                || !saved_day.contains(record.started_at)
                || record.started_at > now
                || record.state.is_terminal() != record.finished_at.is_some()
                || record
                    .finished_at
                    .is_some_and(|finished| finished < record.started_at || finished > now)
            {
                invalid = invalid.saturating_add(1);
                return;
            }
            snapshots.push((record.started_at, record.id, ledger.totals()));
        })?;
        snapshots.sort_by_key(|(started, id, _)| (*started, *id));
        let mut history = IntelHistory {
            week: Some(week),
            unreadable: unreadable.saturating_add(invalid),
            ..Default::default()
        };
        for (_, id, totals) in snapshots {
            // 完整快照应保持历史经验估计与计数单调。旧记录被归档不影响后续
            // 快照；冲突快照不参与目标完成判定，原文件仍留给用户检查。
            if totals.contribution < history.totals.contribution
                || totals.estimated_base_exp < history.totals.estimated_base_exp
                || totals.notorious_hunt_count < history.totals.notorious_hunt_count
                || totals.expert_challenge_count < history.totals.expert_challenge_count
                || (history.totals.experience() > 0
                    && totals.estimated_base_exp != history.totals.estimated_base_exp)
            {
                history.unreadable = history.unreadable.saturating_add(1);
                continue;
            }
            history.totals = totals;
            history.source_run_id = Some(id);
        }
        Ok(history)
    }
}
