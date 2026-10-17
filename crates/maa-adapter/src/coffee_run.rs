use anyhow::{Context, Result, ensure};
use od_core::coffee_run::CoffeeConfig;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

pub fn validate_coffee_run_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root
        .canonicalize()
        .context("咖啡执行资源缺失，请准备包含 coffee-run 的资源包")?;
    let path = root.join("coffee-run-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "咖啡执行清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        manifest["schema_version"] == 1
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["prototype_only"] == true
            && manifest["accepted"] == false
            && manifest["start"] == "dialogue_options",
        "咖啡执行资源不兼容"
    );
    let allowed: BTreeSet<_> = [
        "pipeline/coffee-run.json",
        "image/coffee/world_function.png",
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
        .context("缺少 OCR")?;
    let mut seen = BTreeSet::new();
    for file in manifest["files"].as_array().context("咖啡文件清单无效")? {
        let name = file["path"].as_str().context("咖啡路径无效")?;
        ensure!(
            allowed.contains(name) && seen.insert(name),
            "咖啡文件未登记或重复"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(path.canonicalize()?.starts_with(&root), "咖啡资源指向包外");
        ensure!(
            std::fs::metadata(&path)?.len() <= 128 * 1024 * 1024,
            "咖啡文件过大"
        );
        let bytes = std::fs::read(path)?;
        let hash = hex::encode(Sha256::digest(&bytes));
        ensure!(
            file["sha256"] == hash && file["bytes"] == bytes.len() as u64,
            "咖啡文件长度或哈希不匹配"
        );
        if name.starts_with("model/") {
            let pinned = ocr["files"]
                .as_array()
                .context("OCR 清单无效")?
                .iter()
                .find(|model| model["bundle_path"] == name)
                .context("OCR 文件未锁定")?;
            ensure!(pinned["sha256"] == hash, "咖啡 OCR 不匹配");
        }
        if name.starts_with("image/") {
            ensure!(
                bytes == include_bytes!("../../../resources/source/coffee/world_function.png"),
                "咖啡大世界模板不匹配"
            );
        }
    }
    ensure!(seen == allowed, "咖啡执行包不完整");
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
                name == "coffee-run-manifest.json" || allowed.contains(name.as_str()),
                "咖啡包含未登记文件"
            );
        }
    }
    let pipeline: Value =
        serde_json::from_slice(&std::fs::read(root.join("pipeline/coffee-run.json"))?)?;
    ensure!(
        pipeline == od_core::coffee_run::pipeline(),
        "咖啡执行节点与引擎不匹配"
    );
    Ok(root)
}

#[derive(Debug, Serialize)]
pub struct CoffeeRunObservation {
    pub view: &'static str,
    pub checks: Vec<Value>,
}

impl crate::NativeSession {
    pub fn load_coffee_run_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "运行中不能更换咖啡资源");
        let root = validate_coffee_run_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("资源路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "咖啡资源载入失败"
        );
        self.coffee_bundle = Some(root);
        Ok(())
    }
    pub fn prepare_game_coffee(&mut self, cancel: Arc<AtomicBool>, delay_ms: u32) -> Result<()> {
        ensure!(
            self.game_window.is_some() && self.input_mode == Some(crate::InputMode::Foreground),
            "咖啡实机需要前台键鼠"
        );
        validate_coffee_run_bundle(self.coffee_bundle.as_deref().context("咖啡资源未加载")?)?;
        ensure!(delay_ms <= 10000, "咖啡延时无效");
        self.release_inputs()?;
        crate::input::wait_delay(Duration::from_millis(delay_ms.into()), &cancel, &mut None);
        Ok(())
    }
    pub fn prepare_fixture_coffee(&mut self) -> Result<()> {
        ensure!(
            self.game_window.is_none() && self.input_mode == Some(crate::InputMode::Foreground),
            "咖啡离线控制器无效"
        );
        validate_coffee_run_bundle(&self.resource_root)?;
        self.release_inputs()
    }
    pub fn coffee_step(
        &mut self,
        entry: &str,
        config: CoffeeConfig,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
        game: bool,
    ) -> Result<crate::PipelineExecution> {
        config.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            [
                "Coffee.Inspect",
                "Coffee.Order",
                "Coffee.WaitEnergy",
                "Coffee.ConfirmEnergy",
                "Coffee.AfterEnergy",
                "Coffee.ConfirmBuff"
            ]
            .contains(&entry),
            "咖啡步骤未登记"
        );
        if game {
            ensure!(
                self.game_window.is_some() && self.input_mode == Some(crate::InputMode::Foreground),
                "咖啡需要前台游戏控制器"
            );
            validate_coffee_run_bundle(self.coffee_bundle.as_deref().context("咖啡资源未加载")?)?;
            self.prepare_input_target(CoffeeConfig::recovery_inputs()[0], &cancel, &mut None)?;
        } else {
            ensure!(
                self.game_window.is_none() && self.input_mode == Some(crate::InputMode::Foreground),
                "咖啡离线控制器无效"
            );
            validate_coffee_run_bundle(&self.resource_root)?;
        }
        self.run_pipeline_detailed(
            entry,
            &crate::PipelineInputPlan {
                keys: BTreeSet::new(),
                contacts: [0].into(),
            },
            cancel,
            timeout,
            &config.overrides().to_string(),
            true,
        )
    }
    pub fn inspect_coffee_run_fixture(
        &self,
        image: &Path,
        config: CoffeeConfig,
    ) -> Result<CoffeeRunObservation> {
        ensure!(
            self.game_window.is_none() && self.input_mode.is_none(),
            "咖啡截图检查只允许无输入离线控制器"
        );
        validate_coffee_run_bundle(&self.resource_root)?;
        let image = crate::RecognitionImage::load(image)?;
        let mut pipeline = od_core::coffee_run::pipeline();
        for (name, values) in config.overrides().as_object().unwrap() {
            for (key, value) in values.as_object().unwrap() {
                pipeline[name][key] = value.clone();
            }
        }
        let mut checks = Vec::new();
        let mut hits = Vec::new();
        for alias in [
            "Title",
            "Option",
            "EnergyPrompt",
            "EnergyLabel",
            "EnergyConfirm",
            "BuffTitle",
            "BuffConfirm",
            "WorldFunction",
            "WorldText",
        ] {
            let node = &pipeline[format!("Coffee.{alias}")];
            let mut params = serde_json::Map::new();
            for key in [
                "roi",
                "expected",
                "model",
                "threshold",
                "template",
                "method",
                "only_rec",
            ] {
                if let Some(value) = node.get(key) {
                    params.insert(key.into(), value.clone());
                }
            }
            let result = self.recognize_prepared(
                &image,
                node["recognition"].as_str().context("咖啡识别类型无效")?,
                &Value::Object(params),
            )?;
            hits.push(result.detail["hit"].as_bool().context("咖啡识别缺少命中")?);
            checks.push(json!({"area":alias,"recognition":result}));
        }
        let view = if hits[2] && hits[3] && hits[4] {
            "energy_reward"
        } else if hits[5] && hits[6] {
            "coffee_buff"
        } else if hits[7] && hits[8] {
            "world_visible"
        } else if hits[0] && hits[1] {
            "options_visible"
        } else {
            "unknown"
        };
        Ok(CoffeeRunObservation { view, checks })
    }
}
