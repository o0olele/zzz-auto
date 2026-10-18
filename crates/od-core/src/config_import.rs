use crate::config::{
    AccountConfig, AccountInstance, AppGroup, CONFIG_SCHEMA, ConfigStore, GeneralSettings,
    GroupApp, default_group, reject_links,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ImportFile {
    pub path: String,
    pub sha256: String,
    pub disposition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
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
    #[serde(default)]
    after_done: Option<String>,
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

#[derive(Deserialize)]
struct LegacyEnvironment {
    screenshot_method: Option<String>,
    key_start_running: Option<String>,
    key_stop_running: Option<String>,
}

#[derive(Deserialize)]
struct LegacyGameAccount {
    game_region: Option<crate::daily::GameRegion>,
}

#[derive(Deserialize)]
struct LegacyCoffee {
    transport_point: Option<String>,
    #[serde(default)]
    run_charge_plan_afterwards: bool,
}

pub fn preview(source: &Path, data: &Path) -> Result<ImportPreview> {
    let files = scan(source, data)?;
    plan(&files)
}

pub fn apply(source: &Path, data: &Path, preview_id: &str) -> Result<AccountConfig> {
    // 先检查目录隔离，不能在旧程序目录创建占用锁。
    let current = preview(source, data)?;
    ensure!(
        current.preview_id == preview_id,
        "源配置已变化或预览指纹不符，请重新预览"
    );
    let store = ConfigStore::new(&resolved(data)?)?;
    let _owner = store.acquire_owner()?;
    store.apply_import(source, preview_id)
}

impl ConfigStore {
    pub fn preview_import(&self, source: &Path) -> Result<ImportPreview> {
        preview(source, self.data_directory())
    }

    // 调用方须持有目录占用锁；桌面引擎与离线导入共用转换及归档规则。
    pub fn apply_import(&self, source: &Path, preview_id: &str) -> Result<AccountConfig> {
        let data = self.data_directory();
        let files = scan(source, data)?;
        let preview = plan(&files)?;
        ensure!(
            preview.preview_id == preview_id,
            "源配置已变化或预览指纹不符，请重新预览"
        );
        let current = self.read()?;
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
        self.install_import(preview.config).map_err(Into::into)
    }
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
        "转换账号及区服、应用组、可兼容的截图设置与任务快捷键；其余字段及 YAML 原样归档，凭据、战斗脚本和旧运行记录仍待迁移。"
            .into(),
    ];
    let mut general = GeneralSettings::default();
    if let Some(value) = legacy.after_done.as_deref() {
        if let Some(action) = crate::after_done::AfterDoneAction::from_legacy(value) {
            general.after_done = action;
        } else {
            warnings.push("旧结束后动作不受支持，采用无动作；原值只归档，请导入后核对。".into());
        }
    }
    warnings.push(
        "结束后动作仅适用于桌面本次启动且正常完成的应用组，执行前有 60 秒可取消倒计时。".into(),
    );
    if let Some(env) = files.iter().find(|file| file.relative == "env.yml") {
        let environment: LegacyEnvironment = parse(env)?;
        if environment.screenshot_method.as_deref() == Some("print_window") {
            general.capture_method = crate::CaptureMethod::PrintWindow;
        } else {
            warnings.push("旧截图方式未直接转换，采用 FramePool 默认值，请在通用设置中核对；旧自动回退、BitBlt 和 PIL 不与 Maa 截图方式视为等价。".into());
        }
        for (value, target, label) in [
            (
                environment.key_start_running.as_deref(),
                &mut general.shortcuts.pause_resume,
                "暂停 / 继续",
            ),
            (
                environment.key_stop_running.as_deref(),
                &mut general.shortcuts.stop,
                "停止",
            ),
        ] {
            if let Some(value) = value {
                if let Some(key) = crate::shortcuts::ShortcutKey::from_legacy(value) {
                    *target = Some(key);
                } else {
                    warnings.push(format!(
                        "旧{label}快捷键不在 F1..F11 范围内，采用默认值，请导入后核对；原值只归档。"
                    ));
                }
            }
        }
        if general.shortcuts.validate().is_err() {
            general.shortcuts = crate::shortcuts::ShortcutSettings::default();
            warnings
                .push("旧任务快捷键重复或转换后冲突，采用 F9 / F10 默认值，请导入后核对。".into());
        }
        warnings.push("任务快捷键只暂停 / 继续或停止已有任务；旧开始键不会启动新任务。".into());
        converted.insert(env.relative.clone());
    }
    for item in legacy.instance_list {
        if item.active {
            active.push(item.idx);
        }
        let prefix = format!("{:02}/", item.idx);
        let game_region = if let Some(file) = files
            .iter()
            .find(|file| file.relative == format!("{prefix}game_account.yml"))
        {
            let account: LegacyGameAccount = parse(file)?;
            converted.insert(file.relative.clone());
            account.game_region
        } else {
            None
        };
        if game_region.is_none() {
            warnings.push(format!(
                "账号 {} 缺少区服，采用旧版默认国服；启用按日跳过前请核对。",
                item.idx
            ));
        }
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
                    name: None,
                    run_policy: crate::daily::RunPolicy::default(),
                    coffee: crate::coffee_run::CoffeeConfig::default(),
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
                name: None,
                run_policy: crate::daily::RunPolicy::default(),
                coffee: crate::coffee_run::CoffeeConfig::default(),
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
        // 旧版按应用组保存，组内文件优先；没有组内文件才读取早期账号级文件。
        for group in &groups {
            let grouped = format!("{prefix}{}/coffee.yml", group.id);
            let old = format!("{prefix}coffee.yml");
            let coffee = files
                .iter()
                .find(|file| file.relative == grouped)
                .or_else(|| files.iter().find(|file| file.relative == old));
            let enabled = group
                .apps
                .iter()
                .any(|app| app.app_id == "coffee" && app.enabled);
            if let Some(file) = coffee {
                let legacy: LegacyCoffee = parse(file)?;
                converted.insert(file.relative.clone());
                if legacy.transport_point.as_deref() != Some("澄辉坪 - 汀曼咖啡") {
                    warnings.push(format!("账号 {} 的 {} 组旧咖啡地点不适用当前固定路线，原值只归档；诊断仅支持澄辉坪 → 汀曼咖啡，请核对后再使用。", item.idx, group.id));
                }
                if legacy.run_charge_plan_afterwards {
                    warnings.push(format!(
                        "账号 {} 的 {} 组咖啡后体力计划尚未迁移，原值只归档，不自动运行。",
                        item.idx, group.id
                    ));
                }
            }
            if coffee.is_some() || enabled {
                warnings.push(format!("账号 {} 的 {} 组旧咖啡选择由体力计划动态决定，未转换为固定咖啡；采用汀曼特调、60000ms 超时、3000ms 延时，请导入后核对。咖啡为试运行，完整领取仍待实机验收。", item.idx, group.id));
            }
        }
        instances.push(AccountInstance {
            id: item.idx,
            name: item.name,
            active_in_od: item.active_in_od,
            game_region: game_region.unwrap_or_default(),
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
    let catalog = crate::applications()?;
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
    // 变更转换语义后使旧预览失效，避免用旧确认启用新的转换结果。
    hash.update(b"zzz-auto-config-import-v6\0");
    let mut manifest = Vec::new();
    for file in files {
        hash.update((file.relative.len() as u64).to_le_bytes());
        hash.update(file.relative.as_bytes());
        hash.update((file.bytes.len() as u64).to_le_bytes());
        hash.update(&file.bytes);
        manifest.push(ImportFile {
            path: format!("config/{}", file.relative),
            sha256: hex::encode(Sha256::digest(&file.bytes)),
            disposition: if file.relative == "env.yml" {
                "已检查截图与任务快捷键；其他环境字段仅归档，待迁移"
            } else if file.relative.ends_with("/coffee.yml") && converted.contains(&file.relative) {
                "已检查咖啡地点与后续计划；旧动态选单未转换，完整原文归档"
            } else if file.relative.ends_with("/game_account.yml")
                && converted.contains(&file.relative)
            {
                "已检查区服；凭据和其他账号字段仅归档，待迁移"
            } else if converted.contains(&file.relative) {
                "账号 / 应用组已转换；完整原文归档"
            } else {
                "仅归档，待迁移"
            }
            .into(),
        });
    }
    let preview_id = hex::encode(hash.finalize());
    let config = AccountConfig {
        reserved_instance_ids: Vec::new(),
        schema: CONFIG_SCHEMA,
        revision: 0,
        general,
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
