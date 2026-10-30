//! 先保存待提交结算，再原子更新账号周记录；恢复只写记录。
use crate::{
    config::reject_links,
    daily::{GameDay, GameRegion},
    lost_void_claim_record::LostVoidClaimRecord,
    lost_void_record::{LostVoidAccountLedger, LostVoidProgress, LostVoidSettlementRecord},
};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const RECORD_BYTES: u64 = 64 * 1024;
const CLAIM_BYTES: u64 = 512 * 1024;
const LEDGER_BYTES: u64 = 32 * 1024 * 1024;
const PENDING_LIMIT: usize = 10_000;
const SCAN_BYTES: u64 = 64 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum LostVoidRecordError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("迷失之地记录正在被其他任务更新")]
    Locked,
    #[error("迷失之地记录读写失败：{0}")]
    Io(#[from] std::io::Error),
}
use LostVoidRecordError::{Invalid, Locked};
type Result<T> = std::result::Result<T, LostVoidRecordError>;
fn date(day: &GameDay) -> Result<NaiveDate> {
    day.validate().map_err(Invalid)?;
    NaiveDate::parse_from_str(&day.date, "%Y-%m-%d").map_err(|_| Invalid("记录游戏日无效"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Week {
    region: GameRegion,
    year: i32,
    number: u32,
}
impl Week {
    fn from_day(day: &GameDay) -> Result<Self> {
        let week = date(day)?.iso_week();
        Ok(Self {
            region: day.region,
            year: week.year(),
            number: week.week(),
        })
    }
    fn region_key(&self) -> &'static str {
        match self.region {
            GameRegion::Cn => "cn",
            GameRegion::CnB => "cn_b",
            GameRegion::Us => "us",
            GameRegion::Eu => "eu",
            GameRegion::Asia => "asia",
            GameRegion::Twhkmo => "twhkmo",
        }
    }
    fn file_name(&self) -> String {
        format!(
            "{}-{}-W{:02}.json",
            self.region_key(),
            self.year,
            self.number
        )
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWeek {
    schema_version: u32,
    instance_id: u32,
    week: Week,
    ledger: LostVoidAccountLedger,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    schema_version: u32,
    record: LostVoidSettlementRecord,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredClaim {
    schema_version: u32,
    record: LostVoidClaimRecord,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LostVoidClaimAcknowledgement {
    pub claim_id: Uuid,
    pub changed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct LostVoidApplicationPlanning {
    pub progress: LostVoidProgress,
    pub pending_claims: Vec<Uuid>,
    pub recovered_settlements: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LostVoidRecordCommit {
    pub settlement_id: Uuid,
    pub changed: bool,
}
#[derive(Clone)]
pub struct LostVoidRecordStore {
    directory: PathBuf,
}

fn plain(path: &Path) -> Result<()> {
    reject_links(path).map_err(|_| Invalid("迷失之地记录路径包含联接或无法安全检查"))
}
fn directory(path: &Path) -> Result<()> {
    plain(path)?;
    for parent in path.ancestors() {
        match fs::metadata(parent) {
            Ok(m) if !m.is_dir() => return Err(Invalid("迷失之地记录目录被同名文件占用")),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn read<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> Result<T> {
    plain(path)?;
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(Invalid("迷失之地记录不是普通文件或超过容量"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Invalid("迷失之地记录超过容量"));
    }
    serde_json::from_slice(&bytes).map_err(|_| Invalid("迷失之地记录损坏或字段不兼容"))
}
fn write<T: Serialize>(path: &Path, value: &T, limit: u64) -> Result<()> {
    plain(path)?;
    let bytes = serde_json::to_vec(value).map_err(|_| Invalid("迷失之地记录无法序列化"))?;
    if bytes.len() as u64 > limit {
        return Err(Invalid("迷失之地记录超过容量，未写入"));
    }
    let parent = path.parent().ok_or(Invalid("迷失之地记录缺少目录"))?;
    directory(parent)?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!("lost-void-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        plain(path)?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
impl LostVoidRecordStore {
    pub fn new(data: &Path) -> Result<Self> {
        if !data.is_absolute() {
            return Err(Invalid("迷失之地数据目录必须为绝对路径"));
        }
        let path = data.join("runs/lost-void");
        directory(&path)?;
        Ok(Self { directory: path })
    }
    fn lock(&self) -> Result<File> {
        let path = self.directory.join("owner.lock");
        plain(&path)?;
        fs::create_dir_all(&self.directory)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.try_lock().map_err(|e| match e {
            std::fs::TryLockError::WouldBlock => Locked,
            std::fs::TryLockError::Error(e) => e.into(),
        })?;
        Ok(file)
    }
    fn week_path(&self, instance_id: u32, week: &Week) -> Result<PathBuf> {
        if !(1..=9999).contains(&instance_id) {
            return Err(Invalid("迷失之地账号编号须为 1..9999"));
        }
        let path = self
            .directory
            .join("accounts")
            .join(format!("{instance_id:04}"))
            .join(week.file_name());
        directory(path.parent().ok_or(Invalid("迷失之地周记录目录缺失"))?)?;
        plain(&path)?;
        Ok(path)
    }
    fn week_ledger(&self, instance_id: u32, week: &Week) -> Result<LostVoidAccountLedger> {
        let path = self.week_path(instance_id, week)?;
        if !path.try_exists()? {
            return self.committed_week(instance_id, week);
        }
        let saved: StoredWeek = read(&path, LEDGER_BYTES)?;
        saved.ledger.validate().map_err(Invalid)?;
        if saved.schema_version != 1
            || saved.instance_id != instance_id
            || saved.ledger.instance_id() != instance_id
            || saved.week != *week
            || saved.ledger.records().any(|r| {
                r.instance_id() != instance_id
                    || Week::from_day(r.day()).ok().as_ref() != Some(week)
            })
        {
            return Err(Invalid("迷失之地周记录版本、账号或日期与文件不符"));
        }
        Ok(saved.ledger)
    }
    pub fn progress(&self, instance_id: u32, day: &GameDay) -> Result<LostVoidProgress> {
        let week = Week::from_day(day)?;
        self.reject_future_week(instance_id, &week)?;
        // 周汇总可以从证据重建；丢失汇总不能掩盖时钟回退。
        for path in self.record_paths(&self.directory.join("committed"))? {
            let record = self.pending(&path)?;
            if record.instance_id() == instance_id
                && record.day().region == day.region
                && date(record.day())? > date(day)?
            {
                return Err(Invalid("迷失之地已提交结算日期晚于当前游戏日"));
            }
        }
        let mut progress = self
            .week_ledger(instance_id, &week)?
            .progress(day)
            .map_err(Invalid)?;
        for claim in self.claims(instance_id, day)? {
            progress.bounty_commission_complete |=
                claim.proves_full_bounty(day).map_err(Invalid)?;
        }
        Ok(progress)
    }
    /// 当前周已记账结算减去安全结束的领取检查；只读，不修改次数。
    pub fn pending_claims(&self, instance_id: u32, day: &GameDay) -> Result<Vec<Uuid>> {
        self.progress(instance_id, day)?;
        let week = Week::from_day(day)?;
        let ledger = self.week_ledger(instance_id, &week)?;
        let mut pending: std::collections::BTreeSet<_> =
            ledger.records().map(|r| r.settlement_id()).collect();
        for claim in self.claims(instance_id, day)? {
            for id in claim.source_ids() {
                pending.remove(id);
            }
        }
        Ok(pending.into_iter().collect())
    }
    /// 应用执行时先补交记录，再在同一写锁内读取目标进度和待领取批次。
    /// 此入口可创建运行锁；只读预览继续使用 progress / pending_claims。
    pub fn application_planning(
        &self,
        instance_id: u32,
        day: &GameDay,
    ) -> Result<LostVoidApplicationPlanning> {
        if !(1..=9999).contains(&instance_id) {
            return Err(Invalid("迷失之地账号编号须为 1..9999"));
        }
        day.validate().map_err(Invalid)?;
        let _lock = self.lock()?;
        let recovered_settlements = self.recover_locked()?;
        let progress = self.progress(instance_id, day)?;
        let pending_claims = self.pending_claims(instance_id, day)?;
        Ok(LostVoidApplicationPlanning {
            progress,
            pending_claims,
            recovered_settlements,
        })
    }
    fn claim_path(&self, id: Uuid) -> Result<PathBuf> {
        if id.is_nil() {
            return Err(Invalid("领取检查身份无效"));
        }
        let path = self.directory.join("claims").join(format!("{id}.json"));
        directory(path.parent().ok_or(Invalid("领取检查目录缺失"))?)?;
        plain(&path)?;
        Ok(path)
    }
    /// 只读核对应用历史，不扫描或创建运行目录。
    pub fn committed_record(&self, id: Uuid) -> Result<LostVoidSettlementRecord> {
        if id.is_nil() {
            return Err(Invalid("已提交结算身份无效"));
        }
        let folder = self.directory.join("committed");
        directory(&folder)?;
        self.pending(&folder.join(format!("{id}.json")))
    }
    pub fn claim_record(&self, id: Uuid) -> Result<LostVoidClaimRecord> {
        self.claim_file(&self.claim_path(id)?)
    }
    fn claim_file(&self, path: &Path) -> Result<LostVoidClaimRecord> {
        let saved: StoredClaim = read(path, CLAIM_BYTES)?;
        saved.record.validate().map_err(Invalid)?;
        if saved.schema_version != 1
            || path
                .file_name()
                .is_none_or(|n| n != format!("{}.json", saved.record.claim_id()).as_str())
        {
            return Err(Invalid("领取检查版本或文件身份不一致"));
        }
        Ok(saved.record)
    }
    fn claims(&self, instance_id: u32, day: &GameDay) -> Result<Vec<LostVoidClaimRecord>> {
        let week = Week::from_day(day)?;
        let mut claims = Vec::new();
        for path in self.record_paths(&self.directory.join("claims"))? {
            let claim = self.claim_file(&path)?;
            if claim.instance_id() != instance_id || claim.day().region != day.region {
                continue;
            }
            if date(claim.day())? > date(day)? {
                return Err(Invalid("领取检查日期晚于当前游戏日"));
            }
            if Week::from_day(claim.day())? == week {
                claims.push(claim);
            }
        }
        Ok(claims)
    }
    /// 只保存已完成检查的证据；不能从本接口恢复或投递领取输入。
    pub fn acknowledge_claim(
        &self,
        claim: &LostVoidClaimRecord,
    ) -> Result<LostVoidClaimAcknowledgement> {
        claim.validate().map_err(Invalid)?;
        let _lock = self.lock()?;
        let path = self.claim_path(claim.claim_id())?;
        if path.try_exists()? {
            if self.claim_file(&path)? != *claim {
                return Err(Invalid("同一领取检查的内容或绑定结算发生变化"));
            }
            return Ok(LostVoidClaimAcknowledgement {
                claim_id: claim.claim_id(),
                changed: false,
            });
        }
        // 完成日、账号及区服必须与本批结算一致；后提交的其他结算不被顺带清除。
        let week = Week::from_day(claim.day())?;
        let ledger = self.week_ledger(claim.instance_id(), &week)?;
        let known: std::collections::BTreeSet<_> =
            ledger.records().map(|r| r.settlement_id()).collect();
        if claim.source_ids().iter().any(|id| !known.contains(id)) {
            return Err(Invalid("领取检查绑定的结算不属于该账号与游戏周"));
        }
        let claimed_day = date(claim.day())?;
        for record in ledger.records() {
            if claim
                .source_ids()
                .binary_search(&record.settlement_id())
                .is_ok()
                && date(record.day())? > claimed_day
            {
                return Err(Invalid("领取检查早于其绑定结算的完成日"));
            }
        }
        // 延迟写回仍归原日；更晚的无关结算不阻止原批次提交。
        let saved_claims = self.record_paths(&self.directory.join("claims"))?;
        if saved_claims.len() >= PENDING_LIMIT {
            return Err(Invalid("领取检查记录数量超过容量，未清除待领取"));
        }
        for existing in saved_claims {
            self.claim_file(&existing)?;
        }
        write(
            &path,
            &StoredClaim {
                schema_version: 1,
                record: claim.clone(),
            },
            CLAIM_BYTES,
        )?;
        Ok(LostVoidClaimAcknowledgement {
            claim_id: claim.claim_id(),
            changed: true,
        })
    }
    fn reject_future_week(&self, instance_id: u32, week: &Week) -> Result<()> {
        let path = self.week_path(instance_id, week)?;
        let parent = path.parent().ok_or(Invalid("迷失之地周记录目录缺失"))?;
        if !parent.try_exists()? {
            return Ok(());
        }
        let prefix = format!("{}-", week.region_key());
        for (index, entry) in fs::read_dir(parent)?.enumerate() {
            if index >= PENDING_LIMIT {
                return Err(Invalid("迷失之地周记录文件数量超过容量"));
            }
            let entry = entry?;
            plain(&entry.path())?;
            if !entry.file_type()?.is_file() {
                return Err(Invalid("迷失之地周记录路径不是普通文件"));
            }
            let name = entry.file_name();
            let name = name.to_str().ok_or(Invalid("迷失之地周记录文件名无效"))?;
            let Some(rest) = name
                .strip_prefix(&prefix)
                .and_then(|s| s.strip_suffix(".json"))
            else {
                continue;
            };
            let (year, number) = rest
                .rsplit_once("-W")
                .ok_or(Invalid("迷失之地周记录文件名无效"))?;
            let year = year
                .parse::<i32>()
                .map_err(|_| Invalid("迷失之地周记录年份无效"))?;
            let number = number
                .parse::<u32>()
                .map_err(|_| Invalid("迷失之地周记录周数无效"))?;
            if NaiveDate::from_isoywd_opt(year, number, chrono::Weekday::Mon).is_none()
                || (Week {
                    region: week.region,
                    year,
                    number,
                })
                .file_name()
                    != name
            {
                return Err(Invalid("迷失之地周记录文件名无效"));
            }
            if (year, number) > (week.year, week.number) {
                return Err(Invalid("迷失之地结算周晚于当前游戏周"));
            }
        }
        Ok(())
    }
    fn pending_path(&self, id: Uuid) -> Result<PathBuf> {
        if id.is_nil() {
            return Err(Invalid("迷失之地待提交结算身份无效"));
        }
        let path = self.directory.join("pending").join(format!("{id}.json"));
        directory(path.parent().ok_or(Invalid("迷失之地待提交目录缺失"))?)?;
        plain(&path)?;
        Ok(path)
    }
    fn pending(&self, path: &Path) -> Result<LostVoidSettlementRecord> {
        let saved: Pending = read(path, RECORD_BYTES)?;
        saved.record.validate().map_err(Invalid)?;
        if saved.schema_version != 1
            || path
                .file_name()
                .is_none_or(|n| n != format!("{}.json", saved.record.settlement_id()).as_str())
        {
            return Err(Invalid("迷失之地待提交文件与结算身份不符"));
        }
        Ok(saved.record)
    }
    fn committed_week(&self, instance_id: u32, week: &Week) -> Result<LostVoidAccountLedger> {
        let mut ledger = LostVoidAccountLedger::new(instance_id).map_err(Invalid)?;
        for path in self.record_paths(&self.directory.join("committed"))? {
            let record = self.pending(&path)?;
            if record.instance_id() == instance_id && Week::from_day(record.day())? == *week {
                ledger.commit(record).map_err(Invalid)?;
            }
        }
        Ok(ledger)
    }
    fn commit_locked(&self, record: &LostVoidSettlementRecord) -> Result<LostVoidRecordCommit> {
        record.validate().map_err(Invalid)?;
        let committed = self
            .directory
            .join("committed")
            .join(format!("{}.json", record.settlement_id()));
        plain(&committed)?;
        if committed.try_exists()? && self.pending(&committed)? != *record {
            return Err(Invalid("已提交结算的日期、账号或内容发生变化"));
        }
        let pending = self.pending_path(record.settlement_id())?;
        if pending.try_exists()? {
            if self.pending(&pending)? != *record {
                return Err(Invalid("同一待提交结算的内容发生变化"));
            }
        } else {
            write(
                &pending,
                &Pending {
                    schema_version: 1,
                    record: record.clone(),
                },
                RECORD_BYTES,
            )?;
        }
        let week = Week::from_day(record.day())?;
        let mut ledger = self.week_ledger(record.instance_id(), &week)?;
        let changed = ledger.commit(record.clone()).map_err(Invalid)?;
        if changed || !self.week_path(record.instance_id(), &week)?.try_exists()? {
            write(
                &self.week_path(record.instance_id(), &week)?,
                &StoredWeek {
                    schema_version: 1,
                    instance_id: record.instance_id(),
                    week,
                    ledger,
                },
                LEDGER_BYTES,
            )?;
        }
        if !committed.try_exists()? {
            if self.record_paths(&self.directory.join("committed"))?.len() >= PENDING_LIMIT {
                return Err(Invalid("迷失之地已提交证据数量超过容量，待提交文件已保留"));
            }
            write(
                &committed,
                &Pending {
                    schema_version: 1,
                    record: record.clone(),
                },
                RECORD_BYTES,
            )?;
        }
        plain(&pending)?;
        fs::remove_file(pending)?;
        Ok(LostVoidRecordCommit {
            settlement_id: record.settlement_id(),
            changed,
        })
    }
    pub fn commit(&self, record: &LostVoidSettlementRecord) -> Result<LostVoidRecordCommit> {
        let _lock = self.lock()?;
        self.commit_locked(record)
    }
    /// 仅幂等提交已保存证据，不提供任何游戏继续资格。
    pub fn recover_pending(&self) -> Result<u32> {
        let _lock = self.lock()?;
        self.recover_locked()
    }
    fn recover_locked(&self) -> Result<u32> {
        let paths = self.record_paths(&self.directory.join("pending"))?;
        let mut changed = 0;
        for path in paths {
            let record = self.pending(&path)?;
            changed += u32::from(self.commit_locked(&record)?.changed);
        }
        Ok(changed)
    }
    fn record_paths(&self, path: &Path) -> Result<Vec<PathBuf>> {
        directory(path)?;
        if !path.try_exists()? {
            return Ok(Vec::new());
        }
        let mut paths = Vec::new();
        let mut bytes = 0_u64;
        for (index, entry) in fs::read_dir(path)?.enumerate() {
            if index >= PENDING_LIMIT {
                return Err(Invalid("迷失之地待提交文件数量超过容量"));
            }
            let entry = entry?;
            let path = entry.path();
            plain(&path)?;
            if !entry.file_type()?.is_file() {
                return Err(Invalid("迷失之地待提交路径不是普通文件"));
            }
            if path.extension().is_some_and(|e| e == "tmp") {
                continue;
            }
            if path.extension().is_none_or(|e| e != "json") {
                return Err(Invalid("迷失之地待提交目录含未知文件"));
            }
            bytes = bytes.saturating_add(entry.metadata()?.len());
            if bytes > SCAN_BYTES {
                return Err(Invalid("迷失之地记录扫描超过容量"));
            }
            paths.push(path);
        }
        paths.sort();
        Ok(paths)
    }
}
