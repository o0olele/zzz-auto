//! 先保存可复验的挑战日志，再原子提交次数和去重回执；恢复不执行游戏输入。
use super::{AccountConfig, ConfigError, ConfigStore, invalid, reject_links};
use crate::charge_run::{ChargeRunContext, ChargeSchedule, MAX_ATTEMPTS};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use ts_rs::TS;
use uuid::Uuid;

const COMMIT_LIMIT: usize = 256;
const CHECKPOINT_LIMIT: u64 = 8 * 1024 * 1024;
const SCAN_LIMIT: usize = 10_000;
const SCAN_BYTES_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChargeCommit {
    pub context: ChargeRunContext,
    pub records: u32,
    pub prefix_sha256: String,
    pub saved_revision: u32,
}
pub struct ChargeCommitResult {
    pub config: AccountConfig,
    pub changed: bool,
}
#[derive(Debug, Default)]
pub struct ChargeRecovery {
    pub recovered: u32,
    pub stale: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    schema_version: u32,
    schedule: ChargeSchedule,
}

pub(super) fn validate_commits(commits: &[ChargeCommit], revision: u32) -> Result<(), ConfigError> {
    if commits.len() > COMMIT_LIMIT {
        return Err(invalid("体力提交回执超过容量"));
    }
    let mut ids = BTreeSet::new();
    for commit in commits {
        commit.context.validate().map_err(invalid)?;
        if !ids.insert(commit.context.run_id)
            || commit.records as usize > MAX_ATTEMPTS
            || commit.saved_revision <= commit.context.config_revision
            || commit.saved_revision > revision
            || commit.prefix_sha256.len() != 64
            || !commit.prefix_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid("体力提交回执身份、修订或指纹无效"));
        }
    }
    Ok(())
}
pub(crate) fn read_checkpoint(path: &Path) -> Result<ChargeSchedule, ConfigError> {
    reject_links(path)?;
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > CHECKPOINT_LIMIT {
        return Err(invalid("体力检查点不是普通文件或超过 8 MiB"));
    }
    let mut bytes = Vec::new();
    file.take(CHECKPOINT_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > CHECKPOINT_LIMIT {
        return Err(invalid("体力检查点超过 8 MiB"));
    }
    let saved: Checkpoint =
        serde_json::from_slice(&bytes).map_err(|_| invalid("体力检查点损坏或字段不兼容"))?;
    if saved.schema_version != 1 {
        return Err(invalid("体力检查点版本不支持"));
    }
    saved.schedule.validate().map_err(invalid)?;
    Ok(saved.schedule)
}
impl ConfigStore {
    fn charge_directory(&self) -> Result<PathBuf, ConfigError> {
        let directory = self.data_directory().join("runs/charge");
        reject_links(&directory)?;
        Ok(directory)
    }
    fn charge_path(&self, schedule: &ChargeSchedule) -> Result<PathBuf, ConfigError> {
        Ok(self
            .charge_directory()?
            .join(format!("{}.json", schedule.request().context.run_id)))
    }
    /// 只读取日志，不据此重新发出输入。完整重放后再核对文件名与运行身份。
    pub fn charge_checkpoint(&self, run_id: Uuid) -> Result<Option<ChargeSchedule>, ConfigError> {
        if run_id.is_nil() {
            return Err(invalid("体力运行 ID 无效"));
        }
        let path = self.charge_directory()?.join(format!("{run_id}.json"));
        reject_links(&path)?;
        if !path.try_exists()? {
            return Ok(None);
        }
        let saved = read_checkpoint(&path)?;
        if saved.request().context.run_id != run_id {
            return Err(invalid("体力检查点文件与运行身份不符"));
        }
        Ok(Some(saved))
    }
    fn write_charge_checkpoint(&self, schedule: &ChargeSchedule) -> Result<(), ConfigError> {
        let path = self.charge_path(schedule)?;
        reject_links(&path)?;
        if path.try_exists()? {
            let previous = read_checkpoint(&path)?;
            schedule.validate_successor(&previous).map_err(invalid)?;
        }
        let bytes = serde_json::to_vec(&Checkpoint {
            schema_version: 1,
            schedule: schedule.clone(),
        })
        .map_err(|_| invalid("体力检查点无法序列化"))?;
        if bytes.len() as u64 > CHECKPOINT_LIMIT {
            return Err(invalid("体力检查点超过 8 MiB"));
        }
        let directory = self.charge_directory()?;
        fs::create_dir_all(&directory)?;
        let temporary = directory.join(format!("charge-{}.tmp", Uuid::new_v4()));
        let result = (|| -> Result<(), ConfigError> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
    /// 与 accounts.yml 同一锁串行，日志先于原子配置替换。写配置失败时保留日志供重试或启动恢复。
    /// 只能由已核实挑战证明的引擎保存路径调用，不开放为任意快照写入命令。
    pub fn commit_charge_run(
        &self,
        schedule: &ChargeSchedule,
    ) -> Result<ChargeCommitResult, ConfigError> {
        schedule.validate().map_err(invalid)?;
        let _lock = self.lock()?;
        self.commit_charge_locked(schedule)
    }
    fn commit_charge_locked(
        &self,
        schedule: &ChargeSchedule,
    ) -> Result<ChargeCommitResult, ConfigError> {
        let mut config = self.read_for_startup()?;
        let request = schedule.request();
        let context = &request.context;
        let count = schedule.records().len();
        let fingerprint = schedule.prefix_fingerprint(count).map_err(invalid)?;
        let previous = config
            .charge_commits
            .iter()
            .find(|r| r.context.run_id == context.run_id)
            .cloned();
        if let Some(receipt) = &previous {
            if receipt.context != *context
                || count < receipt.records as usize
                || schedule
                    .prefix_fingerprint(receipt.records as usize)
                    .map_err(invalid)?
                    != receipt.prefix_sha256
            {
                return Err(invalid("体力提交不是已保存结果的后继"));
            }
            if count == receipt.records as usize {
                // 即使后来改过设置，也只能确认同一提交已生效，不能重新覆盖它。
                self.write_charge_checkpoint(schedule)?;
                return Ok(ChargeCommitResult {
                    config,
                    changed: false,
                });
            }
        }
        let expected_revision = previous
            .as_ref()
            .map_or(context.config_revision, |r| r.saved_revision);
        if config.revision != expected_revision {
            // 已保存输入意图的运行仍要保留随后确认的结果，冲突只阻止覆盖当前次数。
            if self.charge_path(schedule)?.try_exists()? {
                self.write_charge_checkpoint(schedule)?;
            }
            return Err(ConfigError::Conflict(Box::new(config)));
        }
        if config.active_instance != context.instance_id {
            return Err(invalid("体力提交账号不是当前账号"));
        }
        let account = config
            .instances
            .iter_mut()
            .find(|a| a.id == context.instance_id)
            .ok_or_else(|| invalid("体力提交账号不存在"))?;
        if account.game_region != context.game_day.region
            || account.predefined_teams != request.teams
        {
            return Err(invalid("体力提交区服或编队已变化"));
        }
        let group = account
            .groups
            .iter_mut()
            .find(|g| g.id == context.group_id)
            .ok_or_else(|| invalid("体力提交应用组不存在"))?;
        let expected = if let Some(receipt) = &previous {
            schedule
                .confirmed_prefix(receipt.records as usize)
                .map_err(invalid)?
                .progress()
                .map_err(invalid)?
                .config
        } else {
            request.config.clone()
        };
        if *group.charge_plan != expected {
            if self.charge_path(schedule)?.try_exists()? {
                self.write_charge_checkpoint(schedule)?;
            }
            return Err(invalid("体力提交冻结设置或计数与当前配置不符"));
        }
        let confirmed = schedule
            .confirmed_prefix(count)
            .map_err(invalid)?
            .progress()
            .map_err(invalid)?
            .config;
        let changed = *group.charge_plan != confirmed || count > 0;
        // pending 也持久保存，但其投递、成功和循环扣减均不计入用户次数。
        self.write_charge_checkpoint(schedule)?;
        if !changed {
            return Ok(ChargeCommitResult {
                config,
                changed: false,
            });
        }
        *group.charge_plan = confirmed;
        config
            .charge_commits
            .retain(|r| r.context.run_id != context.run_id);
        if config.charge_commits.len() == COMMIT_LIMIT {
            config.charge_commits.remove(0);
        }
        config.charge_commits.push(ChargeCommit {
            context: context.clone(),
            records: count as u32,
            prefix_sha256: fingerprint,
            saved_revision: config
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("配置修订号已用尽"))?,
        });
        self.save(&mut config)?;
        Ok(ChargeCommitResult {
            config,
            changed: true,
        })
    }
    /// 在持有数据目录所有权、任何游戏输入之前调用。仅补交同一配置分支上的已确认日志。
    /// 旧设置分支只保留历史；损坏、改名、超量或重复的日志拒绝自动恢复。
    pub fn recover_charge_runs(&self) -> Result<ChargeRecovery, ConfigError> {
        let _lock = self.lock()?;
        self.read_for_startup()?;
        let directory = self.charge_directory()?;
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ChargeRecovery::default());
            }
            Err(e) => return Err(e.into()),
        };
        let mut schedules = Vec::new();
        let mut scanned = 0;
        let mut scanned_bytes = 0_u64;
        for entry in entries {
            let entry = entry?;
            scanned += 1;
            if scanned > SCAN_LIMIT {
                return Err(invalid("体力检查点目录超过扫描上限"));
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("charge-") && name.ends_with(".tmp") {
                reject_links(&entry.path())?;
                continue;
            }
            let id: Uuid = name
                .strip_suffix(".json")
                .ok_or_else(|| invalid("体力检查点目录含未知文件"))?
                .parse()
                .map_err(|_| invalid("体力检查点文件名无效"))?;
            reject_links(&entry.path())?;
            scanned_bytes = scanned_bytes.saturating_add(entry.metadata()?.len());
            if scanned_bytes > SCAN_BYTES_LIMIT {
                return Err(invalid("体力检查点扫描超过 64 MiB"));
            }
            let schedule = read_checkpoint(&entry.path())?;
            if format!("{id}.json") != name || schedule.request().context.run_id != id {
                return Err(invalid("体力检查点文件与身份不符"));
            }
            schedules.push(schedule);
        }
        schedules.sort_by_key(|s| {
            (
                s.request().context.config_revision,
                s.request().context.run_id,
            )
        });
        let mut result = ChargeRecovery::default();
        let initial = self.read()?;
        let eligible = schedules
            .iter()
            .filter(|schedule| {
                let context = &schedule.request().context;
                let previous = initial
                    .charge_commits
                    .iter()
                    .find(|r| r.context.run_id == context.run_id);
                let expected = previous.map_or(context.config_revision, |r| r.saved_revision);
                initial.revision == expected
                    && previous.is_none_or(|r| schedule.records().len() > r.records as usize)
                    && (!schedule.records().is_empty()
                        || schedule
                            .confirmed_prefix(0)
                            .and_then(|s| s.progress())
                            .is_ok_and(|p| p.config != schedule.request().config))
            })
            .count();
        if eligible > 1 {
            return Err(invalid(
                "多个体力运行争用同一配置基线，请先核对历史，未自动选择结果",
            ));
        }
        for schedule in schedules {
            let config = self.read()?;
            let context = &schedule.request().context;
            let previous = config
                .charge_commits
                .iter()
                .find(|r| r.context.run_id == context.run_id);
            if let Some(receipt) = previous {
                if receipt.context != *context
                    || schedule.records().len() < receipt.records as usize
                    || schedule
                        .prefix_fingerprint(receipt.records as usize)
                        .map_err(invalid)?
                        != receipt.prefix_sha256
                {
                    return Err(invalid("体力历史与已提交次数的证明不符"));
                }
                if schedule.records().len() == receipt.records as usize {
                    continue;
                }
            }
            let expected = previous.map_or(context.config_revision, |r| r.saved_revision);
            if config.revision != expected {
                result.stale += 1;
                continue;
            }
            if self.commit_charge_locked(&schedule)?.changed {
                result.recovered += 1;
            }
        }
        Ok(result)
    }
}
