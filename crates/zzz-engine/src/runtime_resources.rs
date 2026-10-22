use anyhow::{Context, Result, ensure};
pub use od_core::runtime_resources::ResourceSelection;
use od_core::runtime_resources::{
    InstalledResources, RuntimeResourceCatalog, RuntimeResourceRequest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const MANIFEST: &str = "resource-manifest.json";
const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_FILE: u64 = 128 * 1024 * 1024;
pub(crate) const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
// 首批只收集当前具有执行器的完整资源，其他生成目录不进入运行包。
const FILES: &[&str] = &[
    "catalog/applications.json",
    "generated/mail/image/mail/menu/back.png",
    "generated/mail/image/mail/menu/dynamic_wallpaper.png",
    "generated/mail/image/mail/normal_world/function_menu.png",
    "generated/mail/image/mail/normal_world/preset_config.png",
    "generated/mail/mail-manifest.json",
    "generated/mail/model/ocr/ppocrv5/det.onnx",
    "generated/mail/model/ocr/ppocrv5/keys.txt",
    "generated/mail/model/ocr/ppocrv5/rec.onnx",
    "generated/mail/pipeline/mail-areas.json",
    "generated/mail/pipeline/mail.json",
    "generated/map-preview/image/map/back.png",
    "generated/map-preview/image/map/function_menu.png",
    "generated/map-preview/image/map/preset_config.png",
    "generated/map-preview/map-manifest.json",
    "generated/map-preview/model/ocr/ppocrv5/det.onnx",
    "generated/map-preview/model/ocr/ppocrv5/keys.txt",
    "generated/map-preview/model/ocr/ppocrv5/rec.onnx",
    "generated/map-preview/pipeline/map.json",
    "pipeline/health.json",
];
const COFFEE_FILES: &[&str] = &[
    "generated/coffee-run/coffee-run-manifest.json",
    "generated/coffee-run/pipeline/coffee-run.json",
    "generated/coffee-run/image/coffee/world_function.png",
    "generated/coffee-run/model/ocr/ppocrv5/det.onnx",
    "generated/coffee-run/model/ocr/ppocrv5/rec.onnx",
    "generated/coffee-run/model/ocr/ppocrv5/keys.txt",
];
const DAILY_FILES: &[&str] = &[
    "generated/basic-daily/basic-daily-manifest.json",
    "generated/basic-daily/pipeline/basic-daily.json",
    "generated/basic-daily/image/daily/back.png",
    "generated/basic-daily/image/daily/close.png",
    "generated/basic-daily/image/daily/compendium.png",
    "generated/basic-daily/image/daily/completed.png",
    "generated/basic-daily/image/daily/menu_wallpaper.png",
    "generated/basic-daily/image/daily/world_function.png",
    "generated/basic-daily/image/daily/world_preset.png",
    "generated/basic-daily/model/ocr/ppocrv5/det.onnx",
    "generated/basic-daily/model/ocr/ppocrv5/rec.onnx",
    "generated/basic-daily/model/ocr/ppocrv5/keys.txt",
];
fn resource_files(schema: u32) -> Vec<&'static str> {
    FILES
        .iter()
        .chain(if schema >= 2 { COFFEE_FILES } else { &[] })
        .chain(if schema >= 3 { DAILY_FILES } else { &[] })
        .copied()
        .collect()
}

fn daily_extension_path(name: &str) -> bool {
    name.strip_prefix("generated/basic-daily/")
        .is_some_and(|relative| {
            od_core::random_play::templates().contains_key(relative)
                || intel_extension_relative(relative)
        })
}
fn intel_extension_relative(relative: &str) -> bool {
    od_core::intel_board::templates().contains_key(relative)
        || od_core::intel_board::DETECTOR_FILES.contains(&relative)
}
fn intel_extension_path(name: &str) -> bool {
    name.strip_prefix("generated/basic-daily/")
        .is_some_and(intel_extension_relative)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceManifest {
    pub schema_version: u32,
    pub version: String,
    pub engine_version: String,
    pub sdk_version: String,
    pub binding_version: String,
    pub protocol_version: u32,
    pub files: Vec<ResourceFile>,
}

#[derive(Debug, Serialize)]
pub struct ResourceStatus {
    pub selection: ResourceSelection,
    pub directory: PathBuf,
    pub manifest: Option<ResourceManifest>,
}

#[derive(Debug, Serialize)]
pub struct PackedResources {
    pub id: String,
    pub archive: PathBuf,
    pub archive_bytes: u64,
    pub sha256: String,
    pub manifest: ResourceManifest,
}

pub(crate) fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(crate) fn check_path(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "资源路径必须是绝对路径");
    od_core::config::reject_links(path)?;
    for ancestor in path.ancestors().skip(1) {
        match fs::metadata(ancestor) {
            Ok(metadata) => ensure!(metadata.is_dir(), "资源路径的上级不是目录"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    check_path(path)?;
    let file = File::open(path).with_context(|| format!("无法读取 {}", path.display()))?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= limit,
        "资源文件过大或类型不符"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "资源文件过大");
    Ok(bytes)
}

pub(crate) fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Acquire), "运行资源下载已取消");
    Ok(())
}

fn copy_hash(input: impl Read, output: impl Write, limit: u64) -> Result<(u64, String)> {
    copy_hash_cancellable(input, output, limit, &AtomicBool::new(false))
}

fn copy_hash_cancellable(
    mut input: impl Read,
    mut output: impl Write,
    limit: u64,
    cancel: &AtomicBool,
) -> Result<(u64, String)> {
    let mut digest = Sha256::new();
    let mut count = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        cancelled(cancel)?;
        let length = input.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        count += length as u64;
        ensure!(count <= limit, "资源文件超过允许长度");
        digest.update(&buffer[..length]);
        output.write_all(&buffer[..length])?;
    }
    Ok((count, hex::encode(digest.finalize())))
}

pub fn supports_protocol(schema: u32, protocol: u32) -> bool {
    let minimum = match schema {
        1 => 15,
        2 => 16,
        3 => 33,
        4 => 34,
        _ => return false,
    };
    (minimum..=od_protocol::PROTOCOL_VERSION).contains(&protocol)
}

fn safe_temple_path(name: &str) -> bool {
    name.starts_with("generated/suibian-temple/")
        && !name.contains([':', '\\'])
        && Path::new(name)
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
}
fn safe_combat_path(name: &str) -> bool {
    name.starts_with("generated/combat/")
        && !name.contains([':', '\\'])
        && Path::new(name)
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
}

fn validate_manifest(manifest: &ResourceManifest) -> Result<()> {
    ensure!(
        matches!(manifest.schema_version, 1..=4)
            && manifest.engine_version == env!("CARGO_PKG_VERSION")
            && manifest.sdk_version == maa_adapter::SDK_VERSION
            && manifest.binding_version == maa_adapter::BINDING_VERSION
            && supports_protocol(manifest.schema_version, manifest.protocol_version),
        "运行资源包与当前引擎、SDK、绑定或协议不兼容"
    );
    ensure!(
        !manifest.version.is_empty()
            && manifest.version.len() <= 64
            && manifest
                .version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._".contains(&byte)),
        "内容版本须为 1..64 位字母、数字或 -._"
    );
    let files = resource_files(manifest.schema_version);
    let mut paths = BTreeSet::new();
    let mut total = 0_u64;
    for file in &manifest.files {
        ensure!(
            (files.contains(&file.path.as_str())
                || (manifest.schema_version >= 3
                    && manifest.protocol_version >= 38
                    && (!intel_extension_path(&file.path) || manifest.protocol_version >= 41)
                    && daily_extension_path(&file.path))
                || (manifest.schema_version >= 3
                    && manifest.protocol_version >= 44
                    && safe_temple_path(&file.path))
                || (manifest.schema_version == 4 && safe_combat_path(&file.path)))
                && paths.insert(file.path.as_str()),
            "资源文件未登记或重复"
        );
        ensure!(
            file.bytes <= MAX_FILE && hash_valid(&file.sha256),
            "资源长度或哈希无效"
        );
        total += file.bytes;
    }
    ensure!(
        files.iter().all(|name| paths.contains(name))
            && (manifest.schema_version != 4
                || paths.contains("generated/combat/combat-manifest.json"))
            && manifest.files.len() <= 4096
            && total <= MAX_ARCHIVE,
        "运行资源包不完整或过大"
    );
    Ok(())
}

fn validate_semantics(root: &Path) -> Result<()> {
    if root.join("generated/suibian-temple").try_exists()? {
        maa_adapter::suibian_temple::validate_full_temple_bundle(
            &root.join("generated/suibian-temple"),
        )?;
    }
    if root.join("generated/combat").try_exists()? {
        maa_adapter::combat_vision::CombatBundle::load(&root.join("generated/combat"))?;
    }
    maa_adapter::validate_mail_bundle(&root.join("generated/mail"))?;
    maa_adapter::validate_map_bundle(&root.join("generated/map-preview"))?;
    if root.join("generated/coffee-run").try_exists()? {
        maa_adapter::validate_coffee_run_bundle(&root.join("generated/coffee-run"))?;
    }
    if root.join("generated/basic-daily").try_exists()? {
        maa_adapter::validate_basic_daily_bundle(&root.join("generated/basic-daily"))?;
    }
    for (name, expected) in [
        (
            "pipeline/health.json",
            include_bytes!("../../../resources/pipeline/health.json").as_slice(),
        ),
        (
            "catalog/applications.json",
            include_bytes!("../../../resources/catalog/applications.json").as_slice(),
        ),
    ] {
        ensure!(
            read_limited(&root.join(name), MAX_MANIFEST)? == expected,
            "基础运行资源与引擎不匹配：{name}"
        );
    }
    Ok(())
}

fn read_manifest(root: &Path, expected_id: Option<&str>) -> Result<ResourceManifest> {
    check_path(root)?;
    let bytes = read_limited(&root.join(MANIFEST), MAX_MANIFEST)?;
    if let Some(id) = expected_id {
        ensure!(
            hash_valid(id) && hex::encode(Sha256::digest(&bytes)) == id,
            "运行资源清单哈希不匹配"
        );
    }
    let manifest: ResourceManifest = serde_json::from_slice(&bytes)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn validate_directory(root: &Path, expected_id: Option<&str>) -> Result<ResourceManifest> {
    validate_directory_cancellable(root, expected_id, &AtomicBool::new(false))
}

fn validate_directory_cancellable(
    root: &Path,
    expected_id: Option<&str>,
    cancel: &AtomicBool,
) -> Result<ResourceManifest> {
    cancelled(cancel)?;
    let manifest = read_manifest(root, expected_id)?;
    let files: Vec<_> = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    for entry in walkdir::WalkDir::new(root).min_depth(1) {
        cancelled(cancel)?;
        let entry = entry?;
        check_path(entry.path())?;
        let name = entry
            .path()
            .strip_prefix(root)?
            .to_str()
            .context("资源路径不是 UTF-8")?
            .replace('\\', "/");
        if entry.file_type().is_dir() {
            ensure!(
                files
                    .iter()
                    .any(|file| file.starts_with(&format!("{name}/"))),
                "资源包包含多余目录"
            );
        } else {
            ensure!(
                entry.file_type().is_file() && (name == MANIFEST || files.contains(&name.as_str())),
                "资源包包含多余文件"
            );
        }
    }
    for file in &manifest.files {
        let input = File::open(root.join(&file.path))?;
        let (bytes, hash) = copy_hash_cancellable(input, std::io::sink(), file.bytes, cancel)?;
        ensure!(
            bytes == file.bytes && hash == file.sha256,
            "运行资源校验失败：{}",
            file.path
        );
    }
    validate_semantics(root)?;
    cancelled(cancel)?;
    Ok(manifest)
}

// 只在本次创建的随机临时目录中收尾；强制终止留下的目录不会被后续采用。
pub(crate) struct Temporary(pub(crate) PathBuf);

impl Temporary {
    pub(crate) fn directory(path: PathBuf) -> Result<Self> {
        check_path(&path)?;
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn file(path: PathBuf) -> Result<(Self, File)> {
        check_path(&path)?;
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)?;
        Ok((Self(path), file))
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.0) {
            if metadata.is_dir() && check_path(&self.0).is_ok() {
                let _ = fs::remove_dir_all(&self.0);
            } else if metadata.is_file() {
                let _ = fs::remove_file(&self.0);
            }
        }
    }
}

pub fn pack(source: &Path, version: &str, output: &Path) -> Result<PackedResources> {
    let source = std::path::absolute(source)?;
    let output = std::path::absolute(output)?;
    check_path(&source)?;
    check_path(&output)?;
    ensure!(!output.starts_with(&source), "资源包输出必须位于来源目录外");
    ensure!(!output.try_exists()?, "输出文件已存在，不能覆盖");
    let mut manifest = ResourceManifest {
        schema_version: if source.join("generated/combat").try_exists()? {
            4
        } else if source.join("generated/basic-daily").try_exists()? {
            3
        } else if source.join("generated/coffee-run").try_exists()? {
            2
        } else {
            1
        },
        version: version.into(),
        engine_version: env!("CARGO_PKG_VERSION").into(),
        sdk_version: maa_adapter::SDK_VERSION.into(),
        binding_version: maa_adapter::BINDING_VERSION.into(),
        protocol_version: od_protocol::PROTOCOL_VERSION,
        files: Vec::new(),
    };
    let mut names: Vec<String> = resource_files(manifest.schema_version)
        .into_iter()
        .map(str::to_string)
        .collect();
    if manifest.schema_version >= 3 {
        let daily =
            maa_adapter::validate_basic_daily_bundle(&source.join("generated/basic-daily"))?;
        let daily_manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(daily.join("basic-daily-manifest.json"))?)?;
        for file in daily_manifest["files"]
            .as_array()
            .context("日常清单缺少文件列表")?
        {
            let name = format!(
                "generated/basic-daily/{}",
                file["path"].as_str().context("日常文件名无效")?
            );
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    if manifest.schema_version == 4 {
        maa_adapter::combat_vision::CombatBundle::load(&source.join("generated/combat"))?;
        let mut combat = Vec::new();
        for entry in walkdir::WalkDir::new(source.join("generated/combat")).min_depth(1) {
            let entry = entry?;
            check_path(entry.path())?;
            if entry.file_type().is_file() {
                combat.push(
                    entry
                        .path()
                        .strip_prefix(&source)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
        combat.sort();
        names.extend(combat);
    }
    if source.join("generated/suibian-temple").try_exists()? {
        let temple = source.join("generated/suibian-temple");
        maa_adapter::suibian_temple::validate_full_temple_bundle(&temple)?;
        for entry in walkdir::WalkDir::new(temple).min_depth(1) {
            let entry = entry?;
            check_path(entry.path())?;
            if entry.file_type().is_file() {
                names.push(
                    entry
                        .path()
                        .strip_prefix(&source)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    for name in names {
        let path = source.join(&name);
        check_path(&path)?;
        let (bytes, sha256) = copy_hash(File::open(path)?, std::io::sink(), MAX_FILE)?;
        manifest.files.push(ResourceFile {
            path: name,
            bytes,
            sha256,
        });
    }
    validate_manifest(&manifest)?;
    validate_semantics(&source)?;
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    let id = hex::encode(Sha256::digest(&bytes));
    let parent = output.parent().context("无法定位输出目录")?;
    fs::create_dir_all(parent)?;
    let (temporary, file) = Temporary::file(parent.join(format!(".pack-{}.tmp", Uuid::new_v4())))?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file(MANIFEST, options)?;
    writer.write_all(&bytes)?;
    for file in &manifest.files {
        check_path(&source.join(&file.path))?;
        writer.start_file(&file.path, options)?;
        let (bytes, hash) = copy_hash(
            File::open(source.join(&file.path))?,
            &mut writer,
            file.bytes,
        )?;
        ensure!(
            bytes == file.bytes && hash == file.sha256,
            "打包期间来源发生变化"
        );
    }
    let mut archive = writer.finish()?;
    archive.sync_all()?;
    archive.rewind()?;
    let (archive_bytes, sha256) = copy_hash(&mut archive, std::io::sink(), MAX_ARCHIVE)?;
    drop(archive);
    // hard_link 在目标存在时失败，避免并发打包覆盖用户文件。
    fs::hard_link(&temporary.0, &output)?;
    Ok(PackedResources {
        id,
        archive: output,
        archive_bytes,
        sha256,
        manifest,
    })
}

fn extract(
    archive: &Path,
    expected_hash: &str,
    staging: &Path,
    cancel: &AtomicBool,
) -> Result<(String, ResourceManifest)> {
    cancelled(cancel)?;
    ensure!(hash_valid(expected_hash), "必须提供小写的 ZIP SHA-256");
    check_path(archive)?;
    // 校验与解压使用同一只读句柄，期间拒绝其他进程改写 ZIP。
    use std::os::windows::fs::OpenOptionsExt;
    let mut file = OpenOptions::new().read(true).share_mode(1).open(archive)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= MAX_ARCHIVE,
        "ZIP 过大或类型无效"
    );
    let (_, hash) = copy_hash_cancellable(&mut file, std::io::sink(), MAX_ARCHIVE, cancel)?;
    ensure!(hash == expected_hash, "ZIP SHA-256 不匹配");
    file.rewind()?;
    let mut zip = ZipArchive::new(file)?;
    ensure!(
        (FILES.len() + 1..=4097).contains(&zip.len()),
        "ZIP 文件数量不匹配"
    );
    let mut names = BTreeSet::new();
    for index in 0..zip.len() {
        cancelled(cancel)?;
        let entry = zip.by_index(index)?;
        ensure!(
            !entry.is_dir()
                && !entry.is_symlink()
                && (entry.name() == MANIFEST
                    || FILES.contains(&entry.name())
                    || COFFEE_FILES.contains(&entry.name())
                    || DAILY_FILES.contains(&entry.name())
                    || daily_extension_path(entry.name())
                    || safe_temple_path(entry.name())
                    || safe_combat_path(entry.name()))
                && names.insert(entry.name().to_owned()),
            "ZIP 包含多余、重复或非法条目"
        );
    }
    let mut bytes = Vec::new();
    zip.by_name(MANIFEST)?
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_MANIFEST, "资源清单过大");
    let manifest: ResourceManifest = serde_json::from_slice(&bytes)?;
    validate_manifest(&manifest)?;
    ensure!(
        zip.len() == manifest.files.len() + 1
            && manifest.files.iter().all(|file| names.contains(&file.path)),
        "ZIP 文件数量与资源格式不匹配"
    );
    let id = hex::encode(Sha256::digest(&bytes));
    let mut manifest_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(staging.join(MANIFEST))?;
    manifest_file.write_all(&bytes)?;
    manifest_file.sync_all()?;
    drop(manifest_file);
    for file in &manifest.files {
        cancelled(cancel)?;
        let mut entry = zip.by_name(&file.path)?;
        ensure!(entry.size() == file.bytes, "ZIP 文件声明长度不匹配");
        let path = staging.join(&file.path);
        fs::create_dir_all(path.parent().context("资源父目录缺失")?)?;
        let mut output = OpenOptions::new().create_new(true).write(true).open(path)?;
        let (bytes, hash) = copy_hash_cancellable(&mut entry, &mut output, file.bytes, cancel)?;
        output.sync_all()?;
        ensure!(
            bytes == file.bytes && hash == file.sha256,
            "ZIP 资源校验失败：{}",
            file.path
        );
    }
    validate_directory_cancellable(staging, Some(&id), cancel)?;
    Ok((id, manifest))
}

pub fn check_archive(archive: &Path, sha256: &str) -> Result<ResourceManifest> {
    Ok(check_archive_cancellable(archive, sha256, &AtomicBool::new(false))?.1)
}

pub(crate) fn check_archive_cancellable(
    archive: &Path,
    sha256: &str,
    cancel: &AtomicBool,
) -> Result<(String, ResourceManifest)> {
    cancelled(cancel)?;
    let staging = Temporary::directory(
        std::env::temp_dir().join(format!("zzz-resources-{}", Uuid::new_v4())),
    )?;
    extract(&std::path::absolute(archive)?, sha256, &staging.0, cancel)
}

fn store(data: &Path) -> Result<PathBuf> {
    ensure!(data.is_absolute(), "数据目录必须是绝对路径");
    let root = data.join("runtime-resources");
    check_path(&root)?;
    Ok(root)
}

fn read_selection(root: &Path) -> Result<ResourceSelection> {
    let path = root.join("selection.json");
    check_path(&path)?;
    if !path.try_exists()? {
        return Ok(ResourceSelection {
            schema_version: 1,
            ..Default::default()
        });
    }
    let selection: ResourceSelection = serde_json::from_slice(&read_limited(&path, MAX_MANIFEST)?)?;
    ensure!(
        selection.schema_version == 1
            && selection.current.as_deref().is_none_or(hash_valid)
            && selection.previous.as_deref().is_none_or(hash_valid)
            && (selection.can_rollback || selection.previous.is_none())
            && (selection.current != selection.previous || !selection.can_rollback),
        "资源版本选择损坏，请修复 selection.json 后重试"
    );
    Ok(selection)
}

fn target(
    root: &Path,
    bundled: &Path,
    id: Option<&str>,
) -> Result<(PathBuf, Option<ResourceManifest>)> {
    if let Some(id) = id {
        ensure!(hash_valid(id), "资源版本编号必须为清单 SHA-256");
        let directory = root.join("versions").join(id);
        let manifest = validate_directory(&directory, Some(id))?;
        Ok((directory, Some(manifest)))
    } else {
        check_path(bundled)?;
        validate_semantics(bundled)?;
        Ok((bundled.to_path_buf(), None))
    }
}

pub fn status(data: &Path, bundled: &Path) -> Result<ResourceStatus> {
    let root = store(data)?;
    let selection = read_selection(&root)?;
    // 尚未启用独立资源时，状态查询不要求已安装游戏资源或 SDK。
    let (directory, manifest) = if selection.current.is_some() {
        target(&root, bundled, selection.current.as_deref())?
    } else {
        (bundled.to_path_buf(), None)
    };
    Ok(ResourceStatus {
        selection,
        directory,
        manifest,
    })
}

pub fn resolve(data: &Path, bundled: &Path) -> Result<PathBuf> {
    Ok(status(data, bundled)?.directory)
}

fn revision(selection: &ResourceSelection) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(selection)?)))
}

pub fn catalog(data: &Path) -> Result<RuntimeResourceCatalog> {
    let root = store(data)?;
    let selection = read_selection(&root)?;
    let directory = root.join("versions");
    check_path(&directory)?;
    let mut versions = Vec::new();
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => Some(entries),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(entries) = entries {
        for (index, entry) in entries.enumerate() {
            ensure!(index < 64, "已安装资源目录超过 64 项，请整理后再刷新");
            let entry = entry?;
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("资源目录名称无效"))?;
            ensure!(hash_valid(&id), "资源目录包含未登记项目");
            let result = read_manifest(&entry.path(), Some(&id));
            let (version, compatible, error) = match result {
                Ok(manifest) => (Some(manifest.version), true, None),
                Err(error) => (None, false, Some(format!("{error:#}"))),
            };
            versions.push(InstalledResources {
                id,
                version,
                compatible,
                error,
            });
        }
    }
    // 保留已选择但目录丢失的版本，允许用户看清错误后回滚。
    for id in [&selection.current, &selection.previous]
        .into_iter()
        .flatten()
    {
        if !versions.iter().any(|version| &version.id == id) {
            versions.push(InstalledResources {
                id: id.clone(),
                version: None,
                compatible: false,
                error: Some("已选择的资源目录缺失".into()),
            });
        }
    }
    versions.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(RuntimeResourceCatalog {
        revision: revision(&selection)?,
        selection,
        directory: directory.display().to_string(),
        versions,
    })
}

pub fn preview_change(
    data: &Path,
    bundled: &Path,
    request: &RuntimeResourceRequest,
) -> Result<ResourceSelection> {
    request.validate().map_err(anyhow::Error::msg)?;
    let root = store(data)?;
    let mut selection = read_selection(&root)?;
    let expected = match request {
        RuntimeResourceRequest::Activate { revision, .. }
        | RuntimeResourceRequest::Rollback { revision } => revision,
        RuntimeResourceRequest::Discover { .. }
        | RuntimeResourceRequest::Refresh
        | RuntimeResourceRequest::Download { .. }
        | RuntimeResourceRequest::Install { .. } => {
            anyhow::bail!("该操作不是版本切换")
        }
    };
    ensure!(
        revision(&selection)? == *expected,
        "资源选择已变化，请刷新后重新确认"
    );
    match request {
        RuntimeResourceRequest::Activate { id, .. } => {
            target(&root, bundled, Some(id))?;
            if selection.current.as_deref() != Some(id) {
                target(&root, bundled, selection.current.as_deref())?;
                selection.previous = selection.current.replace(id.clone());
                selection.can_rollback = true;
            }
        }
        RuntimeResourceRequest::Rollback { .. } => {
            ensure!(selection.can_rollback, "没有可回滚的上一版本");
            target(&root, bundled, selection.previous.as_deref())?;
            std::mem::swap(&mut selection.current, &mut selection.previous);
        }
        RuntimeResourceRequest::Discover { .. }
        | RuntimeResourceRequest::Refresh
        | RuntimeResourceRequest::Download { .. }
        | RuntimeResourceRequest::Install { .. } => unreachable!(),
    }
    Ok(selection)
}

pub fn apply_change(
    data: &Path,
    bundled: &Path,
    request: &RuntimeResourceRequest,
) -> Result<ResourceSelection> {
    let root = store(data)?;
    request.validate().map_err(anyhow::Error::msg)?;
    let _owner = od_core::config::ConfigStore::new(data)?.acquire_owner()?;
    let selection = preview_change(data, bundled, request)?;
    save_selection(&root, &selection)?;
    Ok(selection)
}

// 启动时只核对选择和清单；完整文件校验在原生工作线程中完成，避免阻塞桌面首个快照。
pub(crate) fn select_for_engine(data: &Path, bundled: &Path) -> Result<(PathBuf, Option<String>)> {
    let root = store(data)?;
    let selection = read_selection(&root)?;
    if let Some(id) = selection.current {
        let directory = root.join("versions").join(&id);
        read_manifest(&directory, Some(&id))?;
        Ok((directory, Some(id)))
    } else {
        Ok((bundled.to_path_buf(), None))
    }
}

pub fn install(data: &Path, archive: &Path, sha256: &str) -> Result<String> {
    let root = store(data)?;
    // 与引擎从启动到原生工作线程退出所持有的锁相同，暂停不能解锁。
    let _owner = od_core::config::ConfigStore::new(data)?.acquire_owner()?;
    let versions = root.join("versions");
    check_path(&versions)?;
    fs::create_dir_all(&versions)?;
    let staging = Temporary::directory(root.join(format!(".install-{}", Uuid::new_v4())))?;
    let (id, _) = extract(
        &std::path::absolute(archive)?,
        sha256,
        &staging.0,
        &AtomicBool::new(false),
    )?;
    let destination = versions.join(&id);
    check_path(&destination)?;
    if destination.try_exists()? {
        validate_directory(&destination, Some(&id))?;
    } else {
        fs::rename(&staging.0, &destination)?;
    }
    Ok(id)
}

fn save_selection(root: &Path, selection: &ResourceSelection) -> Result<()> {
    let destination = root.join("selection.json");
    check_path(&destination)?;
    fs::create_dir_all(root)?;
    let (temporary, mut file) =
        Temporary::file(root.join(format!(".selection-{}.tmp", Uuid::new_v4())))?;
    file.write_all(&serde_json::to_vec_pretty(selection)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary.0, &destination).context("无法保存版本选择，原选择保留")?;
    Ok(())
}

pub fn activate(data: &Path, bundled: &Path, id: &str) -> Result<ResourceSelection> {
    let root = store(data)?;
    ensure!(hash_valid(id), "资源版本编号必须为清单 SHA-256");
    let _owner = od_core::config::ConfigStore::new(data)?.acquire_owner()?;
    let mut selection = read_selection(&root)?;
    target(&root, bundled, Some(id))?;
    if selection.current.as_deref() != Some(id) {
        // 保证首次启用也有可校验的回滚目标。
        target(&root, bundled, selection.current.as_deref())?;
        selection.previous = selection.current.replace(id.into());
        selection.can_rollback = true;
        save_selection(&root, &selection)?;
    }
    Ok(selection)
}

pub fn rollback(data: &Path, bundled: &Path) -> Result<ResourceSelection> {
    let root = store(data)?;
    let _owner = od_core::config::ConfigStore::new(data)?.acquire_owner()?;
    let mut selection = read_selection(&root)?;
    ensure!(selection.can_rollback, "没有可回滚的上一版本");
    // 当前版本损坏时仍允许回到完整的上一版本。
    target(&root, bundled, selection.previous.as_deref())?;
    std::mem::swap(&mut selection.current, &mut selection.previous);
    save_selection(&root, &selection)?;
    Ok(selection)
}
