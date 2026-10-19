//! 固定数据目录内的只读素材列表与逐张完整校验。
use crate::{
    CaptureMethod, Screenshot, ScreenshotQuality,
    screen_samples::{MAX_PNG_BYTES, ScreenSampleKind, ScreenSamplePreview, ScreenSampleSource},
};
use anyhow::{Context, Result, ensure};
use base64::Engine;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use ts_rs::TS;
use uuid::Uuid;

pub const PAGE_SIZE: usize = 20;
pub const MAX_ENTRIES: usize = 4096;
const MAX_METADATA_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ScreenSampleQuery {
    pub kind: Option<ScreenSampleKind>,
    pub text: String,
    pub before: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ScreenSampleItem {
    pub key: String,
    pub metadata_sha256: String,
    pub id: Uuid,
    pub kind: ScreenSampleKind,
    pub note: String,
    #[ts(type = "string")]
    pub captured_at: DateTime<Utc>,
    pub capture_method: CaptureMethod,
    pub bytes: u32,
    pub sha256: String,
    pub near_black: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ScreenSamplePage {
    pub items: Vec<ScreenSampleItem>,
    pub matching: u32,
    pub unreadable: u32,
    pub next: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ScreenSampleView {
    pub item: ScreenSampleItem,
    pub preview: ScreenSamplePreview,
    pub image_path: String,
    pub metadata_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    schema_version: u32,
    scope: String,
    id: Uuid,
    source: ScreenSampleSource,
    annotation: Annotation,
    accepted: bool,
    image: Image,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Annotation {
    kind: ScreenSampleKind,
    note: String,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Image {
    file: String,
    bytes: u32,
    sha256: String,
    width: i32,
    height: i32,
    captured_at: DateTime<Utc>,
    elapsed_ms: u64,
    color_order: String,
    quality: ScreenshotQuality,
}

fn directory(data: &Path) -> Result<PathBuf> {
    ensure!(data.is_absolute(), "素材数据目录必须为绝对路径");
    let directory = data.join("diagnostics/screens");
    crate::config::reject_links(&directory)?;
    Ok(directory)
}

fn validate_key(key: &str) -> Result<()> {
    ensure!(
        key.len() == 52 && key.is_ascii(),
        "素材标识无效，请刷新列表"
    );
    let date = NaiveDateTime::parse_from_str(&key[..15], "%Y%m%d-%H%M%S")?;
    let id = Uuid::parse_str(&key[16..])?;
    ensure!(
        format!("{}-{id}", date.format("%Y%m%d-%H%M%S")) == key,
        "素材标识无效"
    );
    Ok(())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn regular_file(path: &Path, limit: usize) -> Result<u64> {
    crate::config::reject_links(path)?;
    let info = fs::symlink_metadata(path)?;
    ensure!(
        info.is_file() && info.len() <= limit as u64,
        "素材文件类型或大小无效"
    );
    Ok(info.len())
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    regular_file(path, limit)?;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "素材文件超过大小限制");
    crate::config::reject_links(path)?;
    Ok(bytes)
}

fn read_metadata(directory: &Path, key: &str) -> Result<(Metadata, ScreenSampleItem)> {
    validate_key(key)?;
    let folder = directory.join(key);
    let bytes = read_bounded(&folder.join("metadata.json"), MAX_METADATA_BYTES)?;
    let metadata: Metadata = serde_json::from_slice(&bytes)?;
    let image = &metadata.image;
    ensure!(
        metadata.schema_version == 1
            && metadata.scope == "user_reviewed_screen_sample"
            && !metadata.accepted
            && metadata.annotation.source == "user",
        "素材格式不兼容"
    );
    ensure!(
        key == format!(
            "{}-{}",
            image.captured_at.format("%Y%m%d-%H%M%S"),
            metadata.id
        ),
        "素材标识与元信息不匹配"
    );
    ensure!(
        image.file == "image.png"
            && image.width == 1920
            && image.height == 1080
            && image.bytes > 0
            && image.bytes as usize <= MAX_PNG_BYTES
            && valid_hash(&image.sha256),
        "素材图片元信息无效"
    );
    ensure!(
        metadata.annotation.note.chars().count() <= 240
            && !metadata.annotation.note.chars().any(char::is_control)
            && image.color_order.len() <= 128
            && metadata.source.hwnd.len() <= 64
            && !metadata.source.hwnd.chars().any(char::is_control)
            && image.quality.mean_luma.is_finite()
            && (0.0..=255.0).contains(&image.quality.mean_luma)
            && image.quality.dark_pixel_ratio.is_finite()
            && (0.0..=1.0).contains(&image.quality.dark_pixel_ratio),
        "素材标注或截图信息无效"
    );
    ensure!(
        regular_file(&folder.join("image.png"), MAX_PNG_BYTES)? == u64::from(image.bytes),
        "素材图片大小已变化"
    );
    let item = ScreenSampleItem {
        key: key.into(),
        metadata_sha256: hex::encode(Sha256::digest(&bytes)),
        id: metadata.id,
        kind: metadata.annotation.kind,
        note: metadata.annotation.note.clone(),
        captured_at: image.captured_at,
        capture_method: metadata.source.capture_method,
        bytes: image.bytes,
        sha256: image.sha256.clone(),
        near_black: image.quality.near_black,
    };
    Ok((metadata, item))
}

pub fn list(data: &Path, query: &ScreenSampleQuery) -> Result<ScreenSamplePage> {
    ensure!(
        query.text.chars().count() <= 240 && !query.text.chars().any(char::is_control),
        "筛选备注最多 240 字且不能包含控制字符"
    );
    if let Some(key) = &query.before {
        validate_key(key)?;
    }
    let directory = directory(data)?;
    let mut page = ScreenSamplePage {
        items: Vec::new(),
        matching: 0,
        unreadable: 0,
        next: None,
    };
    if !directory.try_exists()? {
        return Ok(page);
    }
    let text = query.text.trim().to_lowercase();
    for (index, entry) in fs::read_dir(&directory)?.enumerate() {
        ensure!(
            index < MAX_ENTRIES,
            "素材目录超过 4096 项，请先手动归档部分文件夹后重试"
        );
        let entry = entry?;
        let name = entry.file_name();
        let Some(key) = name.to_str() else {
            page.unreadable += 1;
            continue;
        };
        if key.starts_with('.') {
            continue;
        }
        match read_metadata(&directory, key) {
            Ok((_, item)) => {
                if query.kind.is_some_and(|kind| kind != item.kind)
                    || !item.note.to_lowercase().contains(&text)
                {
                    continue;
                }
                page.matching += 1;
                if query
                    .before
                    .as_ref()
                    .is_none_or(|before| item.key < *before)
                {
                    page.items.push(item);
                }
            }
            Err(_) => page.unreadable += 1,
        }
    }
    page.items.sort_by(|a, b| b.key.cmp(&a.key));
    if page.items.len() > PAGE_SIZE {
        page.items.truncate(PAGE_SIZE);
        page.next = page.items.last().map(|item| item.key.clone());
    }
    Ok(page)
}

pub fn open(data: &Path, key: &str, metadata_sha256: &str) -> Result<ScreenSampleView> {
    ensure!(valid_hash(metadata_sha256), "素材指纹无效，请刷新列表");
    let directory = directory(data)?;
    let (metadata, item) = read_metadata(&directory, key).context("素材无法读取，请刷新列表")?;
    ensure!(
        item.metadata_sha256 == metadata_sha256,
        "素材元信息已变化，请刷新列表后重新打开"
    );
    let folder = directory.join(key);
    let image_path = folder.join("image.png");
    let bytes = read_bounded(&image_path, MAX_PNG_BYTES)?;
    ensure!(
        bytes.len() == item.bytes as usize && hex::encode(Sha256::digest(&bytes)) == item.sha256,
        "素材图片校验失败，请检查原文件并刷新列表"
    );
    crate::screen_samples::validate_png(&bytes)?;
    // 解码期间文件可能变化；回看只展示本次读到且校验通过的完整快照。
    let (_, checked) = read_metadata(&directory, key)?;
    ensure!(
        checked.metadata_sha256 == item.metadata_sha256,
        "素材元信息已变化，请刷新列表"
    );
    let preview = ScreenSamplePreview {
        id: metadata.id,
        source: metadata.source,
        sha256: item.sha256.clone(),
        bytes: item.bytes,
        screenshot: Screenshot {
            png_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            width: metadata.image.width,
            height: metadata.image.height,
            color_order: metadata.image.color_order,
            capture_method: Some(item.capture_method),
            quality: metadata.image.quality,
            captured_at: item.captured_at,
            elapsed_ms: metadata.image.elapsed_ms,
        },
    };
    Ok(ScreenSampleView {
        item,
        preview,
        image_path: image_path.to_string_lossy().into_owned(),
        metadata_path: folder.join("metadata.json").to_string_lossy().into_owned(),
    })
}
