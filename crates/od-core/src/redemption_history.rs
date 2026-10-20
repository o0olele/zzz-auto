use super::{HistoryError, RunStore};
use crate::redemption::{RedemptionHistory, RedemptionResult};
use std::collections::BTreeMap;

impl RunStore {
    /// 逐码结果跨游戏日保存；后续失败、停止和输入状态不撤销已识别的结果。
    pub fn redemption_history(&self, instance_id: u32) -> Result<RedemptionHistory, HistoryError> {
        let mut results: BTreeMap<
            String,
            (chrono::DateTime<chrono::Utc>, uuid::Uuid, RedemptionResult),
        > = BTreeMap::new();
        let mut invalid = 0_u32;
        let (_, unreadable) = self.scan_records(instance_id, |record| {
            if !record.matches_account_context()
                || !record
                    .context
                    .as_ref()
                    .is_some_and(|c| c.app_ids.iter().any(|id| id == "redemption_code"))
            {
                return;
            }
            let Some(progress) = record.daily.iter().find(|p| p.app_id == "redemption_code") else {
                return;
            };
            let Some(redemption) = progress.redemption.as_ref() else {
                return;
            };
            if redemption.validate().is_err() {
                invalid = invalid.saturating_add(1);
                return;
            }
            for result in &redemption.results {
                let candidate = (record.started_at, record.id, result.clone());
                let entry = results
                    .entry(result.code.clone())
                    .or_insert_with(|| candidate.clone());
                if (candidate.0, candidate.1) > (entry.0, entry.1) {
                    *entry = candidate;
                }
            }
        })?;
        if results.len() > 4096 {
            return Err(HistoryError::Invalid(
                "账号兑换记录超过 4096 个码，请归档旧记录",
            ));
        }
        Ok(RedemptionHistory {
            results: results.into_values().map(|(_, _, r)| r).collect(),
            unreadable: unreadable.saturating_add(invalid),
        })
    }
}
