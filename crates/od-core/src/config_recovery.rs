use super::backups::{backup_revision, read_backup};
use super::{
    AccountConfig, CONFIG_SCHEMA, ConfigBackup, ConfigError, ConfigStore, invalid, reject_links,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};
use uuid::Uuid;

const LIST_LIMIT: usize = 50;
const SCAN_BYTES_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigRecoveryState {
    Missing,
    Damaged,
    Healthy,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigRecoveryCurrent {
    pub state: ConfigRecoveryState,
    pub bytes: Option<u32>,
    pub sha256: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConfigRecoveryCatalog {
    pub data_directory: String,
    pub current: ConfigRecoveryCurrent,
    pub backups: Vec<ConfigBackup>,
    pub omitted: u32,
    pub unavailable: u32,
}

#[derive(Debug, Serialize)]
pub struct ConfigRecoveryPreview {
    pub data_directory: String,
    pub id: String,
    pub preview_id: String,
    pub current: ConfigRecoveryCurrent,
    pub recovered: AccountConfig,
    pub unreadable_backups: u32,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ConfigRecoveryResult {
    pub config: AccountConfig,
    // 相对数据目录的固定归档位置；不返回损坏文件的内容。
    pub preserved_file: Option<String>,
    pub recovered_backup: String,
}

/// 独占数据目录的离线工具；整个预览 / 恢复期间不允许引擎取得所有权。
pub struct ConfigRecovery {
    store: ConfigStore,
    _owner: File,
}

struct Plan {
    preview: ConfigRecoveryPreview,
    current_bytes: Option<Vec<u8>>,
}

fn decode(bytes: &[u8]) -> Result<AccountConfig, ConfigError> {
    let config: AccountConfig =
        serde_yaml::from_slice(bytes).map_err(|_| invalid("备份损坏或包含不支持的字段"))?;
    config.validate()?;
    Ok(config)
}

fn current_state(bytes: Option<&[u8]>) -> ConfigRecoveryCurrent {
    let state = match bytes {
        None => ConfigRecoveryState::Missing,
        Some(bytes) if decode(bytes).is_ok() => ConfigRecoveryState::Healthy,
        Some(bytes) => {
            let value = serde_yaml::from_slice::<serde_yaml::Value>(bytes).ok();
            if value
                .as_ref()
                .and_then(|value| value.get("schema"))
                .is_some_and(|schema| schema.as_u64() != Some(u64::from(CONFIG_SCHEMA)))
            {
                ConfigRecoveryState::Unsupported
            } else {
                ConfigRecoveryState::Damaged
            }
        }
    };
    ConfigRecoveryCurrent {
        state,
        bytes: bytes.map(|bytes| bytes.len() as u32),
        sha256: bytes.map(|bytes| hex::encode(Sha256::digest(bytes))),
    }
}

fn hash_part(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    let result = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = result {
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}

impl ConfigRecovery {
    pub fn new(data: &Path) -> Result<Self, ConfigError> {
        // 不在拼错的路径中创建一个新的数据目录。
        if !data.is_absolute() || !data.is_dir() {
            return Err(invalid("离线恢复须显式指定已存在的绝对数据目录"));
        }
        reject_links(data)?;
        let store = ConfigStore::new(&data.canonicalize()?)?;
        let owner = store.acquire_owner()?;
        Ok(Self {
            store,
            _owner: owner,
        })
    }

    fn current_bytes(&self) -> Result<Option<Vec<u8>>, ConfigError> {
        let path = self.store.directory.join("accounts.yml");
        reject_links(&path)?;
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
            Ok(_) => read_backup(&path).map(Some),
        }
    }

    pub fn catalog(&self) -> Result<ConfigRecoveryCatalog, ConfigError> {
        let current = current_state(self.current_bytes()?.as_deref());
        let (mut backups, unavailable) = self.store.scan_backups()?;
        let omitted = backups.len().saturating_sub(LIST_LIMIT) as u32;
        backups.truncate(LIST_LIMIT);
        Ok(ConfigRecoveryCatalog {
            data_directory: self.store.data_directory().display().to_string(),
            current,
            backups,
            omitted,
            unavailable,
        })
    }

    pub fn preview(&self, id: &str) -> Result<ConfigRecoveryPreview, ConfigError> {
        let _lock = self.store.lock()?;
        Ok(self.plan(id)?.preview)
    }

    fn plan(&self, id: &str) -> Result<Plan, ConfigError> {
        let revision = backup_revision(id)?;
        let current_bytes = self.current_bytes()?;
        let current = current_state(current_bytes.as_deref());
        match current.state {
            ConfigRecoveryState::Healthy => {
                return Err(invalid("当前配置可正常读取，请使用桌面配置备份恢复"));
            }
            ConfigRecoveryState::Unsupported => {
                return Err(invalid(
                    "当前配置版本不支持，请使用配套版本读取，不能离线覆盖",
                ));
            }
            _ => {}
        }
        let bytes = read_backup(&self.store.directory.join("backups").join(id))?;
        let mut recovered = decode(&bytes)?;
        if recovered.revision != revision {
            return Err(invalid("备份内容与文件修订不一致"));
        }
        let (backups, mut unreadable) = self.store.scan_backups()?;
        let mut max_revision = revision;
        let mut reserved = BTreeSet::new();
        let mut hash = Sha256::new();
        hash_part(&mut hash, b"offline-config-recovery-v1");
        hash_part(
            &mut hash,
            self.store.data_directory().as_os_str().as_encoded_bytes(),
        );
        hash_part(&mut hash, id.as_bytes());
        hash_part(&mut hash, &bytes);
        hash_part(
            &mut hash,
            &serde_json::to_vec(&current).map_err(|_| invalid("无法生成恢复预览"))?,
        );
        let mut scanned_bytes = 0_u64;
        for backup in backups {
            max_revision = max_revision.max(backup.revision);
            hash_part(&mut hash, backup.id.as_bytes());
            // 总读取量有界，不对任意数量的配置逐个分配内存。
            if scanned_bytes.saturating_add(u64::from(backup.bytes)) > SCAN_BYTES_LIMIT {
                return Err(invalid(
                    "离线恢复备份总量超过 64 MiB，请先归档旧备份后重新预览",
                ));
            }
            match read_backup(&self.store.directory.join("backups").join(&backup.id)) {
                Ok(bytes) => {
                    scanned_bytes = scanned_bytes.saturating_add(bytes.len() as u64);
                    if scanned_bytes > SCAN_BYTES_LIMIT {
                        return Err(invalid(
                            "离线恢复备份总量超过 64 MiB，请先归档旧备份后重新预览",
                        ));
                    }
                    hash_part(&mut hash, &bytes);
                    match decode(&bytes) {
                        Ok(config) if config.revision == backup.revision => {
                            reserved.extend(config.instances.iter().map(|instance| instance.id));
                            reserved.extend(config.reserved_instance_ids);
                        }
                        _ => unreadable += 1,
                    }
                }
                Err(_) => {
                    hash_part(&mut hash, b"unreadable");
                    unreadable += 1;
                }
            }
        }
        // 即使最新配置完全无法解析，也不把已有历史的编号交给新账号。
        self.reserve_history_ids(&mut reserved)?;
        reserved.extend(recovered.instances.iter().map(|instance| instance.id));
        reserved.extend(&recovered.reserved_instance_ids);
        recovered.reserved_instance_ids = reserved.into_iter().collect();
        // 正常保存时备份落后一版；额外留一版给已经损坏的当前配置。
        recovered.revision = max_revision
            .checked_add(2)
            .ok_or_else(|| invalid("配置修订号已用尽"))?;
        // 语法完整但字段校验失败时，也保留可辨认的编号与修订；不修补或回显原文。
        if let Some(value) = current_bytes
            .as_deref()
            .and_then(|bytes| serde_yaml::from_slice::<serde_yaml::Value>(bytes).ok())
        {
            if let Some(revision) = value.get("revision").and_then(|value| value.as_u64()) {
                let revision = u32::try_from(revision)
                    .ok()
                    .and_then(|revision| revision.checked_add(1))
                    .ok_or_else(|| invalid("损坏配置的修订号已用尽"))?;
                recovered.revision = recovered.revision.max(revision);
            }
            let mut ids: BTreeSet<_> = recovered.reserved_instance_ids.iter().copied().collect();
            if let Some(instances) = value.get("instances").and_then(|value| value.as_sequence()) {
                ids.extend(
                    instances
                        .iter()
                        .filter_map(|item| item.get("id").and_then(|id| id.as_u64()))
                        .filter(|id| (1..=9999).contains(id))
                        .map(|id| id as u32),
                );
            }
            if let Some(reserved) = value
                .get("reserved_instance_ids")
                .and_then(|value| value.as_sequence())
            {
                ids.extend(
                    reserved
                        .iter()
                        .filter_map(|id| id.as_u64())
                        .filter(|id| (1..=9999).contains(id))
                        .map(|id| id as u32),
                );
            }
            recovered.reserved_instance_ids = ids.into_iter().collect();
        }
        recovered.validate()?;
        hash_part(
            &mut hash,
            &serde_json::to_vec(&recovered).map_err(|_| invalid("无法生成恢复预览"))?,
        );
        hash_part(&mut hash, &unreadable.to_le_bytes());
        Ok(Plan {
            preview: ConfigRecoveryPreview {
                data_directory: self.store.data_directory().display().to_string(),
                id: id.into(),
                preview_id: hex::encode(hash.finalize()),
                current,
                recovered,
                unreadable_backups: unreadable,
                warnings: vec![
                    "恢复内容与导入来源以所选备份为准；损坏文件无法完整核对，请确认账号对应实际游戏登录。".into(),
                    "历史、每日完成记录、导入原文、资源选择和模型缓存保留；只恢复账号与通用设置。".into(),
                    "修订依据现存备份重新递增；已知备份和历史中的账号编号均保留，不分配给新账号。".into(),
                ],
            },
            current_bytes,
        })
    }

    fn reserve_history_ids(&self, reserved: &mut BTreeSet<u32>) -> Result<(), ConfigError> {
        let directory = self.store.data_directory().join("runs/accounts");
        reject_links(&directory)?;
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for (count, entry) in entries.enumerate() {
            if count >= 10_000 {
                return Err(invalid("账号历史目录过多，请先整理后重新预览"));
            }
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| invalid("账号历史目录名无效"))?;
            let id: u32 = name.parse().map_err(|_| invalid("账号历史目录名无效"))?;
            reject_links(&entry.path())?;
            if !(1..=9999).contains(&id)
                || name != format!("{id:04}")
                || !entry.file_type()?.is_dir()
            {
                return Err(invalid("账号历史目录编号或类型无效"));
            }
            reserved.insert(id);
        }
        Ok(())
    }

    pub fn restore(&self, id: &str, preview_id: &str) -> Result<ConfigRecoveryResult, ConfigError> {
        if preview_id.len() != 64
            || !preview_id
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(invalid("请先预览并提供有效的离线恢复指纹"));
        }
        let _lock = self.store.lock()?;
        let plan = self.plan(id)?;
        if plan.preview.preview_id != preview_id {
            return Err(invalid("当前文件、备份或账号历史已变化，请重新预览"));
        }
        let config = plan.preview.recovered;
        let bytes = serde_yaml::to_string(&config).map_err(|_| invalid("账号配置无法序列化"))?;
        if bytes.len() as u64 > super::MAX_CONFIG_BYTES {
            return Err(invalid("恢复配置过大，未保存"));
        }
        let token = Uuid::new_v4();
        let temporary = self.store.directory.join(format!("accounts-{token}.tmp"));
        let preserved_file = plan
            .current_bytes
            .as_ref()
            .map(|_| format!("config/recovery/accounts-damaged-{token}.yml"));
        let recovered_backup = format!("accounts-r{}-{token}.yml", config.revision);
        let result = (|| -> Result<(), ConfigError> {
            write_new(&temporary, bytes.as_bytes())?;
            if let (Some(original), Some(relative)) = (&plan.current_bytes, &preserved_file) {
                let path = self.store.data_directory().join(relative);
                reject_links(&path)?;
                fs::create_dir_all(path.parent().expect("固定恢复目录"))?;
                write_new(&path, original)?;
            }
            // 同时保留有效恢复结果，后续文件再次丢失也不会重复使用旧修订和编号。
            let backup_path = self.store.directory.join("backups").join(&recovered_backup);
            reject_links(&backup_path)?;
            write_new(&backup_path, bytes.as_bytes())?;
            if self.current_bytes()? != plan.current_bytes {
                return Err(invalid("当前文件已变化，未替换；请重新预览"));
            }
            fs::rename(&temporary, self.store.directory.join("accounts.yml"))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        Ok(ConfigRecoveryResult {
            config,
            preserved_file,
            recovered_backup,
        })
    }
}
