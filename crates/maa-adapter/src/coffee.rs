use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub fn validate_coffee_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root
        .canonicalize()
        .context("咖啡只读资源缺失，请先运行 coffee-bundle")?;
    let manifest_path = root.join("coffee-manifest.json");
    od_core::config::reject_links(&manifest_path)?;
    ensure!(
        std::fs::metadata(&manifest_path)?.len() <= 1024 * 1024,
        "咖啡清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
    ensure!(
        manifest["schema_version"] == 1
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["read_only"] == true
            && manifest["accepted"] == false,
        "咖啡资源版本或只读标记不兼容"
    );
    let allowed: BTreeSet<_> = [
        "pipeline/coffee-readonly.json",
        "model/ocr/ppocrv5/det.onnx",
        "model/ocr/ppocrv5/rec.onnx",
        "model/ocr/ppocrv5/keys.txt",
    ]
    .into_iter()
    .collect();
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|model| model["id"] == "ppocrv5")
        .context("缺少固定 OCR")?;
    let mut seen = BTreeSet::new();
    for file in manifest["files"].as_array().context("咖啡资源清单无效")? {
        let name = file["path"].as_str().context("咖啡路径无效")?;
        ensure!(
            allowed.contains(name) && seen.insert(name),
            "咖啡资源路径未登记或重复"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(path.canonicalize()?.starts_with(&root), "咖啡资源指向包外");
        let size = std::fs::metadata(&path)?.len();
        ensure!(
            size <= 128 * 1024 * 1024 && Some(size) == file["bytes"].as_u64(),
            "咖啡资源长度不匹配"
        );
        let hash = hex::encode(Sha256::digest(std::fs::read(path)?));
        ensure!(file["sha256"] == hash, "咖啡资源哈希不匹配：{name}");
        if name.starts_with("model/") {
            let pinned = ocr["files"]
                .as_array()
                .context("OCR 清单无效")?
                .iter()
                .find(|model| model["bundle_path"] == name)
                .context("OCR 文件未锁定")?;
            ensure!(pinned["sha256"] == hash, "咖啡 OCR 模型或字典不匹配");
        }
    }
    ensure!(seen == allowed, "咖啡资源不完整");
    for entry in walkdir::WalkDir::new(&root).min_depth(1) {
        let entry = entry?;
        od_core::config::reject_links(entry.path())?;
        if entry.file_type().is_file() {
            let name = entry
                .path()
                .strip_prefix(&root)?
                .to_string_lossy()
                .replace('\\', "/");
            ensure!(
                name == "coffee-manifest.json" || allowed.contains(name.as_str()),
                "咖啡包包含未登记文件"
            );
        }
    }
    let pipeline: Value =
        serde_json::from_slice(&std::fs::read(root.join("pipeline/coffee-readonly.json"))?)?;
    ensure!(
        pipeline == od_core::coffee::pipeline(),
        "咖啡只读节点与引擎不匹配，不能含输入动作或后继"
    );
    Ok(root)
}

#[derive(Debug, Serialize)]
pub struct CoffeeObservation {
    pub title_visible: bool,
    pub tinman_option_visible: bool,
    pub leave_option_visible: bool,
    /// 只描述可见状态，绝不表示已经喝过或当日完成。
    pub view: &'static str,
    pub checks: Vec<Value>,
}

impl crate::NativeSession {
    pub fn inspect_coffee_fixture(&self, image: &Path) -> Result<CoffeeObservation> {
        ensure!(
            self.game_window.is_none() && self.input_mode.is_none(),
            "咖啡只读检查只允许离线截图控制器"
        );
        validate_coffee_bundle(&self.resource_root)?;
        let image = crate::RecognitionImage::load(image)?;
        let pipeline = od_core::coffee::pipeline();
        let mut checks = Vec::new();
        let mut hits = Vec::new();
        for alias in ["Title", "TinmanOption", "LeaveOption"] {
            let node = &pipeline[format!("Coffee.ReadOnly.{alias}")];
            let mut params = node.clone();
            params
                .as_object_mut()
                .context("咖啡识别节点无效")?
                .remove("action");
            params.as_object_mut().unwrap().remove("recognition");
            let result = self.recognize_prepared(&image, "OCR", &params)?;
            let hit = result.detail["hit"]
                .as_bool()
                .context("咖啡识别缺少命中结果")?;
            hits.push(hit);
            checks.push(json!({"area":alias,"recognition":result}));
        }
        let view = if hits.iter().all(|hit| *hit) {
            "options_visible"
        } else if hits[0] {
            "dialogue_visible"
        } else {
            "unknown"
        };
        Ok(CoffeeObservation {
            title_visible: hits[0],
            tinman_option_visible: hits[1],
            leave_option_visible: hits[2],
            view,
            checks,
        })
    }
}
