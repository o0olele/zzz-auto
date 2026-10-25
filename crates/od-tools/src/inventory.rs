use anyhow::{Context, Result, bail, ensure};
use od_core::AppDefinition;
use regex::Regex;
use serde_json::json;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    if !root.exists() {
        return Ok(result);
    }
    for entry in WalkDir::new(root) {
        let entry = entry?;
        if entry.file_type().is_file() {
            result.push(entry.into_path());
        }
    }
    result.sort();
    Ok(result)
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn constant(text: &str, field: &str) -> Result<String> {
    let re = Regex::new(&format!(r#"(?m)^{}\s*=\s*['"]([^'"\r\n]+)['"]"#, field))?;
    re.captures(text)
        .map(|cap| cap[1].into())
        .with_context(|| format!("缺少静态常量 {field}"))
}

fn execution(id: &str) -> Result<(&'static str, &'static str)> {
    Ok(match id {
        "email"
        | "engagement_reward"
        | "city_fund"
        | "scratch_card"
        | "ridu_weekly"
        | "trigrams_collection"
        | "redemption_code"
        | "drive_disc_dismantle" => (
            "pipeline",
            "识别、点击和画面等待为主，结果及配置由 Rust 管理",
        ),
        "daily_signin" | "notify" => ("rust_service", "网络服务和通知使用 Rust，不经过游戏控制器"),
        "coffee" | "random_play" | "hou_hou_bakery" | "intel_board" | "suibian_temple" => (
            "hybrid",
            "Rust 保留配置、导航和经营决策，界面步骤使用 Maa Pipeline",
        ),
        "charge_plan" | "notorious_hunt" | "shiyu_defense" | "life_on_line" => (
            "hybrid",
            "Rust 管理挑战、战斗与恢复，界面操作使用 Maa Pipeline",
        ),
        "auto_battle" | "dodge_assistant" => (
            "rust",
            "高频状态时间窗、声音及动作抢占由 Rust 实现，识别和输入经 Maa",
        ),
        "withered_domain" | "lost_void" | "world_patrol" | "commission_assistant" => (
            "rust",
            "路径、转向和复杂状态决策由 Rust 实现，识别和输入经 Maa",
        ),
        "one_dragon"
        | "screenshot_helper"
        | "operation_debug"
        | "mouse_sensitivity_checker"
        | "predefined_team_checker" => ("rust", "编排、调试、测量或识别结果处理保留 Rust 业务接口"),
        _ => bail!("未登记执行方式的应用 {id}，请先补迁移决策"),
    })
}

pub fn scan(reference: &Path, output: &Path) -> Result<()> {
    let application_files = files(&reference.join("src/zzz_od/application"))?;
    let gui_files = files(&reference.join("src/zzz_od/gui"))?;
    let factories: Vec<_> = application_files
        .iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with("_factory.py"))
        })
        .collect();
    ensure!(
        factories.len() == 30,
        "参考工厂数量为 {}，需重新登记路线图范围",
        factories.len()
    );
    let mut apps = Vec::new();
    let priority_pattern = Regex::new(r"(?m)^PRIORITY\s*=\s*(\d+)")?;
    let default_group_pattern = Regex::new(r"(?m)^DEFAULT_GROUP\s*=\s*True")?;
    for factory in factories {
        let parent = factory.parent().context("工厂没有所属目录")?;
        let constants: Vec<_> = application_files
            .iter()
            .filter(|path| {
                path.parent() == Some(parent)
                    && path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().ends_with("_const.py"))
            })
            .collect();
        let mut registration = None;
        for path in constants {
            let text = std::fs::read_to_string(path)?;
            if let Ok(id) = constant(&text, "APP_ID") {
                registration = Some((id, constant(&text, "APP_NAME")?, text));
                break;
            }
        }
        let (id, name, text) =
            registration.with_context(|| format!("工厂 {} 没有注册常量", factory.display()))?;
        let (execution, reason) = execution(&id)?;
        let source_files: Vec<_> = application_files
            .iter()
            .filter(|path| {
                path.starts_with(parent) && path.extension().is_some_and(|ext| ext == "py")
            })
            .map(|path| relative(reference, path))
            .collect();
        let configs = source_files
            .iter()
            .filter(|path| path.ends_with("config.py"))
            .cloned()
            .collect();
        let run_records = source_files
            .iter()
            .filter(|path| path.ends_with("run_record.py"))
            .cloned()
            .collect();
        let settings = gui_files
            .iter()
            .filter(|path| {
                path.to_string_lossy().contains(&id) && path.to_string_lossy().contains("setting")
            })
            .map(|path| relative(reference, path))
            .collect();
        let priority = priority_pattern
            .captures(&text)
            .map(|cap| cap[1].parse::<i32>())
            .transpose()?
            .unwrap_or(0);
        apps.push(AppDefinition {
            id,
            name,
            execution: execution.into(),
            reason: reason.into(),
            factory: relative(reference, factory),
            configs,
            settings,
            run_records,
            source_files,
            priority,
            default_group: default_group_pattern.is_match(&text),
            implemented: false,
        });
    }
    let reference_factory_count = apps.len();
    // 本工程新增的 Rust 应用没有旧 Python 工厂，重新扫描时仍须保留登记。
    apps.extend(
        od_core::applications()?
            .into_iter()
            .filter(|app| app.factory.starts_with("crates/")),
    );
    apps.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));
    ensure!(
        apps.iter()
            .map(|app| &app.id)
            .collect::<std::collections::HashSet<_>>()
            .len()
            == apps.len(),
        "存在重复应用 ID"
    );
    crate::write_json(&output.join("resources/catalog/applications.json"), &apps)?;
    let mut resources = Vec::new();
    for path in files(&reference.join("assets"))? {
        resources.push(json!({"path": relative(reference, &path), "sha256": crate::sha256(&path)?, "bytes": path.metadata()?.len()}));
    }
    let plugin_factories: Vec<_> = files(&reference.join("plugins"))?
        .into_iter()
        .filter(|path| path.to_string_lossy().ends_with("_factory.py"))
        .map(|path| relative(reference, &path))
        .collect();
    let public_files: Vec<_> = files(&reference.join("src/zzz_od/backend"))?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "py"))
        .map(|path| relative(reference, &path))
        .collect();
    let revision = std::process::Command::new("git")
        .args(["-C"])
        .arg(reference)
        .args(["rev-parse", "HEAD"])
        .output()?;
    ensure!(revision.status.success(), "参考目录不是 Git 仓库");
    crate::write_json(
        &output.join("docs/migration/inventory.json"),
        &json!({
            "schema_version": 1, "reference_revision": String::from_utf8(revision.stdout)?.trim(),
            "applications": "../../resources/catalog/applications.json", "factory_count": reference_factory_count,
            "native_app_count": apps.len() - reference_factory_count,
            "gui_files": gui_files.iter().map(|path| relative(reference, path)).collect::<Vec<_>>(),
            "plugin_factories": plugin_factories, "public_interface_sources": public_files, "resources": resources,
            "acceptance": "pending", "models": "外部下载模型未在参考 assets 中出现，需另备模型、字典与旧结果"
        }),
    )?;
    println!(
        "已登记 {} 个应用、{} 个界面文件和 {} 项资源",
        apps.len(),
        gui_files.len(),
        resources.len()
    );
    Ok(())
}

pub fn sdk_manifest(directory: &Path, output: &Path) -> Result<()> {
    let mut binaries = Vec::new();
    for path in files(directory)? {
        if path.parent() == Some(directory) && path.extension().is_some_and(|ext| ext == "dll") {
            let data = std::fs::read(&path)?;
            ensure!(
                data.len() > 64 && &data[..2] == b"MZ",
                "{} 不是 PE 文件",
                path.display()
            );
            let offset = u32::from_le_bytes(data[60..64].try_into()?) as usize;
            ensure!(
                data.get(offset..offset + 4) == Some(b"PE\0\0")
                    && data.get(offset + 4..offset + 6) == Some(&[0x64, 0x86]),
                "{} 不是 Windows x64 DLL",
                path.display()
            );
            binaries.push(json!({"path": relative(directory, &path), "sha256": crate::sha256(&path)?, "bytes": data.len()}));
        }
    }
    ensure!(
        binaries
            .iter()
            .any(|entry| entry["path"] == "MaaFramework.dll"),
        "SDK 未包含 MaaFramework.dll"
    );
    crate::write_json(
        output,
        &json!({"sdk_version": "5.14.3", "binding_version": "1.25.2", "target": "x86_64-pc-windows-msvc", "binaries": binaries}),
    )
}
