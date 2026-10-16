use anyhow::{Context, Result, ensure};
use od_core::map_transport::MapStep;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

pub fn validate_map_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root
        .canonicalize()
        .context("地图资源缺失，请先运行 map-bundle")?;
    let path = root.join("map-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "地图清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        manifest["schema_version"] == 2
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["prototype_only"] == true
            && manifest["accepted"] == false
            && manifest["route"]
                == json!({"area":od_core::map_transport::AREA_NAME,"point":od_core::map_transport::POINT_NAME}),
        "地图资源版本或路线不兼容"
    );
    let allowed: BTreeSet<_> = [
        "pipeline/map.json",
        "image/map/back.png",
        "image/map/function_menu.png",
        "image/map/preset_config.png",
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
    for file in manifest["files"].as_array().context("地图文件清单无效")? {
        let name = file["path"].as_str().context("地图路径无效")?;
        ensure!(
            allowed.contains(name) && seen.insert(name),
            "地图路径未登记或重复"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(path.canonicalize()?.starts_with(&root), "地图资源指向包外");
        let size = std::fs::metadata(&path)?.len();
        ensure!(
            size <= 128 * 1024 * 1024 && Some(size) == file["bytes"].as_u64(),
            "地图资源长度不匹配"
        );
        let hash = hex::encode(Sha256::digest(std::fs::read(path)?));
        ensure!(file["sha256"] == hash, "地图资源哈希不匹配：{name}");
        if name.starts_with("model/") {
            let pinned = ocr["files"]
                .as_array()
                .context("OCR 清单无效")?
                .iter()
                .find(|model| model["bundle_path"] == name)
                .context("OCR 文件未锁定")?;
            ensure!(pinned["sha256"] == hash, "地图 OCR 模型或字典不匹配");
        }
    }
    ensure!(seen == allowed, "地图资源不完整");
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
                name == "map-manifest.json" || allowed.contains(name.as_str()),
                "地图包包含未登记文件"
            );
        }
    }
    let nodes: Value = serde_json::from_slice(&std::fs::read(root.join("pipeline/map.json"))?)?;
    ensure!(
        nodes == od_core::map_transport::pipeline(),
        "地图流程与引擎不匹配"
    );
    Ok(root)
}

#[derive(Debug, Serialize)]
pub struct MapObservation {
    pub view: &'static str,
    pub checks: Vec<Value>,
}

impl crate::NativeSession {
    pub fn load_map_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "任务运行中不能替换地图资源");
        let root = validate_map_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("地图路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "地图资源载入失败"
        );
        self.map_bundle = Some(root);
        Ok(())
    }

    pub fn prepare_game_map(&mut self, cancel: Arc<AtomicBool>, delay_ms: u32) -> Result<()> {
        self.validate_game_map()?;
        ensure!(delay_ms <= 10_000, "地图延时超出限制");
        self.release_inputs()?;
        crate::input::wait_delay(Duration::from_millis(delay_ms.into()), &cancel, &mut None);
        Ok(())
    }

    fn validate_game_map(&self) -> Result<()> {
        ensure!(
            self.game_window.is_some() && self.input_mode == Some(crate::InputMode::Foreground),
            "地图实机流程需要前台键鼠游戏控制器"
        );
        validate_map_bundle(self.map_bundle.as_deref().context("地图资源尚未载入")?)?;
        Ok(())
    }

    pub fn game_map_step(
        &mut self,
        step: MapStep,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<crate::PipelineExecution> {
        self.validate_game_map()?;
        self.prepare_input_target(
            od_core::map_transport::MapConfig::recovery_inputs()[0],
            &cancel,
            &mut None,
        )?;
        self.run_pipeline_detailed(
            step.entry(),
            &crate::PipelineInputPlan {
                keys: BTreeSet::new(),
                contacts: [0].into(),
            },
            cancel,
            timeout,
            "{}",
            true,
        )
    }

    pub fn prepare_fixture_map(&mut self) -> Result<()> {
        ensure!(
            self.game_window.is_none() && self.input_mode == Some(crate::InputMode::Foreground),
            "地图执行只允许离线前台语义控制器"
        );
        validate_map_bundle(&self.resource_root)?;
        self.release_inputs()
    }

    pub fn inspect_map_fixture(&self, image: &Path) -> Result<MapObservation> {
        ensure!(
            self.game_window.is_none() && self.input_mode.is_none(),
            "地图截图检查只允许无输入离线控制器"
        );
        validate_map_bundle(&self.resource_root)?;
        let image = crate::RecognitionImage::load(image)?;
        let pipeline = od_core::map_transport::pipeline();
        let mut checks = Vec::new();
        let mut hits = Vec::new();
        for alias in [
            "Prompt",
            "ConfirmText",
            "Back",
            "AreaText",
            "PointText",
            "WorldFunction",
            "WorldPreset",
        ] {
            let node = &pipeline[format!("Map.{alias}")];
            let kind = node["recognition"].as_str().context("地图识别类型无效")?;
            let mut params = serde_json::Map::new();
            for key in [
                "roi",
                "expected",
                "model",
                "threshold",
                "template",
                "method",
                "green_mask",
            ] {
                if let Some(value) = node.get(key) {
                    params.insert(key.to_owned(), value.clone());
                }
            }
            let result = self.recognize_prepared(&image, kind, &Value::Object(params))?;
            hits.push(
                result.detail["hit"]
                    .as_bool()
                    .context("地图识别缺少命中结果")?,
            );
            checks.push(json!({"area":alias,"recognition":result}));
        }
        let view = if hits[0] && hits[1] {
            "transport_dialog"
        } else if hits[5] && hits[6] {
            "world_visible"
        } else if hits[2] && !hits[0] && hits[4] {
            "point_visible"
        } else if hits[2] && !hits[0] && hits[3] {
            "area_visible"
        } else {
            "unknown"
        };
        Ok(MapObservation { view, checks })
    }

    /// 固定步骤仅供测试仓验证；尚不提供实机入口。
    pub fn fixture_map_step(
        &mut self,
        step: MapStep,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<crate::PipelineExecution> {
        ensure!(
            self.game_window.is_none() && self.input_mode == Some(crate::InputMode::Foreground),
            "地图执行只允许离线前台语义控制器"
        );
        validate_map_bundle(&self.resource_root)?;
        self.run_pipeline_detailed(
            step.entry(),
            &crate::PipelineInputPlan {
                keys: BTreeSet::new(),
                contacts: [0].into(),
            },
            cancel,
            timeout,
            "{}",
            true,
        )
    }
}
