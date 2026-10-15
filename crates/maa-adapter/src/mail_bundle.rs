use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub fn validate_mail_bundle(root: &Path) -> Result<PathBuf> {
    let root = root
        .canonicalize()
        .context("邮件资源包缺失或无法读取，请重新完整解压包含邮件资源的客户端包，保留 resources/generated/mail 目录")?;
    let manifest_path = root.join("mail-manifest.json");
    ensure!(
        std::fs::metadata(&manifest_path)
            .context("邮件资源清单缺失或无法读取，请重新完整解压客户端包")?
            .len()
            <= 1024 * 1024,
        "邮件清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
    ensure!(
        manifest["schema_version"] == 1
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080]),
        "邮件资源版本或坐标基准不兼容"
    );
    ensure!(
        manifest["pipeline_source_sha256"]
            == hex::encode(Sha256::digest(include_bytes!(
                "../../../resources/mail/pipeline.json"
            ))),
        "邮件流程与引擎版本不匹配，请重新生成资源"
    );
    let allowed: BTreeSet<String> = [
        "pipeline/mail.json",
        "pipeline/mail-areas.json",
        "image/mail/menu/back.png",
        "image/mail/menu/dynamic_wallpaper.png",
        "image/mail/normal_world/function_menu.png",
        "image/mail/normal_world/preset_config.png",
        "model/ocr/ppocrv5/det.onnx",
        "model/ocr/ppocrv5/rec.onnx",
        "model/ocr/ppocrv5/keys.txt",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let mut seen = BTreeSet::new();
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|model| model["id"] == "ppocrv5")
        .context("缺少固定 OCR")?;
    for file in manifest["files"].as_array().context("邮件资源清单无效")? {
        let name = file["path"].as_str().context("邮件路径无效")?;
        ensure!(
            allowed.contains(name) && seen.insert(name.to_owned()),
            "邮件资源路径未登记或重复"
        );
        let path = root.join(name).canonicalize()?;
        ensure!(path.starts_with(&root), "邮件资源指向包外");
        let metadata = std::fs::metadata(&path)?;
        ensure!(
            metadata.len() <= 128 * 1024 * 1024 && Some(metadata.len()) == file["bytes"].as_u64(),
            "邮件资源长度不匹配"
        );
        let hash = hex::encode(Sha256::digest(std::fs::read(path)?));
        ensure!(file["sha256"] == hash, "邮件资源哈希不匹配：{name}");
        if name.starts_with("model/") {
            let pinned = ocr["files"]
                .as_array()
                .context("OCR 清单无效")?
                .iter()
                .find(|model| model["bundle_path"] == name)
                .context("OCR 文件未锁定")?;
            ensure!(pinned["sha256"] == hash, "邮件 OCR 模型或字典不匹配");
        }
    }
    ensure!(seen == allowed, "邮件资源不完整");
    for entry in walkdir::WalkDir::new(&root).min_depth(1) {
        let entry = entry?;
        ensure!(!entry.file_type().is_symlink(), "邮件资源包不允许符号链接");
        if entry.file_type().is_file() {
            let name = entry
                .path()
                .strip_prefix(&root)?
                .to_string_lossy()
                .replace('\\', "/");
            ensure!(
                name == "mail-manifest.json" || allowed.contains(&name),
                "邮件包包含未登记文件"
            );
        }
    }
    let areas: Value =
        serde_json::from_slice(&std::fs::read(root.join("pipeline/mail-areas.json"))?)?;
    let area_names = [
        "Title",
        "ClaimText",
        "ConfirmText",
        "Back",
        "MenuBack",
        "MenuWallpaper",
        "MenuMail",
        "WorldFunction",
        "WorldPreset",
        "WorldMenu",
    ];
    ensure!(
        areas
            .as_object()
            .is_some_and(|nodes| nodes.len() == area_names.len()),
        "邮件区域数量不匹配"
    );
    for alias in area_names {
        let node = &areas[format!("Mail.Area.{alias}")];
        ensure!(
            node["action"] == "DoNothing" && node.get("next").is_none(),
            "邮件识别区域不能含动作或后继"
        );
        ensure!(
            ["OCR", "TemplateMatch", "DirectHit"]
                .iter()
                .any(|kind| node["recognition"] == *kind),
            "邮件区域含未支持识别"
        );
        let roi: [i32; 4] = serde_json::from_value(node["roi"].clone())?;
        let [x, y, w, h] = roi;
        ensure!(
            x >= 0
                && y >= 0
                && w > 0
                && h > 0
                && i64::from(x) + i64::from(w) <= 1920
                && i64::from(y) + i64::from(h) <= 1080,
            "邮件区域越界"
        );
        if node["recognition"] == "OCR" {
            ensure!(node["model"] == "ppocrv5", "邮件区域不能选择其他 OCR");
        }
        if node["recognition"] == "TemplateMatch" {
            for template in node["template"].as_array().context("模板列表无效")? {
                let name = format!("image/{}", template.as_str().context("模板路径无效")?);
                ensure!(allowed.contains(&name), "邮件区域模板未登记");
            }
        }
    }
    let mut expected: Value =
        serde_json::from_str(include_str!("../../../resources/mail/pipeline.json"))?;
    for (name, node) in expected.as_object_mut().context("流程无效")? {
        if name.starts_with('$') {
            continue;
        }
        for field in ["roi", "target"] {
            if let Some(alias) = node[field]
                .as_str()
                .filter(|name| name.starts_with("Mail.Area."))
            {
                node[field] = areas[alias]["roi"].clone();
            }
        }
        for (key, value) in [
            ("recognition", json!("DirectHit")),
            ("action", json!("DoNothing")),
            ("pre_delay", json!(0)),
            ("post_delay", json!(0)),
            ("rate_limit", json!(25)),
            ("timeout", json!(10_000)),
            ("next", json!([])),
            ("on_error", json!([])),
            ("focus", json!({})),
        ] {
            node.as_object_mut()
                .context("节点无效")?
                .entry(key)
                .or_insert(value);
        }
    }
    let actual: Value = serde_json::from_slice(&std::fs::read(root.join("pipeline/mail.json"))?)?;
    ensure!(actual == expected, "邮件流程含非本引擎定义的节点或动作");
    Ok(root)
}

impl crate::NativeSession {
    pub fn load_mail_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "任务运行中不能替换邮件资源");
        let root = validate_mail_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("邮件路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "邮件资源载入失败"
        );
        Ok(())
    }
}
