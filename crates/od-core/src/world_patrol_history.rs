use super::{HistoryError, RunStore};
use crate::{
    daily::GameDay,
    world_patrol::{PatrolProgress, WorldPatrolConfig},
};
use chrono::{DateTime, Utc};
impl RunStore {
    pub fn patrol_checkpoint(
        &self,
        instance_id: u32,
        group_id: &str,
        day: &GameDay,
        config: &WorldPatrolConfig,
        now: DateTime<Utc>,
    ) -> Result<Option<(PatrolProgress, u64)>, HistoryError> {
        let mut latest = None;
        let (_, unreadable) = self.scan_records(instance_id, |record| {
            let Some(context) = &record.context else {
                return;
            };
            if context.group_id != group_id
                || context.game_day.as_ref() != Some(day)
                || !context.app_ids.iter().any(|id| id == "world_patrol")
                || !day.contains(record.started_at)
                || record.started_at > now
                || !record.matches_account_context()
                || !matches!(record.app_id.as_str(), "world_patrol" | "application_group")
                || record.state.is_terminal() != record.finished_at.is_some()
                || record
                    .finished_at
                    .is_some_and(|at| at < record.started_at || at > now)
            {
                return;
            }
            let mut items = record.daily.iter().filter(|p| p.app_id == "world_patrol");
            let Some(item) = items.next() else {
                return;
            };
            let Some(p) = &item.world_patrol else {
                return;
            };
            if items.next().is_some()
                || p.config != *config
                || p.validate().is_err()
                || p.round_started_ms > item.active_ms
            {
                return;
            }
            let key = (record.started_at, record.id);
            if latest
                .as_ref()
                .is_none_or(|(previous, _, _)| key > *previous)
            {
                latest = Some((key, p.clone(), item.active_ms));
            }
        })?;
        if unreadable > 0 {
            return Err(HistoryError::Invalid(
                "锄大地历史含无法读取的记录，请先检查运行记录",
            ));
        }
        Ok(latest.map(|(_, mut patrol, active)| {
            if let Some(first_failed) = patrol
                .routes
                .iter()
                .position(|r| patrol.failed.contains(&r.id))
            {
                patrol.route_index = first_failed;
                patrol
                    .finished
                    .retain(|id| patrol.routes.iter().take(first_failed).any(|r| &r.id == id));
                patrol.failed.clear();
            }
            patrol.point_index = 0;
            patrol.attempt = 0;
            (patrol, active)
        }))
    }
}
