use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub fn validate_engagement_reward_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root
        .canonicalize()
        .context("活跃度奖励只读资源缺失，请先运行 engagement-reward-bundle")?;
    let manifest_path = root.join("engagement-reward-manifest.json");
    od_core::config::reject_links(&manifest_path)?;
    ensure!(
        std::fs::metadata(&manifest_path)?.len() <= 1024 * 1024,
        "活跃度奖励清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
    ensure!(
        manifest["schema_version"] == od_core::engagement_reward::BUNDLE_SCHEMA_VERSION
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["read_only"] == true
            && manifest["accepted"] == false,
        "活跃度奖励资源版本或只读标记不兼容，请重新运行 engagement-reward-bundle"
    );
    let allowed: BTreeSet<_> = [
        "pipeline/engagement-reward-readonly.json",
        "image/engagement-reward/completed.png",
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
    for file in manifest["files"]
        .as_array()
        .context("活跃度奖励文件清单无效")?
    {
        let name = file["path"].as_str().context("活跃度奖励路径无效")?;
        ensure!(
            allowed.contains(name) && seen.insert(name),
            "活跃度奖励文件未登记或重复"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            path.canonicalize()?.starts_with(&root),
            "活跃度奖励资源指向包外"
        );
        let metadata = std::fs::metadata(&path)?;
        let limit = if name.starts_with("model/") {
            128 * 1024 * 1024
        } else {
            1024 * 1024
        };
        ensure!(
            metadata.is_file()
                && metadata.len() <= limit
                && Some(metadata.len()) == file["bytes"].as_u64(),
            "活跃度奖励文件长度无效"
        );
        let hash = hex::encode(Sha256::digest(std::fs::read(&path)?));
        ensure!(file["sha256"] == hash, "活跃度奖励资源哈希不匹配：{name}");
        if name.starts_with("model/") {
            let pinned = ocr["files"]
                .as_array()
                .context("OCR 清单无效")?
                .iter()
                .find(|model| model["bundle_path"] == name)
                .context("OCR 文件未锁定")?;
            ensure!(pinned["sha256"] == hash, "活跃度奖励 OCR 模型或字典不匹配");
        }
        if name.starts_with("image/") {
            ensure!(
                hash == od_core::engagement_reward::TEMPLATE_SHA256,
                "活跃度奖励完成模板不匹配"
            );
        }
    }
    ensure!(seen == allowed, "活跃度奖励资源不完整");
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
                name == "engagement-reward-manifest.json" || allowed.contains(name.as_str())
            } else {
                allowed
                    .iter()
                    .any(|file| file.starts_with(&format!("{name}/")))
            },
            "活跃度奖励包包含未登记文件或目录"
        );
    }
    let pipeline: Value = serde_json::from_slice(&std::fs::read(
        root.join("pipeline/engagement-reward-readonly.json"),
    )?)?;
    ensure!(
        pipeline == od_core::engagement_reward::pipeline(),
        "活跃度奖励只读节点与引擎不匹配，不能含输入动作或后继"
    );
    Ok(root)
}

#[derive(Debug, Serialize)]
pub struct EngagementRewardObservation {
    pub read_only: bool,
    pub daily_tab_selected: bool,
    pub daily_title_visible: bool,
    pub confirm_text_visible: bool,
    pub preview_title_visible: bool,
    pub completed_marker_visible: bool,
    /// 可见标记只有在日常选中页签和标题同时命中且无歧义时才有画面上下文。
    pub completed_marker_on_daily: bool,
    pub view: &'static str,
    pub checks: Vec<Value>,
}

impl crate::NativeSession {
    pub fn inspect_engagement_reward_fixture(
        &self,
        image: &Path,
    ) -> Result<EngagementRewardObservation> {
        ensure!(
            self.game_window.is_none() && self.input_mode.is_none(),
            "活跃度奖励只读检查只允许离线截图控制器"
        );
        validate_engagement_reward_bundle(&self.resource_root)?;
        let image = crate::RecognitionImage::load(image)?;
        let pipeline = od_core::engagement_reward::pipeline();
        let mut checks = Vec::new();
        let mut hits = Vec::new();
        for area in od_core::engagement_reward::AREAS
            .iter()
            .chain(std::iter::once(&od_core::engagement_reward::DAILY_TAB))
        {
            let mut params = pipeline[format!("EngagementReward.ReadOnly.{}", area.alias)].clone();
            let kind = params["recognition"]
                .as_str()
                .context("活跃度奖励识别类型缺失")?
                .to_owned();
            let object = params.as_object_mut().context("活跃度奖励节点无效")?;
            object.remove("action");
            object.remove("recognition");
            let result = self.recognize_prepared(&image, &kind, &params)?;
            hits.push(
                result.detail["hit"]
                    .as_bool()
                    .context("活跃度奖励识别缺少命中结果")?,
            );
            checks.push(json!({"area":area.alias,"recognition":result}));
        }
        let view = od_core::engagement_reward::visible_view(hits[4], hits[0], hits[2]);
        Ok(EngagementRewardObservation {
            read_only: true,
            daily_tab_selected: hits[4],
            daily_title_visible: hits[0],
            confirm_text_visible: hits[1],
            preview_title_visible: hits[2],
            completed_marker_visible: hits[3],
            completed_marker_on_daily: view == "daily_visible" && hits[3],
            view,
            checks,
        })
    }
}
