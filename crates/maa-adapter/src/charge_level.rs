use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::charge_level::{LevelReading, LevelView, canonical_level};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(crate) struct LevelSight {
    pub(crate) view: LevelView,
    pub(crate) choices: BTreeMap<String, [i32; 2]>,
}
impl NativeSession {
    pub fn load_charge_level_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换等级资源");
        self.release_inputs()?;
        let root = validate_charge_level_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("等级路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "等级资源载入失败"
        );
        self.charge_level_bundle = Some(root);
        Ok(())
    }
    pub fn clear_charge_level_bundle(&mut self) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能移除等级能力");
        self.charge_level_bundle = None;
        Ok(())
    }
    pub fn inspect_charge_level(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LevelView>> {
        Ok(self
            .level_sight(frame, cancel, timeout, false)?
            .map(|s| s.view))
    }
    pub(crate) fn hunt_level_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LevelSight>> {
        self.level_sight(frame, cancel, timeout, true)
    }
    fn level_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
        hunt_context: bool,
    ) -> Result<Option<LevelSight>> {
        validate_charge_level_bundle(
            self.charge_level_bundle
                .as_deref()
                .context("指定等级资源未载入")?,
        )?;
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let started = Instant::now();
        let r = Reader {
            session: self,
            frame,
            cancel,
            started,
            timeout,
            pipeline: od_core::charge_level_pipeline::pipeline(),
        };
        let result = (|| -> Result<LevelSight> {
            let read = |n: &str| words(&r.read(&format!("ChargeLevel.{n}"), None)?);
            let mut sight = LevelSight {
                view: LevelView::Unknown,
                choices: BTreeMap::new(),
            };
            if !hunt_context {
                let modal = read("Modal")?;
                if modal.iter().any(|w| {
                    ["确认", "确定", "取消", "获得", "快捷使用", "确定并出战"]
                        .contains(&w.text.as_str())
                }) {
                    sight.view = LevelView::Blocked;
                    return Ok(sight);
                }
                let title = read("Title")?;
                if point(&read("Street")?, "街区", [238, 24, 388, 82]).is_none()
                    || title.len() != 1
                    || od_core::charge_transport::canonical_name("恶名狩猎", &title[0].text)
                        .is_none()
                {
                    return Ok(sight);
                }
            }
            let header = read("Header")?;
            let current: Vec<_> = header
                .iter()
                .filter_map(|w| canonical_level(&w.text))
                .collect();
            if current.len() > 1
                || header.iter().any(|w| {
                    (w.text.contains("等级") || w.text.to_lowercase().contains("lv"))
                        && canonical_level(&w.text).is_none()
                })
            {
                sight.view = LevelView::Blocked;
                return Ok(sight);
            }
            for word in read("Choices")? {
                let Some(level) = canonical_level(&word.text) else {
                    if word.text.contains("等级") || word.text.to_lowercase().contains("lv") {
                        sight.view = LevelView::Blocked;
                        return Ok(sight);
                    }
                    continue;
                };
                let [x, y, w, h] = word.rect;
                ensure!(
                    x >= 1570 && y >= 100 && w > 0 && h > 0 && x + w <= 1828 && y + h <= 424,
                    "等级文字框超出登记列表"
                );
                if sight.choices.insert(level, word.center()).is_some() {
                    sight.view = LevelView::Blocked;
                    return Ok(sight);
                }
            }
            let reading = LevelReading {
                current: current.into_iter().next(),
                choices: sight.choices.keys().cloned().collect(),
            };
            reading.validate().map_err(anyhow::Error::msg)?;
            sight.view = LevelView::Page(reading);
            Ok(sight)
        })();
        if cancel.load(Ordering::Acquire) {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
}
pub fn validate_charge_level_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("深度追猎等级资源缺失")?;
    let path = root.join("charge_level-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "深度追猎等级清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sources = match manifest["schema_version"].as_u64() {
        Some(1) => include_str!("../../../resources/charge-level/source-lock.json"),
        _ => anyhow::bail!("深度追猎等级资源版本不兼容"),
    };
    ensure!(
        manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["charge_level"])
            && manifest["sources"] == serde_json::from_str::<Value>(sources)?,
        "深度追猎等级资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let pipeline = serde_json::to_vec_pretty(&od_core::charge_level_pipeline::pipeline())?;
    expected.insert(
        "pipeline/charge_level.json".into(),
        (
            pipeline.len() as u64,
            hex::encode(Sha256::digest(&pipeline)),
        ),
    );
    for file in ocr["files"].as_array().context("OCR 清单无效")? {
        if let Some(name) = file["bundle_path"].as_str() {
            expected.insert(
                name.into(),
                (
                    file["bytes"].as_u64().context("OCR 大小无效")?,
                    file["sha256"].as_str().context("OCR 哈希无效")?.into(),
                ),
            );
        }
    }
    let files = manifest["files"]
        .as_array()
        .context("深度追猎等级文件清单无效")?;
    ensure!(files.len() == expected.len(), "深度追猎等级文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("深度追猎等级文件路径无效")?;
        let (size, hash) = expected.get(name).context("深度追猎等级资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "深度追猎等级文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "深度追猎等级文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "深度追猎等级文件内容不匹配"
        );
    }
    for entry in walkdir::WalkDir::new(&root).min_depth(1) {
        let entry = entry?;
        od_core::config::reject_links(entry.path())?;
        let name = entry
            .path()
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        ensure!(
            if entry.file_type().is_file() {
                name == "charge_level-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "深度追猎等级资源含额外文件或目录"
        );
    }
    Ok(root)
}
