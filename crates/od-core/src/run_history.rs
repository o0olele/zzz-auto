use crate::{RunRecord, config::reject_links};
use serde::{Deserialize, Serialize};
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
        let directory = self.account_directory(instance_id)?;
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RunHistory {
                    instance_id,
                    items: Vec::new(),
                    unreadable: 0,
                    has_more: false,
                });
            }
            Err(error) => return Err(error.into()),
        };
        let mut items = Vec::new();
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
                }
                Err(_) => unreadable += 1,
            }
        }
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
