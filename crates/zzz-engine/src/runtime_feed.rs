use anyhow::{Context, Result, ensure};
use od_core::runtime_feed::{
    RuntimeFeedCatalog, RuntimeFeedSource, RuntimeRelease, RuntimeReleaseOption,
    RuntimeReleaseSource,
};
use od_platform::download::{ConfiguredHttpsDownloader, ResourceDownloader, validate_https_url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{BufRead, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const MAX_FEED_BYTES: usize = 512 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseFeed {
    pub schema_version: u32,
    pub releases: Vec<RuntimeReleaseSource>,
}

pub fn compatible(release: &RuntimeRelease) -> bool {
    release.engine_version == env!("CARGO_PKG_VERSION")
        && release.sdk_version == maa_adapter::SDK_VERSION
        && release.binding_version == maa_adapter::BINDING_VERSION
        && crate::runtime_resources::supports_protocol(
            release.resource_schema,
            release.protocol_version,
        )
}
pub fn validate_source(source: &RuntimeFeedSource) -> Result<()> {
    validate_https_url(&source.url)?;
    ensure!(
        crate::runtime_resources::hash_valid(&source.sha256),
        "发布清单须提供 64 位小写 SHA-256"
    );
    Ok(())
}
pub fn parse(
    bytes: &[u8],
    sha256: &str,
) -> Result<(RuntimeFeedCatalog, Vec<RuntimeReleaseSource>)> {
    ensure!(
        bytes.len() <= MAX_FEED_BYTES && crate::runtime_resources::hash_valid(sha256),
        "发布清单大小或校验值无效"
    );
    ensure!(
        hex::encode(Sha256::digest(bytes)) == sha256,
        "发布清单 SHA-256 不匹配"
    );
    let feed: ReleaseFeed =
        serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("发布清单 JSON 无效"))?;
    ensure!(
        feed.schema_version == 1 && !feed.releases.is_empty() && feed.releases.len() <= 64,
        "发布清单版本或条目数量无效"
    );
    let mut seen = BTreeSet::new();
    let mut versions = BTreeSet::new();
    let mut options = Vec::new();
    for entry in &feed.releases {
        let release = &entry.release;
        validate_https_url(&entry.url)?;
        ensure!(
            !release.version.is_empty()
                && release.version.len() <= 64
                && release
                    .version
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b)),
            "发布资源版本无效"
        );
        ensure!(
            release.bytes > 0
                && release.bytes <= 536870912
                && crate::runtime_resources::hash_valid(&release.sha256)
                && seen.insert(&release.sha256)
                && versions.insert(&release.version),
            "发布条目大小、哈希或重复版本无效"
        );
        for value in [
            &release.engine_version,
            &release.sdk_version,
            &release.binding_version,
        ] {
            ensure!(
                !value.is_empty()
                    && value.len() <= 64
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b)),
                "发布兼容版本无效"
            );
        }
        let compatible = compatible(release);
        options.push(RuntimeReleaseOption {
            release: release.clone(),
            compatible,
            message: if compatible {
                "与当前引擎兼容"
            } else {
                "引擎、SDK、绑定或资源协议不兼容"
            }
            .into(),
        });
    }
    Ok((
        RuntimeFeedCatalog {
            id: sha256.into(),
            releases: options,
        },
        feed.releases,
    ))
}
pub fn discover_with(
    source: &RuntimeFeedSource,
    downloader: &impl ResourceDownloader,
    cancel: &AtomicBool,
) -> Result<(RuntimeFeedCatalog, Vec<RuntimeReleaseSource>)> {
    validate_source(source)?;
    crate::runtime_resources::cancelled(cancel)?;
    let mut stream = downloader.open_cancellable(&source.url, cancel)?;
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        crate::runtime_resources::cancelled(cancel)?;
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        ensure!(bytes.len() + n <= MAX_FEED_BYTES, "发布清单超过 512 KiB");
        bytes.extend_from_slice(&chunk[..n]);
    }
    crate::runtime_resources::cancelled(cancel)?;
    let result = parse(&bytes, &source.sha256)?;
    crate::runtime_resources::cancelled(cancel)?;
    Ok(result)
}
pub fn discover(
    source: &RuntimeFeedSource,
    proxy: od_core::download_proxy::DownloadProxy,
    cancel: &AtomicBool,
) -> Result<(RuntimeFeedCatalog, Vec<RuntimeReleaseSource>)> {
    discover_with(source, &ConfiguredHttpsDownloader::new(proxy)?, cancel)
}
pub fn matches_manifest(
    release: &RuntimeRelease,
    manifest: &crate::runtime_resources::ResourceManifest,
) -> bool {
    release.version == manifest.version
        && release.engine_version == manifest.engine_version
        && release.sdk_version == manifest.sdk_version
        && release.binding_version == manifest.binding_version
        && release.protocol_version == manifest.protocol_version
        && release.resource_schema == manifest.schema_version
}

pub fn write_feed(
    archive: &std::path::Path,
    sha256: &str,
    url: String,
    output: &std::path::Path,
) -> Result<serde_json::Value> {
    use std::os::windows::fs::OpenOptionsExt;
    validate_https_url(&url)?;
    let archive = std::path::absolute(archive)?;
    crate::runtime_resources::check_path(&archive)?;
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&archive)?;
    let manifest = crate::runtime_resources::check_archive(&archive, sha256)?;
    let release = RuntimeRelease {
        version: manifest.version,
        engine_version: manifest.engine_version,
        sdk_version: manifest.sdk_version,
        binding_version: manifest.binding_version,
        protocol_version: manifest.protocol_version,
        resource_schema: manifest.schema_version,
        bytes: locked.metadata()?.len(),
        sha256: sha256.into(),
    };
    let bytes = serde_json::to_vec_pretty(&ReleaseFeed {
        schema_version: 1,
        releases: vec![RuntimeReleaseSource { release, url }],
    })?;
    let hash = hex::encode(Sha256::digest(&bytes));
    parse(&bytes, &hash)?;
    let output = std::path::absolute(output)?;
    crate::runtime_resources::check_path(&output)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(serde_json::json!({"output":output,"sha256":hash,"bytes":bytes.len()}))
}

pub fn managed() -> Result<()> {
    use od_core::runtime_feed::{ManagedFeedRequest, RuntimeFeedEvent};
    let mut input = std::io::BufReader::new(std::io::stdin());
    let mut line = Vec::new();
    input.by_ref().take(16385).read_until(b'\n', &mut line)?;
    ensure!(
        line.len() <= 16384 && line.last() == Some(&b'\n'),
        "发布清单请求无效"
    );
    let request: ManagedFeedRequest =
        serde_json::from_slice(&line).map_err(|_| anyhow::anyhow!("发布清单请求无效"))?;
    let cancel = Arc::new(AtomicBool::new(false));
    let disconnected = cancel.clone();
    std::thread::spawn(move || {
        let _ = input.read(&mut [0]);
        disconnected.store(true, Ordering::Release);
    });
    let event = match discover(&request.source, request.proxy, &cancel) {
        Ok((catalog, sources)) => RuntimeFeedEvent::Completed { catalog, sources },
        Err(_) if cancel.load(Ordering::Acquire) => RuntimeFeedEvent::Cancelled,
        Err(error) => RuntimeFeedEvent::Failed(format!("{error:#}")),
    };
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, &event)?;
    writeln!(out)?;
    out.flush().context("无法写入清单结果")
}
