use crate::runtime_resources::{
    MAX_ARCHIVE, ResourceManifest, Temporary, cancelled, check_archive_cancellable, check_path,
    hash_valid,
};
use anyhow::{Context, Result, ensure};
use od_platform::download::{HttpsDownloader, ResourceDownloader, validate_https_url};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
use uuid::Uuid;

pub struct ResourceDownloadSource {
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

impl ResourceDownloadSource {
    pub fn validate(&self) -> Result<()> {
        validate_https_url(&self.url)?;
        ensure!(
            self.bytes > 0 && self.bytes <= MAX_ARCHIVE,
            "预期 ZIP 大小须为 1..536870912 字节"
        );
        ensure!(hash_valid(&self.sha256), "必须提供 64 位小写 ZIP SHA-256");
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct ResourceDownloadProgress {
    pub phase: &'static str,
    pub completed_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct DownloadedResources {
    pub id: String,
    pub archive: PathBuf,
    pub archive_bytes: u64,
    pub sha256: String,
    pub manifest: ResourceManifest,
    pub reused: bool,
}

pub fn download(
    source: &ResourceDownloadSource,
    output: &Path,
    cancel: &AtomicBool,
    observer: impl FnMut(ResourceDownloadProgress),
) -> Result<DownloadedResources> {
    download_with(source, output, &HttpsDownloader, cancel, observer)
}

/// 只有 CLI 提供来源；桌面宿主不接受任意 URL，测试可以替换传输。
pub fn download_with(
    source: &ResourceDownloadSource,
    output: &Path,
    downloader: &impl ResourceDownloader,
    cancel: &AtomicBool,
    mut observer: impl FnMut(ResourceDownloadProgress),
) -> Result<DownloadedResources> {
    source.validate()?;
    cancelled(cancel)?;
    let output = std::path::absolute(output)?;
    check_path(&output)?;
    ensure!(
        output
            .extension()
            .is_some_and(|value| value.eq_ignore_ascii_case("zip")),
        "输出文件须使用 .zip 扩展名"
    );
    let mut progress = |phase, bytes| {
        observer(ResourceDownloadProgress {
            phase,
            completed_bytes: bytes,
            total_bytes: source.bytes,
        })
    };
    progress("checking", 0);
    cancelled(cancel)?;
    if output.try_exists()? {
        let file = OpenOptions::new().read(true).share_mode(1).open(&output)?;
        ensure!(
            file.metadata()?.is_file() && file.metadata()?.len() == source.bytes,
            "已有输出大小不符，原文件保留"
        );
        let (id, manifest) = check_archive_cancellable(&output, &source.sha256, cancel)
            .context("已有输出未通过完整检查，原文件保留")?;
        progress("ready", source.bytes);
        return Ok(DownloadedResources {
            id,
            archive: output,
            archive_bytes: source.bytes,
            sha256: source.sha256.clone(),
            manifest,
            reused: true,
        });
    }
    let parent = output.parent().context("无法定位输出目录")?;
    fs::create_dir_all(parent)?;
    let work = Temporary::directory(parent.join(format!(".download-{}", Uuid::new_v4())))?;
    let partial = work.0.join("archive.partial");
    progress("connecting", 0);
    cancelled(cancel)?;
    let mut input = downloader
        .open_cancellable(&source.url, cancel)
        .context("无法下载运行资源包")?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&partial)?;
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        cancelled(cancel)?;
        let length = input.read(&mut buffer)?;
        cancelled(cancel)?;
        if length == 0 {
            break;
        }
        bytes += length as u64;
        ensure!(bytes <= source.bytes, "下载超过预期 ZIP 大小");
        file.write_all(&buffer[..length])?;
        digest.update(&buffer[..length]);
        progress("downloading", bytes);
    }
    ensure!(
        bytes == source.bytes && hex::encode(digest.finalize()) == source.sha256,
        "下载大小或 ZIP SHA-256 不匹配"
    );
    file.sync_all()?;
    drop(file);
    drop(input);
    // 完整检查与最终发布期间禁止其他进程改写或删除临时包。
    let locked = OpenOptions::new().read(true).share_mode(1).open(&partial)?;
    progress("validating", bytes);
    let (id, manifest) = check_archive_cancellable(&partial, &source.sha256, cancel)?;
    progress("publishing", bytes);
    cancelled(cancel)?;
    check_path(&output)?;
    fs::hard_link(&partial, &output).context("无法保存已校验 ZIP；已有输出不会被覆盖")?;
    drop(locked);
    progress("ready", bytes);
    Ok(DownloadedResources {
        id,
        archive: output,
        archive_bytes: source.bytes,
        sha256: source.sha256.clone(),
        manifest,
        reused: false,
    })
}
