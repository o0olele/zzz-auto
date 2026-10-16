use anyhow::{Context, Result, bail, ensure};
use od_core::config::{
    AccountConfig, AccountInstance, AppGroup, CONFIG_SCHEMA, ConfigStore, GroupApp, default_group,
    reject_links,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportFile {
    pub path: String,
    pub sha256: String,
    pub disposition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportPreview {
    pub preview_id: String,
    pub config: AccountConfig,
    pub files: Vec<ImportFile>,
    pub warnings: Vec<String>,
}

struct SourceFile {
    relative: String,
    bytes: Vec<u8>,
}

#[derive(Deserialize)]
struct LegacyAccounts {
    instance_list: Vec<LegacyInstance>,
}

#[derive(Deserialize)]
struct LegacyInstance {
    idx: u32,
    name: String,
    #[serde(default)]
    active: bool,
    #[serde(default = "yes")]
    active_in_od: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct LegacyGroup {
    #[serde(default)]
    app_list: Vec<LegacyGroupApp>,
}

#[derive(Deserialize)]
struct LegacyGroupApp {
    app_id: String,
    #[serde(default)]
    enabled: bool,
}

#[derive(Deserialize)]
struct OldGroup {
    #[serde(default)]
    app_order: Vec<String>,
    #[serde(default)]
    app_run_list: Vec<String>,
}

pub fn preview(source: &Path, data: &Path) -> Result<ImportPreview> {
    let files = scan(source, data)?;
    plan(&files)
}

pub fn apply(source: &Path, data: &Path, preview_id: &str) -> Result<AccountConfig> {
    let files = scan(source, data)?;
    let preview = plan(&files)?;
    ensure!(
        preview.preview_id == preview_id,
        "源配置已变化或预览指纹不符，请重新预览"
    );
    let data = resolved(data)?;
    let store = ConfigStore::new(&data)?;
    let _owner = store.acquire_owner()?;
    let current = store.read()?;
    if current.revision != 0 && current.import_id.as_deref() != Some(preview_id) {
        bail!("已有账号配置，首批导入不能覆盖；请使用新的数据目录");
    }
    let imports = data.join("imports");
    reject_links(&imports)?;
    fs::create_dir_all(&imports)?;
    let archive = imports.join(preview_id);
    reject_links(&archive)?;
    if archive.try_exists()? {
        // 重跑不得默默接受损坏的归档。
        for file in &files {
            let path = archive.join("config").join(&file.relative);
            reject_links(&path)?;
            ensure!(
                fs::read(path)? == file.bytes,
                "已有归档与源文件不符，导入未启用"
            );
        }
    } else {
        let temporary = imports.join(format!("import-{}.tmp", Uuid::new_v4()));
        fs::create_dir(&temporary)?;
        // 失败时保留未启用的临时归档，避免清理逻辑误删用户目录。
        for file in &files {
            write_new(&temporary.join("config").join(&file.relative), &file.bytes)?;
        }
        write_new(
            &temporary.join("manifest.json"),
            &serde_json::to_vec_pretty(&preview)?,
        )?;
        match fs::rename(&temporary, &archive) {
            Ok(()) => {}
            Err(error) => {
                return Err(error).context("归档启用失败；账号配置未修改，可重新执行导入");
            }
        }
    }
    store.install_import(preview.config).map_err(Into::into)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().context("归档路径无效")?)?;
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

// 新目录可以尚未创建；先解析最近存在的父目录，再补上普通路径部分。
fn resolved(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "旧程序和新数据目录必须为绝对路径");
    let mut missing = Vec::new();
    let mut parent = path;
    while !parent.try_exists()? {
        let name = parent.file_name().context("目录路径无效")?;
        ensure!(
            parent
                .components()
                .all(|p| !matches!(p, Component::ParentDir)),
            "目录不能包含 .."
        );
        missing.push(name.to_os_string());
        parent = parent.parent().context("找不到已存在的父目录")?;
    }
    let mut result = parent.canonicalize()?;
    for name in missing.into_iter().rev() {
        result.push(name);
    }
    Ok(result)
}

fn scan(source: &Path, data: &Path) -> Result<Vec<SourceFile>> {
    ensure!(source.is_absolute(), "旧程序目录必须为绝对路径");
    let source = source.canonicalize().context("旧程序目录不存在")?;
    let data = resolved(data)?;
    // Windows 的 canonicalize 统一长路径前缀，比较时还需忽略大小写。
    let source_key = source.to_string_lossy().to_lowercase();
    let data_key = data.to_string_lossy().to_lowercase();
    ensure!(
        !Path::new(&source_key).starts_with(&data_key)
            && !Path::new(&data_key).starts_with(&source_key),
        "新旧目录不能重叠，禁止共同写入旧配置"
    );
    let config = source.join("config");
    ensure!(config.is_dir(), "旧程序目录中缺少 config");
    let mut files = Vec::new();
    let mut total = 0;
    for entry in walkdir::WalkDir::new(&config)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        let mut link = metadata.file_type().is_symlink();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            link |= metadata.file_attributes() & 0x400 != 0;
        }
        ensure!(!link, "配置目录不能包含符号链接或目录联接");
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if !(name.ends_with(".yml") || name.ends_with(".yaml"))
            || name.ends_with(".sample.yml")
            || name.ends_with(".sample.yaml")
        {
            continue;
        }
        ensure!(
            metadata.len() <= 1024 * 1024 && files.len() < 4096,
            "旧配置文件过大或数量超过限制"
        );
        let relative = entry
            .path()
            .strip_prefix(&config)?
            .to_str()
            .context("配置路径不是 UTF-8")?
            .replace('\\', "/");
        ensure!(
            relative
                .split('/')
                .all(|part| !part.contains(':') && part != ".."),
            "配置路径无效"
        );
        let bytes = fs::read(entry.path())?;
        std::str::from_utf8(&bytes).map_err(|_| anyhow::anyhow!("配置不是 UTF-8：{relative}"))?;
        total += bytes.len();
        ensure!(
            bytes.len() <= 1024 * 1024 && total <= 32 * 1024 * 1024,
            "旧配置总量超过 32 MiB"
        );
        let _: serde_yaml::Value = serde_yaml::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("配置 YAML 损坏：{relative}"))?;
        files.push(SourceFile { relative, bytes });
    }
    Ok(files)
}

fn parse<T: serde::de::DeserializeOwned>(file: &SourceFile) -> Result<T> {
    serde_yaml::from_slice(&file.bytes)
        .map_err(|_| anyhow::anyhow!("配置结构无效：{}", file.relative))
}

fn plan(files: &[SourceFile]) -> Result<ImportPreview> {
    let root = files
        .iter()
        .find(|file| file.relative == "one_dragon.yml")
        .context("缺少 config/one_dragon.yml，无法确定旧账号列表")?;
    let legacy: LegacyAccounts = parse(root)?;
    let mut converted = HashSet::from([root.relative.clone()]);
    let mut instances = Vec::new();
    let mut active = Vec::new();
    let mut warnings = vec![
        "仅转换账号与应用组；其余字段及 YAML 原样归档，设置、凭据、战斗脚本和旧运行记录仍待迁移。"
            .into(),
    ];
    for item in legacy.instance_list {
        if item.active {
            active.push(item.idx);
        }
        let prefix = format!("{:02}/", item.idx);
        let mut groups = Vec::new();
        for file in files {
            let Some(suffix) = file.relative.strip_prefix(&prefix) else {
                continue;
            };
            let parts: Vec<_> = suffix.split('/').collect();
            if parts.len() == 2 && parts[1] == "_group.yml" {
                let group: LegacyGroup = parse(file)?;
                groups.push(AppGroup {
                    id: parts[0].into(),
                    apps: group
                        .app_list
                        .into_iter()
                        .map(|app| GroupApp {
                            app_id: app.app_id,
                            enabled: app.enabled,
                        })
                        .collect(),
                });
                converted.insert(file.relative.clone());
            }
        }
        if !groups.iter().any(|g| g.id == "one_dragon") {
            let mut group = AppGroup {
                id: "one_dragon".into(),
                apps: Vec::new(),
            };
            if let Some(file) = files
                .iter()
                .find(|f| f.relative == format!("{prefix}one_dragon_app.yml"))
            {
                let old: OldGroup = parse(file)?;
                ensure!(
                    old.app_order.iter().collect::<HashSet<_>>().len() == old.app_order.len()
                        && old.app_run_list.iter().collect::<HashSet<_>>().len()
                            == old.app_run_list.len(),
                    "旧应用顺序或启用列表存在重复项"
                );
                let mut seen = HashSet::new();
                for id in old.app_order.iter().chain(&old.app_run_list) {
                    if seen.insert(id.clone()) {
                        group.apps.push(GroupApp {
                            app_id: id.clone(),
                            enabled: old.app_run_list.contains(id),
                        });
                    }
                }
                converted.insert(file.relative.clone());
            }
            groups.push(group);
        }
        let defaults = default_group();
        let group = groups
            .iter_mut()
            .find(|g| g.id == "one_dragon")
            .expect("默认组已创建");
        for app in defaults.apps {
            if !group.apps.iter().any(|a| a.app_id == app.app_id) {
                group.apps.push(app);
            }
        }
        instances.push(AccountInstance {
            id: item.idx,
            name: item.name,
            active_in_od: item.active_in_od,
            groups,
        });
    }
    ensure!(active.len() <= 1, "旧配置存在多个当前账号，需先修正");
    let active_instance = active
        .first()
        .copied()
        .or_else(|| instances.first().map(|i| i.id))
        .context("旧账号列表为空")?;
    if active.is_empty() {
        warnings.push("旧配置没有当前账号，预览选用列表中第一个账号。".into());
    }
    let catalog = od_core::applications()?;
    for instance in &instances {
        for group in &instance.groups {
            if group
                .apps
                .iter()
                .any(|app| !catalog.iter().any(|a| a.id == app.app_id))
            {
                warnings.push(format!(
                    "账号 {} 的 {} 组含未注册应用，已保留原顺序与启用值，暂不可运行。",
                    instance.id, group.id
                ));
            }
        }
    }
    let mut hash = Sha256::new();
    let mut manifest = Vec::new();
    for file in files {
        hash.update((file.relative.len() as u64).to_le_bytes());
        hash.update(file.relative.as_bytes());
        hash.update((file.bytes.len() as u64).to_le_bytes());
        hash.update(&file.bytes);
        manifest.push(ImportFile {
            path: format!("config/{}", file.relative),
            sha256: hex::encode(Sha256::digest(&file.bytes)),
            disposition: if converted.contains(&file.relative) {
                "账号 / 应用组已转换；完整原文归档"
            } else {
                "仅归档，待迁移"
            }
            .into(),
        });
    }
    let preview_id = hex::encode(hash.finalize());
    let config = AccountConfig {
        schema: CONFIG_SCHEMA,
        revision: 0,
        active_instance,
        instances,
        import_id: Some(preview_id.clone()),
    };
    config.validate()?;
    Ok(ImportPreview {
        preview_id,
        config,
        files: manifest,
        warnings,
    })
}
