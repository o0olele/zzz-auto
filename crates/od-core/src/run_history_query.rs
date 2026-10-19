use super::*;
use sha2::{Digest, Sha256};

pub const HISTORY_PAGE_SIZE: usize = 20;
const PAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum HistoryOutcome {
    Active,
    Succeeded,
    Failed,
    TimedOut,
    Stopped,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HistoryCursor {
    #[ts(type = "string")]
    pub started_at: DateTime<Utc>,
    pub id: Uuid,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryQuery {
    pub group_id: Option<String>,
    pub app_id: Option<String>,
    pub game_day: Option<String>,
    pub outcome: Option<HistoryOutcome>,
    pub before: Option<HistoryCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct HistoryPage {
    pub instance_id: u32,
    pub query: HistoryQuery,
    pub items: Vec<HistoryItem>,
    pub matching: u32,
    pub unreadable: u32,
    pub next: Option<HistoryCursor>,
    pub content_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct HistoryExport {
    pub path: String,
    pub instance_id: u32,
    pub record_count: u32,
    pub content_id: String,
}

impl HistoryQuery {
    pub fn validate(&self) -> Result<(), HistoryError> {
        for value in [&self.group_id, &self.app_id].into_iter().flatten() {
            if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
                return Err(HistoryError::Invalid(
                    "历史筛选标识必须为 1..256 字节且不含控制字符",
                ));
            }
        }
        if self.game_day.as_ref().is_some_and(|date| {
            date.len() != 10
                || chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .ok()
                    .is_none_or(|parsed| parsed.format("%Y-%m-%d").to_string() != *date)
        }) {
            return Err(HistoryError::Invalid("游戏日必须为有效的 YYYY-MM-DD 日期"));
        }
        Ok(())
    }

    fn matches(&self, item: &HistoryItem) -> bool {
        let Some(context) = &item.record.context else {
            return false;
        };
        self.group_id
            .as_ref()
            .is_none_or(|id| id == &context.group_id)
            && self
                .app_id
                .as_ref()
                .is_none_or(|id| context.app_ids.contains(id))
            && self.game_day.as_ref().is_none_or(|date| {
                context
                    .game_day
                    .as_ref()
                    .is_some_and(|day| &day.date == date)
            })
            && self.outcome.is_none_or(|outcome| outcome == item.outcome())
    }
}

impl HistoryItem {
    pub fn outcome(&self) -> HistoryOutcome {
        if self.interrupted {
            return HistoryOutcome::Interrupted;
        }
        if self
            .record
            .daily
            .iter()
            .any(|p| p.state == MailState::TimedOut)
            || self
                .record
                .mail
                .as_ref()
                .is_some_and(|p| p.state == MailState::TimedOut)
            || self
                .record
                .coffee
                .as_ref()
                .is_some_and(|p| p.state == crate::coffee_run::CoffeeState::TimedOut)
        {
            return HistoryOutcome::TimedOut;
        }
        match self.record.state {
            RunState::Succeeded => HistoryOutcome::Succeeded,
            RunState::Failed => HistoryOutcome::Failed,
            RunState::Stopped => HistoryOutcome::Stopped,
            _ => HistoryOutcome::Active,
        }
    }

    fn cursor(&self) -> HistoryCursor {
        HistoryCursor {
            started_at: self.record.started_at,
            id: self.record.id,
        }
    }
}

impl RunStore {
    pub fn query(
        &self,
        instance_id: u32,
        live_run: Option<Uuid>,
        query: HistoryQuery,
    ) -> Result<HistoryPage, HistoryError> {
        query.validate()?;
        let mut matching = 0;
        let mut remaining = 0;
        let mut items = Vec::new();
        let (_, unreadable) = self.scan_records(instance_id, |record| {
            let item = HistoryItem {
                interrupted: !record.state.is_terminal() && live_run != Some(record.id),
                record,
            };
            if !query.matches(&item) {
                return;
            }
            matching += 1;
            if query.before.as_ref().is_some_and(|before| {
                (item.record.started_at, item.record.id) >= (before.started_at, before.id)
            }) {
                return;
            }
            remaining += 1;
            items.push(item);
            items.sort_by(|a, b| {
                (b.record.started_at, b.record.id).cmp(&(a.record.started_at, a.record.id))
            });
            items.truncate(HISTORY_PAGE_SIZE);
        })?;
        let mut bytes = 0;
        let keep = items
            .iter()
            .take_while(|item| {
                bytes += serde_json::to_vec(item).map_or(PAGE_BYTES + 1, |value| value.len());
                bytes <= PAGE_BYTES
            })
            .count();
        items.truncate(keep);
        let next =
            (remaining > items.len()).then(|| items.last().expect("单条记录小于页面上限").cursor());
        let content =
            serde_json::to_vec(&(instance_id, &query, &items, matching, unreadable, &next))
                .map_err(|_| HistoryError::Invalid("历史页面无法序列化"))?;
        Ok(HistoryPage {
            instance_id,
            query,
            items,
            matching,
            unreadable,
            next,
            content_id: hex::encode(Sha256::digest(content)),
        })
    }

    pub fn export_page(
        &self,
        instance_id: u32,
        live_run: Option<Uuid>,
        query: HistoryQuery,
        expected: &str,
    ) -> Result<HistoryExport, HistoryError> {
        let page = self.query(instance_id, live_run, query)?;
        if expected != page.content_id {
            return Err(HistoryError::Invalid(
                "运行记录已变化，请刷新本页后重新导出",
            ));
        }
        if page.items.is_empty() {
            return Err(HistoryError::Invalid("当前页没有可导出的记录"));
        }
        let bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1, "scope": "account_history_page", "exported_at": Utc::now(), "page": page,
        })).map_err(|_| HistoryError::Invalid("历史导出无法序列化"))?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(HistoryError::Invalid("历史导出超过 16 MiB"));
        }
        let directory = self.directory.join("exports");
        plain(&directory)?;
        fs::create_dir_all(&directory)?;
        plain(&directory)?;
        let path = directory.join(format!("account-{instance_id:04}-{}.json", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        let result = file.write_all(&bytes).and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = result {
            let _ = fs::remove_file(&path);
            return Err(error.into());
        }
        Ok(HistoryExport {
            path: path.to_string_lossy().into_owned(),
            instance_id,
            record_count: page.items.len() as u32,
            content_id: page.content_id,
        })
    }
}
