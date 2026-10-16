use crate::{
    RunRecord, RunState,
    config::reject_links,
    daily::GameDay,
    mail::{MailPhase, MailState},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use ts_rs::TS;
use uuid::Uuid;

const RECORD_LIMIT: u64 = 1024 * 1024;
const DIRECTORY_LIMIT: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct HistoryItem {
    pub record: RunRecord,
    pub interrupted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RunHistory {
    pub instance_id: u32,
    pub items: Vec<HistoryItem>,
    pub unreadable: u32,
    pub has_more: bool,
}

pub struct DailyCompletions {
    pub game_day: GameDay,
    pub apps: BTreeMap<String, Uuid>,
    pub unreadable: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRun {
    version: u32,
    record: RunRecord,
}

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("运行记录读写失败：{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct RunStore {
    directory: PathBuf,
}

impl RunStore {
    pub fn new(data: &Path) -> Result<Self, HistoryError> {
        if !data.is_absolute() {
            return Err(HistoryError::Invalid("数据目录必须为绝对路径"));
        }
        let directory = data.join("runs");
        plain(&directory)?;
        Ok(Self { directory })
    }

    fn account_directory(&self, instance_id: u32) -> Result<PathBuf, HistoryError> {
        if !(1..=9999).contains(&instance_id) {
            return Err(HistoryError::Invalid("账号编号必须为 1..9999"));
        }
        let path = self
            .directory
            .join("accounts")
            .join(format!("{instance_id:04}"));
        plain(&path)?;
        // Windows 可能将“上级是文件”报告为子路径不存在，不能据此当作空历史。
        for directory in [&self.directory, &self.directory.join("accounts"), &path] {
            match fs::metadata(directory) {
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(HistoryError::Invalid(
                        "运行记录路径应为目录，不能被同名文件占用",
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(path)
    }

    pub fn save(&self, record: &RunRecord) -> Result<(), HistoryError> {
        let (directory, bytes) = if let Some(context) = &record.context {
            context.validate().map_err(HistoryError::Invalid)?;
            if !context.app_ids.contains(&record.app_id) {
                return Err(HistoryError::Invalid("运行结果不属于保存的应用组计划"));
            }
            (
                self.account_directory(context.instance_id)?,
                serde_json::to_vec_pretty(&StoredRun {
                    version: 1,
                    record: record.clone(),
                }),
            )
        } else {
            plain(&self.directory)?;
            (self.directory.clone(), serde_json::to_vec_pretty(record))
        };
        let bytes = bytes.map_err(|_| HistoryError::Invalid("运行记录无法序列化"))?;
        if bytes.len() as u64 > RECORD_LIMIT {
            return Err(HistoryError::Invalid("运行记录超过 1 MiB"));
        }
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{}.json", record.id));
        plain(&path)?;
        let temp = directory.join(format!("{}-{}.tmp", record.id, Uuid::new_v4()));
        let result = (|| -> Result<(), HistoryError> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }

    pub fn history(
        &self,
        instance_id: u32,
        live_run: Option<Uuid>,
    ) -> Result<RunHistory, HistoryError> {
        let mut items = Vec::new();
        let (valid, unreadable) = self.scan_records(instance_id, |record| {
            let interrupted = !record.state.is_terminal() && live_run != Some(record.id);
            items.push(HistoryItem {
                record,
                interrupted,
            });
            items.sort_by(|a, b| {
                b.record
                    .started_at
                    .cmp(&a.record.started_at)
                    .then(b.record.id.cmp(&a.record.id))
            });
            items.truncate(100);
        })?;
        // 查询结果留出协议封装余量，不让大量错误详情挤爆 IPC 响应。
        let mut size: usize = 0;
        items.retain(|item| {
            size = size.saturating_add(
                serde_json::to_vec(item).map_or(usize::MAX / 2, |bytes| bytes.len()),
            );
            size <= 8 * 1024 * 1024
        });
        let has_more = valid > items.len();
        Ok(RunHistory {
            instance_id,
            items,
            unreadable,
            has_more,
        })
    }

    pub fn daily_completions(
        &self,
        instance_id: u32,
        game_day: GameDay,
        now: DateTime<Utc>,
    ) -> Result<DailyCompletions, HistoryError> {
        game_day.validate().map_err(HistoryError::Invalid)?;
        let mut successes: BTreeMap<String, (DateTime<Utc>, Uuid)> = BTreeMap::new();
        let (_, unreadable) = self.scan_records(instance_id, |record| {
            let Some(finished) = record.finished_at else {
                return;
            };
            let Some(context) = &record.context else {
                return;
            };
            // 目前仅邮件有经过验证的应用完成与释放契约；登记表不能充当执行证明。
            if record.app_id != "email"
                || record.state != RunState::Succeeded
                || context.game_day.as_ref() != Some(&game_day)
                || finished < record.started_at
                || finished > now
                || !game_day.contains(record.started_at)
                || !game_day.contains(finished)
                || record.group.as_ref().is_some_and(|group| {
                    group.state != RunState::Succeeded
                        || group.current_index.is_some()
                        || group.steps.len() != 1
                        || context.app_ids != ["email"]
                        || group.steps.iter().any(|step| {
                            step.app_id != "email"
                                || step.state != crate::group_run::GroupStepState::Succeeded
                                || !step.inputs_released
                                || !step.started_at.zip(step.finished_at).is_some_and(
                                    |(start, end)| {
                                        start <= end
                                            && end <= finished
                                            && game_day.contains(start)
                                            && game_day.contains(end)
                                    },
                                )
                        })
                })
                || !record.mail.as_ref().is_some_and(|mail| {
                    mail.state == MailState::Succeeded
                        && mail.phase == MailPhase::Done
                        && mail.inputs_released
                })
            {
                return;
            }
            let entry = successes
                .entry(record.app_id)
                .or_insert((finished, record.id));
            if (finished, record.id) > *entry {
                *entry = (finished, record.id);
            }
        })?;
        Ok(DailyCompletions {
            game_day,
            apps: successes
                .into_iter()
                .map(|(app, (_, id))| (app, id))
                .collect(),
            unreadable,
        })
    }

    fn scan_records(
        &self,
        instance_id: u32,
        mut visit: impl FnMut(RunRecord),
    ) -> Result<(usize, u32), HistoryError> {
        let directory = self.account_directory(instance_id)?;
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((0, 0));
            }
            Err(error) => return Err(error.into()),
        };
        let mut unreadable = 0;
        let mut valid = 0;
        for (count, entry) in entries.enumerate() {
            if count >= DIRECTORY_LIMIT {
                return Err(HistoryError::Invalid(
                    "账号历史目录超过 10000 项，请先归档旧记录",
                ));
            }
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            match read_record(&path, instance_id) {
                Ok(record) => {
                    valid += 1;
                    visit(record);
                }
                Err(_) => unreadable += 1,
            }
        }
        Ok((valid, unreadable))
    }
}

fn plain(path: &Path) -> Result<(), HistoryError> {
    reject_links(path).map_err(|_| HistoryError::Invalid("运行记录路径不可用或包含目录联接"))
}

fn read_record(path: &Path, instance_id: u32) -> Result<RunRecord, HistoryError> {
    plain(path)?;
    let id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .and_then(|name| Uuid::parse_str(name).ok())
        .ok_or(HistoryError::Invalid("运行文件名无效"))?;
    let file = File::open(path)?;
    if file.metadata()?.len() > RECORD_LIMIT {
        return Err(HistoryError::Invalid("运行记录过大"));
    }
    let mut bytes = Vec::new();
    file.take(RECORD_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return Err(HistoryError::Invalid("运行记录过大"));
    }
    let stored: StoredRun =
        serde_json::from_slice(&bytes).map_err(|_| HistoryError::Invalid("运行记录损坏"))?;
    let context = stored
        .record
        .context
        .as_ref()
        .ok_or(HistoryError::Invalid("运行记录缺少账号信息"))?;
    context.validate().map_err(HistoryError::Invalid)?;
    if stored.version != 1
        || stored.record.id != id
        || context.instance_id != instance_id
        || !context.app_ids.contains(&stored.record.app_id)
    {
        return Err(HistoryError::Invalid("运行记录版本、账号或运行 ID 不匹配"));
    }
    Ok(stored.record)
}
