//! 独立培养达成读取；只有有效结算上下文才解释左下角文字。
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::agent_goal::AgentGoalStatus;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
impl NativeSession {
    pub fn load_agent_goal_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换培养达成资源");
        self.release_inputs()?;
        let root = validate_agent_goal_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("培养达成路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "培养达成资源载入失败"
        );
        self.agent_goal_bundle = Some(root);
        Ok(())
    }
    pub fn clear_agent_goal_bundle(&mut self) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能移除培养达成能力");
        self.agent_goal_bundle = None;
        Ok(())
    }
    pub fn require_agent_goal(&self) -> Result<()> {
        validate_agent_goal_bundle(
            self.agent_goal_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    pub fn inspect_agent_goal(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<AgentGoalStatus>> {
        self.agent_goal_sight(frame, cancel, timeout, false)
    }
    pub(crate) fn agent_goal_on_settlement(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<AgentGoalStatus>> {
        self.agent_goal_sight(frame, cancel, timeout, true)
    }
    fn agent_goal_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
        known_settlement: bool,
    ) -> Result<Option<AgentGoalStatus>> {
        self.require_agent_goal()?;
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
            pipeline: od_core::agent_goal_pipeline::pipeline(),
        };
        let result = (|| -> Result<AgentGoalStatus> {
            if !known_settlement {
                let modal = words(&r.read("AgentGoal.Modal", None)?)?;
                if modal.iter().any(|w| {
                    ["确认", "确定", "取消", "获得", "恢复电量", "快捷使用"]
                        .contains(&w.text.as_str())
                }) {
                    return Ok(AgentGoalStatus::Unknown);
                }
                if point(
                    &words(&r.read("AgentGoal.Finish", None)?)?,
                    "完成",
                    [1566, 1004, 1832, 1056],
                )
                .is_none()
                {
                    return Ok(AgentGoalStatus::Unknown);
                }
            }
            let words = words(&r.read("AgentGoal.Goal", None)?)?;
            Ok(AgentGoalStatus::from_words(
                &words.into_iter().map(|w| w.text).collect::<Vec<_>>(),
            ))
        })();
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
}
pub fn validate_agent_goal_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("培养达成资源缺失")?;
    let path = root.join("agent_goal-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "培养达成清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sources = match manifest["schema_version"].as_u64() {
        Some(1) => include_str!("../../../resources/agent-goal/source-lock.json"),
        _ => anyhow::bail!("培养达成资源版本不兼容"),
    };
    ensure!(
        manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["agent_goal"])
            && manifest["sources"] == serde_json::from_str::<Value>(sources)?,
        "培养达成资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let pipeline = serde_json::to_vec_pretty(&od_core::agent_goal_pipeline::pipeline())?;
    expected.insert(
        "pipeline/agent_goal.json".into(),
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
        .context("培养达成文件清单无效")?;
    ensure!(files.len() == expected.len(), "培养达成文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("培养达成文件路径无效")?;
        let (size, hash) = expected.get(name).context("培养达成资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "培养达成文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "培养达成文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "培养达成文件内容不匹配"
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
                name == "agent_goal-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "培养达成资源含额外文件或目录"
        );
    }
    Ok(root)
}
