use super::{AccountConfig, ConfigError, ConfigStore, MAX_CONFIG_BYTES, invalid, reject_links};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};
use ts_rs::TS;
use uuid::Uuid;

const LIST_LIMIT: usize = 50;
const SCAN_LIMIT: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ConfigBackup {
    pub id: String,
    pub revision: u32,
    pub bytes: u32,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ConfigBackups {
    pub revision: u32,
    pub items: Vec<ConfigBackup>,
    pub omitted: u32,
    pub unavailable: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ConfigBackupPreview {
    pub id: String,
    pub preview_id: String,
    pub current: AccountConfig,
    pub backup: AccountConfig,
}

fn backup_revision(id: &str) -> Result<u32, ConfigError> {
    let (revision, uuid) = id
        .strip_prefix("accounts-r")
        .and_then(|value| value.strip_suffix(".yml"))
        .and_then(|value| value.split_once('-'))
        .ok_or_else(|| invalid("备份标识无效，请从备份列表选择"))?;
    let number: u32 = revision.parse().map_err(|_| invalid("备份修订无效"))?;
    let parsed = Uuid::parse_str(uuid).map_err(|_| invalid("备份标识无效，请从备份列表选择"))?;
    if revision != number.to_string() || parsed.to_string() != uuid {
        return Err(invalid("备份标识无效，请从备份列表选择"));
    }
    Ok(number)
}

fn read_backup(path: &Path) -> Result<Vec<u8>, ConfigError> {
    reject_links(path)?;
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES {
        return Err(invalid("备份不是普通文件或超过大小限制"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(invalid("备份超过大小限制"));
    }
    Ok(bytes)
}

fn fingerprint(id: &str, current: &AccountConfig, bytes: &[u8]) -> Result<String, ConfigError> {
    let current = serde_json::to_vec(current).map_err(|_| invalid("无法生成配置预览"))?;
    let mut hash = Sha256::new();
    hash.update(b"config-backup-restore-v1");
    for value in [id.as_bytes(), current.as_slice(), bytes] {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value);
    }
    Ok(hex::encode(hash.finalize()))
}

impl ConfigStore {
    pub fn backups(&self) -> Result<ConfigBackups, ConfigError> {
        let revision = self.read()?.revision;
        let path = self.directory.join("backups");
        reject_links(&path)?;
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ConfigBackups {
                    revision,
                    items: Vec::new(),
                    omitted: 0,
                    unavailable: 0,
                });
            }
            Err(error) => return Err(error.into()),
        };
        let mut items = Vec::new();
        let mut unavailable = 0;
        for (index, entry) in entries.enumerate() {
            if index >= SCAN_LIMIT {
                return Err(invalid("备份目录文件过多，请先整理后重试"));
            }
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            let value = (|| {
                let revision = backup_revision(&id)?;
                reject_links(&entry.path())?;
                let metadata = fs::symlink_metadata(entry.path())?;
                if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES {
                    return Err(invalid("备份不可用"));
                }
                Ok(ConfigBackup {
                    id,
                    revision,
                    bytes: metadata.len() as u32,
                    modified_at: metadata
                        .modified()
                        .ok()
                        .map(|value| chrono::DateTime::<chrono::Utc>::from(value).to_rfc3339()),
                })
            })();
            match value {
                Ok(item) => items.push(item),
                Err(_) => unavailable += 1,
            }
        }
        items.sort_by(|a, b| {
            b.revision
                .cmp(&a.revision)
                .then(b.modified_at.cmp(&a.modified_at))
                .then(a.id.cmp(&b.id))
        });
        let omitted = items.len().saturating_sub(LIST_LIMIT) as u32;
        items.truncate(LIST_LIMIT);
        Ok(ConfigBackups {
            revision,
            items,
            omitted,
            unavailable,
        })
    }

    pub fn preview_backup(
        &self,
        revision: u32,
        id: &str,
    ) -> Result<ConfigBackupPreview, ConfigError> {
        let expected_revision = backup_revision(id)?;
        let current = self.read()?;
        if current.revision != revision {
            return Err(ConfigError::Conflict(Box::new(current)));
        }
        let bytes = read_backup(&self.directory.join("backups").join(id))?;
        let backup: AccountConfig =
            serde_yaml::from_slice(&bytes).map_err(|_| invalid("备份损坏或包含不支持的字段"))?;
        backup.validate()?;
        if backup.revision != expected_revision {
            return Err(invalid("备份内容与文件修订不一致"));
        }
        if current.import_id != backup.import_id {
            return Err(invalid("备份的导入来源与当前配置不同，不能恢复"));
        }
        Ok(ConfigBackupPreview {
            id: id.into(),
            preview_id: fingerprint(id, &current, &bytes)?,
            current,
            backup,
        })
    }

    pub fn restore_backup(
        &self,
        revision: u32,
        id: &str,
        preview_id: &str,
    ) -> Result<AccountConfig, ConfigError> {
        if preview_id.len() != 64
            || !preview_id
                .bytes()
                .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
        {
            return Err(invalid("请重新预览配置备份后恢复"));
        }
        let _lock = self.lock()?;
        let preview = self.preview_backup(revision, id)?;
        if preview.preview_id != preview_id {
            return Err(invalid("备份或当前配置已变化，请重新预览"));
        }
        let mut config = preview.backup;
        // 恢复内容可以移除或找回账号，但已使用的编号不能交给另一个新账号。
        config.reserved_instance_ids = config
            .reserved_instance_ids
            .iter()
            .chain(&preview.current.reserved_instance_ids)
            .copied()
            .chain(
                preview
                    .current
                    .instances
                    .iter()
                    .filter(|old| !config.instances.iter().any(|item| item.id == old.id))
                    .map(|item| item.id),
            )
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        config.revision = preview.current.revision;
        self.save(&mut config)?;
        Ok(config)
    }
}
