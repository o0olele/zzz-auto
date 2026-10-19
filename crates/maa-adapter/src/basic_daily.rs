use anyhow::{Context, Result, ensure};
use od_core::basic_daily::DailyProgress;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

pub fn validate_basic_daily_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root
        .canonicalize()
        .context("日常执行资源缺失，请准备 basic-daily 资源包")?;
    let mut pinned: BTreeMap<String, Vec<u8>> = od_core::basic_daily::templates()
        .into_iter()
        .map(|(name, bytes)| (format!("image/daily/{name}.png"), bytes.to_vec()))
        .collect();
    let pipeline = od_core::basic_daily::pipeline();
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut allowed: BTreeSet<_> = pinned.keys().cloned().collect();
    allowed.insert("pipeline/basic-daily.json".into());
    for file in ocr["files"].as_array().context("OCR 清单无效")? {
        if let Some(path) = file["bundle_path"].as_str() {
            allowed.insert(path.into());
        }
    }
    let manifest_path = root.join("basic-daily-manifest.json");
    od_core::config::reject_links(&manifest_path)?;
    ensure!(
        std::fs::metadata(&manifest_path)?.len() <= 1024 * 1024,
        "日常清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
    ensure!(
        manifest["schema_version"] == 1
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["apps"] == json!(od_core::basic_daily::APP_IDS),
        "日常执行资源版本不兼容"
    );
    let mut seen = BTreeSet::new();
    for file in manifest["files"].as_array().context("日常文件清单无效")? {
        let name = file["path"].as_str().context("日常路径无效")?;
        ensure!(
            allowed.contains(name) && seen.insert(name.to_owned()),
            "日常文件未登记或重复"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(path.canonicalize()?.starts_with(&root), "日常资源指向包外");
        let metadata = std::fs::metadata(&path)?;
        let limit = if name.starts_with("model/") {
            128 * 1024 * 1024
        } else {
            1024 * 1024
        };
        ensure!(
            metadata.is_file() && metadata.len() <= limit && file["bytes"] == metadata.len(),
            "日常文件长度无效"
        );
        let bytes = std::fs::read(&path)?;
        let hash = hex::encode(Sha256::digest(&bytes));
        ensure!(file["sha256"] == hash, "日常文件哈希不匹配");
        if let Some(expected) = pinned.remove(name) {
            ensure!(bytes == expected, "日常模板与固定资源不匹配");
        } else if name.starts_with("model/") {
            let model = ocr["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["bundle_path"] == name)
                .context("OCR 未锁定")?;
            ensure!(model["sha256"] == hash, "日常 OCR 或字典不匹配");
        } else {
            ensure!(
                serde_json::from_slice::<Value>(&bytes)? == pipeline,
                "日常执行节点与引擎不匹配"
            );
        }
    }
    ensure!(seen == allowed, "日常资源包不完整");
    for entry in walkdir::WalkDir::new(&root).min_depth(1) {
        let entry = entry?;
        od_core::config::reject_links(entry.path())?;
        let relative = entry
            .path()
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        ensure!(
            if entry.file_type().is_file() {
                relative == "basic-daily-manifest.json" || allowed.contains(&relative)
            } else {
                allowed
                    .iter()
                    .any(|name| name.starts_with(&format!("{relative}/")))
            },
            "日常资源包含额外文件或目录"
        );
    }
    Ok(root)
}
impl crate::NativeSession {
    pub fn load_basic_daily_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "任务执行中不能替换日常资源");
        let root = validate_basic_daily_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("日常路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "日常资源载入失败"
        );
        self.daily_bundle = Some(root);
        Ok(())
    }
    pub fn basic_daily_step(
        &mut self,
        progress: &DailyProgress,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
        game: bool,
    ) -> Result<crate::PipelineExecution> {
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "日常控制器模式不匹配"
        );
        validate_basic_daily_bundle(self.daily_bundle.as_deref().unwrap_or(&self.resource_root))?;
        let pipeline = od_core::basic_daily::pipeline();
        let entry = progress.entry();
        ensure!(pipeline.get(&entry).is_some(), "日常步骤未登记");
        if game {
            self.prepare_input_target(
                od_core::mail::MailConfig::recovery_inputs()[1],
                &cancel,
                &mut None,
            )?;
        }
        let mut overrides = json!({});
        if progress.stage == "ScratchRow" {
            let rows = [
                ([780, 522], [1134, 556]),
                ([782, 556], [1138, 588]),
                ([788, 590], [1134, 632]),
            ];
            let (begin, end) = rows
                .get(usize::from(progress.scratch_rows))
                .context("刮层编号无效")?;
            overrides["Scratch.ScratchRow"] = json!({"begin":begin,"end":end});
        }
        self.run_pipeline_detailed(
            &entry,
            &crate::PipelineInputPlan {
                keys: [18, 27, 70, 78, 87].into(),
                contacts: [0].into(),
            },
            cancel,
            timeout,
            &overrides.to_string(),
            true,
        )
    }
}
