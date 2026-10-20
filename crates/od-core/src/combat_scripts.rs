//! 用户战斗 YAML 的只读预览与不可变快照导入；不与旧程序共写配置。
use crate::{conditional::Script, config::reject_links};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use ts_rs::TS;

const CATEGORIES: [&str; 4] = [
    "auto_battle",
    "dodge",
    "auto_battle_operation",
    "auto_battle_state_handler",
];
const MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CombatScriptImportPreview {
    pub source: String,
    pub preview_id: String,
    pub snapshot_id: String,
    pub previous: Option<String>,
    pub battle: Vec<String>,
    pub dodge: Vec<String>,
    pub files: u32,
    #[ts(type = "number")]
    pub bytes: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    bytes: u64,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    id: String,
    source: String,
    battle: Vec<String>,
    dodge: Vec<String>,
    files: Vec<Entry>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Active {
    schema: u32,
    id: String,
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    reject_links(path)?;
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= limit,
        "脚本文件不是普通文件或过大"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "脚本文件过大");
    Ok(bytes)
}
fn names(files: &BTreeMap<String, Vec<u8>>, category: &str) -> Vec<String> {
    files
        .keys()
        .filter_map(|path| path.strip_prefix(&format!("{category}/")))
        .filter_map(|name| {
            name.strip_suffix(".merged.yml")
                .or_else(|| name.strip_suffix(".sample.yml"))
                .or_else(|| name.strip_suffix(".yml"))
        })
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn scan(source: &Path) -> Result<(PathBuf, BTreeMap<String, Vec<u8>>)> {
    reject_links(source)?;
    let mut root = source.canonicalize()?;
    if root.join("config/auto_battle").is_dir() {
        root = root.join("config");
    }
    reject_links(&root)?;
    let mut files = BTreeMap::new();
    let mut total = 0;
    for category in CATEGORIES {
        let folder = root.join(category);
        reject_links(&folder)?;
        if !folder.try_exists()? {
            continue;
        }
        for entry in std::fs::read_dir(folder)? {
            let path = entry?.path();
            reject_links(&path)?;
            ensure!(
                path.is_file(),
                "脚本目录只能包含普通 YAML 文件：{}",
                path.display()
            );
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .context("脚本文件名不是 UTF-8")?;
            if !name.ends_with(".yml") {
                continue;
            }
            let bytes = read(&path, 4 * 1024 * 1024)?;
            total += bytes.len() as u64;
            ensure!(
                total <= MAX_BYTES && files.len() < 4096,
                "脚本导入超过文件数或 64 MiB 上限"
            );
            files.insert(format!("{category}/{name}"), bytes);
        }
    }
    for category in ["auto_battle", "dodge"] {
        for name in names(&files, category) {
            Script::load_entry(&root, category, &name, true)?;
            if ["yml", "sample.yml"].iter().any(|suffix| {
                root.join(category)
                    .join(format!("{name}.{suffix}"))
                    .is_file()
            }) {
                Script::load_entry(&root, category, &name, false)?;
            }
        }
    }
    ensure!(
        !names(&files, "auto_battle").is_empty() || !names(&files, "dodge").is_empty(),
        "来源没有可导入的战斗或闪避脚本"
    );
    Ok((root, files))
}
fn active_id(data: &Path) -> Result<Option<String>> {
    let path = data.join("combat-scripts/active.json");
    reject_links(&path)?;
    if !path.try_exists()? {
        return Ok(None);
    }
    let active: Active = serde_json::from_slice(&read(&path, 1024)?)?;
    ensure!(
        active.schema == 1 && valid_id(&active.id),
        "战斗脚本选择文件损坏"
    );
    Ok(Some(active.id))
}
fn identity(root: &Path, files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut digest = Sha256::new();
    digest.update(root.to_string_lossy().as_bytes());
    for (path, bytes) in files {
        digest.update((path.len() as u64).to_le_bytes());
        digest.update(path.as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    hex::encode(digest.finalize())
}
pub fn preview(source: &Path, data: &Path) -> Result<CombatScriptImportPreview> {
    ensure!(data.is_absolute(), "脚本数据目录必须是绝对路径");
    reject_links(data)?;
    let (root, files) = scan(source)?;
    let snapshot_id = identity(&root, &files);
    let previous = active_id(data)?;
    let preview_id = hash(&serde_json::to_vec(&(data, &snapshot_id, &previous))?);
    Ok(CombatScriptImportPreview {
        source: root.display().to_string(),
        preview_id,
        snapshot_id,
        previous,
        battle: names(&files, "auto_battle"),
        dodge: names(&files, "dodge"),
        files: files.len() as u32,
        bytes: files.values().map(|b| b.len() as u64).sum(),
    })
}
pub fn active_root(data: &Path) -> Result<Option<PathBuf>> {
    let Some(id) = active_id(data)? else {
        return Ok(None);
    };
    let root = data.join("combat-scripts/snapshots").join(&id);
    reject_links(&root)?;
    validate_snapshot(&root, &id)?;
    Ok(Some(root))
}
fn validate_snapshot(root: &Path, id: &str) -> Result<()> {
    let manifest: Manifest =
        serde_json::from_slice(&read(&root.join("manifest.json"), 1024 * 1024)?)?;
    ensure!(
        manifest.schema == 1 && manifest.id == id && manifest.files.len() <= 4096,
        "脚本快照清单无效"
    );
    let mut seen = BTreeSet::new();
    let mut files = BTreeMap::new();
    let mut total = 0;
    for file in manifest.files {
        let Some((category, name)) = file.path.split_once('/') else {
            anyhow::bail!("脚本快照路径无效");
        };
        ensure!(
            CATEGORIES.contains(&category)
                && !name.contains(['/', '\\', ':'])
                && name.ends_with(".yml")
                && seen.insert(file.path.clone()),
            "脚本快照路径无效或重复"
        );
        let bytes = read(&root.join(&file.path), 4 * 1024 * 1024)?;
        total += bytes.len() as u64;
        ensure!(
            file.bytes == bytes.len() as u64 && file.sha256 == hash(&bytes) && total <= MAX_BYTES,
            "脚本快照内容不匹配"
        );
        files.insert(file.path, bytes);
    }
    ensure!(
        identity(Path::new(&manifest.source), &files) == id,
        "脚本快照身份不匹配"
    );
    let (_, actual) = scan(root)?;
    ensure!(files == actual, "脚本快照包含清单外 YAML");
    Ok(())
}
pub fn catalog(data: &Path) -> Result<(Option<String>, Vec<String>, Vec<String>)> {
    let Some(root) = active_root(data)? else {
        return Ok((None, Vec::new(), Vec::new()));
    };
    let (_, files) = scan(&root)?;
    Ok((
        active_id(data)?,
        names(&files, "auto_battle"),
        names(&files, "dodge"),
    ))
}
pub fn select_root(data: &Path, fallback: &Path, category: &str, name: &str) -> Result<PathBuf> {
    if let Some(root) = active_root(data)? {
        for suffix in ["yml", "sample.yml", "merged.yml"] {
            if root
                .join(category)
                .join(format!("{name}.{suffix}"))
                .try_exists()?
            {
                return Ok(root);
            }
        }
    }
    Ok(fallback.into())
}
pub fn apply(source: &Path, data: &Path, preview_id: &str) -> Result<CombatScriptImportPreview> {
    reject_links(data)?;
    let directory = data.join("combat-scripts");
    reject_links(&directory)?;
    std::fs::create_dir_all(&directory)?;
    let lock_path = directory.join("import.lock");
    reject_links(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.try_lock().context("其他进程正在导入战斗脚本")?;
    let result = preview(source, data)?;
    ensure!(
        result.preview_id == preview_id,
        "脚本来源或当前选择已变化，请重新预览"
    );
    let (_, files) = scan(source)?;
    ensure!(
        identity(Path::new(&result.source), &files) == result.snapshot_id,
        "脚本来源已变化，请重新预览"
    );
    let destination = directory.join("snapshots").join(&result.snapshot_id);
    reject_links(&destination)?;
    if !destination.try_exists()? {
        std::fs::create_dir_all(directory.join("snapshots"))?;
        let staging = directory.join(format!(".import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&staging)?;
        let manifest = Manifest {
            schema: 1,
            id: result.snapshot_id.clone(),
            source: result.source.clone(),
            battle: result.battle.clone(),
            dodge: result.dodge.clone(),
            files: files
                .iter()
                .map(|(path, bytes)| Entry {
                    path: path.clone(),
                    bytes: bytes.len() as u64,
                    sha256: hash(bytes),
                })
                .collect(),
        };
        for (path, bytes) in &files {
            let path = staging.join(path);
            std::fs::create_dir_all(path.parent().context("脚本路径无父目录")?)?;
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staging.join("manifest.json"))?;
        file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(staging, &destination)?;
    }
    validate_snapshot(&destination, &result.snapshot_id)?;
    let pointer = serde_json::to_vec(&Active {
        schema: 1,
        id: result.snapshot_id.clone(),
    })?;
    let temporary = directory.join(format!(".active-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&pointer)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, directory.join("active.json"))?;
    active_root(data)?;
    Ok(result)
}
