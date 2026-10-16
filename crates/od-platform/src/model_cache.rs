pub use crate::download::{
    HttpsDownloader as HttpsModelDownloader, ResourceDownloader as ModelDownloader,
};
use anyhow::{Context, Result, ensure};
pub use od_core::model_resources::{CacheState, ModelCacheStatus, ModelProgress, PreparedModel};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;

#[derive(Debug, Deserialize, Serialize)]
pub struct ModelLock {
    pub schema_version: u32,
    pub reference_commit: String,
    pub sdk_version: String,
    pub hash_source: String,
    pub models: Vec<ModelAsset>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ModelAsset {
    pub id: String,
    pub role: String,
    pub archive: String,
    pub bytes: u64,
    pub url: String,
    pub sha256: String,
    pub files: Vec<ModelFile>,
    pub known_differences: Vec<String>,
    #[serde(default)]
    pub adaptation: Option<ModelAdaptation>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ModelAdaptation {
    pub kind: String,
    pub bundle_path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ModelFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub bundle_path: Option<String>,
}

pub fn locked_models() -> Result<ModelLock> {
    let lock: ModelLock = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    ensure!(lock.schema_version == 1, "模型清单版本无效");
    let mut ids = BTreeSet::new();
    for model in &lock.models {
        validate_model(model)?;
        ensure!(ids.insert(&model.id), "模型清单编号重复");
    }
    ensure!(!ids.is_empty(), "模型清单不能为空");
    Ok(lock)
}

fn validate_model(model: &ModelAsset) -> Result<()> {
    ensure!(
        !model.id.is_empty()
            && model
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "模型编号无效"
    );
    ensure!(
        model.archive == format!("{}.zip", model.id),
        "模型压缩包名称无效"
    );
    ensure!(
        model
            .url
            .starts_with("https://github.com/OneDragon-Anything/")
            && model.url.ends_with(&format!("/{}", model.archive)),
        "模型来源不是锁定的官方发布包"
    );
    ensure!(!model.files.is_empty(), "模型文件清单不能为空");
    let mut paths = BTreeSet::new();
    for file in &model.files {
        ensure!(
            !file.path.contains(['/', '\\']),
            "锁定模型文件不能包含子目录"
        );
        safe_zip_name(&file.path)?;
        ensure!(
            paths.insert(file.path.to_ascii_lowercase()),
            "模型文件路径重复"
        );
        ensure!(
            file.bytes > 0
                && file.sha256.len() == 64
                && file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "模型文件校验信息无效"
        );
    }
    ensure!(
        model.bytes > 0
            && model.sha256.len() == 64
            && model.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "模型压缩包校验信息无效"
    );
    Ok(())
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Acquire), "模型资源准备已取消");
    Ok(())
}

// 不穿过 Windows 联接或符号链接。缓存根目录本身也要检查。
fn check_path(path: &Path) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    for part in path.ancestors() {
        match std::fs::symlink_metadata(part) {
            Ok(metadata) => ensure!(
                metadata.file_attributes() & 0x400 == 0,
                "模型缓存路径不能包含联接或符号链接：{}",
                part.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn selected_models(ids: &[String]) -> Result<Vec<ModelAsset>> {
    let lock = locked_models()?;
    let selected: BTreeSet<_> = ids.iter().map(String::as_str).collect();
    ensure!(selected.len() == ids.len(), "模型编号不能重复");
    for id in &selected {
        ensure!(
            lock.models.iter().any(|model| &model.id == id),
            "未知模型编号：{id}"
        );
    }
    Ok(lock
        .models
        .into_iter()
        .filter(|model| selected.is_empty() || selected.contains(model.id.as_str()))
        .collect())
}

fn file_state(path: &Path, bytes: u64, hash: &str, cancel: &AtomicBool) -> Result<CacheState> {
    check_path(path)?;
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CacheState::Missing);
        }
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.len() != bytes {
        return Ok(CacheState::Invalid);
    }
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 65536];
    let mut count = 0_u64;
    loop {
        cancelled(cancel)?;
        let length = file.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        count += length as u64;
        if count > bytes {
            return Ok(CacheState::Invalid);
        }
        digest.update(&buffer[..length]);
    }
    Ok(
        if count == bytes && hex::encode(digest.finalize()) == hash {
            CacheState::Valid
        } else {
            CacheState::Invalid
        },
    )
}

fn files_state(directory: &Path, model: &ModelAsset, cancel: &AtomicBool) -> Result<CacheState> {
    check_path(directory)?;
    if !directory.try_exists()? {
        return Ok(CacheState::Missing);
    }
    if !directory.is_dir() {
        return Ok(CacheState::Invalid);
    }
    for file in &model.files {
        if file_state(
            &directory.join(&file.path),
            file.bytes,
            &file.sha256,
            cancel,
        )? != CacheState::Valid
        {
            return Ok(CacheState::Invalid);
        }
    }
    Ok(CacheState::Valid)
}

pub fn cached_files_valid(cache: &Path, model: &ModelAsset) -> bool {
    if validate_model(model).is_err() {
        return false;
    }
    let Ok(cache) = std::path::absolute(cache) else {
        return false;
    };
    files_state(&cache.join(&model.id), model, &AtomicBool::new(false))
        .is_ok_and(|state| state == CacheState::Valid)
}

/// 只读状态检查。不存在的缓存目录不会因此创建。
pub fn model_cache_status(cache: &Path, ids: &[String]) -> Result<Vec<ModelCacheStatus>> {
    model_cache_status_cancellable(cache, ids, &AtomicBool::new(false))
}

pub fn model_cache_status_cancellable(
    cache: &Path,
    ids: &[String],
    cancel: &AtomicBool,
) -> Result<Vec<ModelCacheStatus>> {
    cancelled(cancel)?;
    let models = selected_models(ids)?;
    let cache = std::path::absolute(cache)?;
    check_path(&cache)?;
    let lock_path = cache.join(".models.lock");
    check_path(&lock_path)?;
    let _lock = match File::open(&lock_path) {
        Ok(file) => {
            file.try_lock_shared()
                .context("模型缓存正在准备，请稍后查询")?;
            Some(file)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    models
        .iter()
        .map(|model| {
            Ok(ModelCacheStatus {
                id: model.id.clone(),
                archive: file_state(
                    &cache.join(&model.archive),
                    model.bytes,
                    &model.sha256,
                    cancel,
                )?,
                files: files_state(&cache.join(&model.id), model, cancel)?,
            })
        })
        .collect()
}

struct WorkDirectory(PathBuf);

impl WorkDirectory {
    fn new(cache: &Path) -> Result<Self> {
        let path = cache.join(format!(".prepare-{}", Uuid::new_v4()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for WorkDirectory {
    fn drop(&mut self) {
        // 临时目录只含本次创建的普通文件；逐个删除，不进行递归删除。
        if check_path(&self.0).is_err() {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_file()) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn progress(
    model: &ModelAsset,
    phase: &'static str,
    completed: u64,
    observer: &mut impl FnMut(ModelProgress),
) {
    observer(ModelProgress {
        id: model.id.clone(),
        phase: phase.into(),
        completed_bytes: completed,
        total_bytes: model.bytes,
    });
}

fn download(
    model: &ModelAsset,
    destination: &Path,
    downloader: &impl ModelDownloader,
    cancel: &AtomicBool,
    observer: &mut impl FnMut(ModelProgress),
) -> Result<()> {
    cancelled(cancel)?;
    let mut source = downloader
        .open_cancellable(&model.url, cancel)
        .with_context(|| format!("下载模型失败：{}", model.id))?;
    let mut target = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut buffer = [0_u8; 65536];
    let mut digest = Sha256::new();
    let mut count = 0_u64;
    loop {
        cancelled(cancel)?;
        let length = source.read(&mut buffer)?;
        cancelled(cancel)?;
        if length == 0 {
            break;
        }
        count += length as u64;
        ensure!(count <= model.bytes, "模型下载超过锁定大小：{}", model.id);
        target.write_all(&buffer[..length])?;
        digest.update(&buffer[..length]);
        progress(model, "downloading", count, observer);
    }
    ensure!(
        count == model.bytes && hex::encode(digest.finalize()) == model.sha256,
        "模型下载大小或 SHA-256 校验失败：{}",
        model.id
    );
    target.sync_all()?;
    Ok(())
}

fn safe_zip_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && !name.starts_with('/') && !name.contains(['\\', ':', '\0']),
        "模型 ZIP 路径无效"
    );
    ensure!(
        name.trim_end_matches('/')
            .split('/')
            .all(|part| !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with([' ', '.'])),
        "模型 ZIP 路径越界或含歧义"
    );
    Ok(())
}

fn extract(model: &ModelAsset, archive: &Path, output: &Path, cancel: &AtomicBool) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    ensure!(zip.len() <= 1024, "模型 ZIP 条目过多");
    let mut names = BTreeSet::new();
    for index in 0..zip.len() {
        cancelled(cancel)?;
        let entry = zip.by_index(index)?;
        safe_zip_name(entry.name())?;
        ensure!(entry.enclosed_name().is_some(), "模型 ZIP 路径越界");
        ensure!(
            names.insert(entry.name().trim_end_matches('/').to_ascii_lowercase()),
            "模型 ZIP 条目重复"
        );
        if let Some(mode) = entry.unix_mode() {
            ensure!(
                matches!(mode & 0o170000, 0 | 0o100000 | 0o040000),
                "模型 ZIP 不能包含链接或特殊文件"
            );
        }
    }
    for file in &model.files {
        // 当前锁定清单只使用平铺文件，新增嵌套布局时需要显式迁移。
        ensure!(
            !file.path.contains(['/', '\\']),
            "锁定模型文件不能包含子目录"
        );
        let mut entry = zip
            .by_name(&file.path)
            .with_context(|| format!("模型 ZIP 缺少 {}", file.path))?;
        ensure!(
            !entry.is_dir() && entry.size() == file.bytes,
            "模型 ZIP 文件大小不符：{}",
            file.path
        );
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join(&file.path))?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 65536];
        let mut count = 0_u64;
        loop {
            cancelled(cancel)?;
            let length = entry.read(&mut buffer)?;
            if length == 0 {
                break;
            }
            count += length as u64;
            ensure!(count <= file.bytes, "模型 ZIP 解压超过锁定大小");
            target.write_all(&buffer[..length])?;
            digest.update(&buffer[..length]);
        }
        ensure!(
            count == file.bytes && hex::encode(digest.finalize()) == file.sha256,
            "解压模型 SHA-256 校验失败：{}",
            file.path
        );
        target.sync_all()?;
    }
    Ok(())
}

fn promote(staged: &Path, destination: &Path, cache: &Path) -> Result<()> {
    check_path(destination)?;
    let backup = if destination.try_exists()? {
        let backup = cache.join(format!(
            ".invalid-{}-{}",
            destination.file_name().unwrap().to_string_lossy(),
            Uuid::new_v4()
        ));
        std::fs::rename(destination, &backup).context("无法保留旧模型缓存")?;
        Some(backup)
    } else {
        None
    };
    if let Err(error) = std::fs::rename(staged, destination) {
        if let Some(backup) = backup {
            std::fs::rename(&backup, destination).with_context(|| {
                format!("启用失败且还原失败，旧内容保留在 {}", backup.display())
            })?;
        }
        return Err(error).context("无法启用已校验模型缓存");
    }
    Ok(())
}

/// 只使用编译时锁定清单。调用者负责在工作线程执行及持有取消标记。
pub fn prepare_models(
    cache: &Path,
    ids: &[String],
    offline: bool,
    downloader: &impl ModelDownloader,
    cancel: &AtomicBool,
    mut observer: impl FnMut(ModelProgress),
) -> Result<Vec<PreparedModel>> {
    let models = selected_models(ids)?;
    cancelled(cancel)?;
    let cache = std::path::absolute(cache)?;
    check_path(&cache)?;
    std::fs::create_dir_all(&cache)?;
    let cache = cache.canonicalize()?;
    let lock_path = cache.join(".models.lock");
    check_path(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.try_lock().context("另一个进程正在准备模型缓存")?;
    let mut prepared = Vec::new();
    for model in models {
        cancelled(cancel)?;
        progress(&model, "checking", 0, &mut observer);
        if files_state(&cache.join(&model.id), &model, cancel)? == CacheState::Valid {
            progress(&model, "ready", model.bytes, &mut observer);
            prepared.push(PreparedModel {
                id: model.id,
                reused: true,
                downloaded: false,
            });
            continue;
        }
        let archive = cache.join(&model.archive);
        let archive_state = file_state(&archive, model.bytes, &model.sha256, cancel)?;
        let downloaded = archive_state != CacheState::Valid;
        if downloaded {
            ensure!(!offline, "离线缓存缺少有效压缩包：{}", model.archive);
            let work = WorkDirectory::new(&cache)?;
            let partial = work.0.join("download.partial");
            download(&model, &partial, downloader, cancel, &mut observer)?;
            cancelled(cancel)?;
            promote(&partial, &archive, &cache)?;
        }
        let work = WorkDirectory::new(&cache)?;
        progress(&model, "extracting", model.bytes, &mut observer);
        extract(&model, &archive, &work.0, cancel)?;
        ensure!(
            files_state(&work.0, &model, cancel)? == CacheState::Valid,
            "解压目录复验失败"
        );
        cancelled(cancel)?;
        promote(&work.0, &cache.join(&model.id), &cache)?;
        progress(&model, "ready", model.bytes, &mut observer);
        prepared.push(PreparedModel {
            id: model.id,
            reused: false,
            downloaded,
        });
    }
    Ok(prepared)
}
