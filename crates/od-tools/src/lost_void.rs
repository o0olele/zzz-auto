//! 固定参考目录。运行时不再依赖参考仓或 Python。
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

pub fn extract(reference: &Path, output: &Path) -> Result<serde_json::Value> {
    let reference = reference.canonicalize()?;
    let mut sources = BTreeMap::new();
    let mut files = Vec::new();
    for (source, target) in [
        ("lost_void_artifact_data.yml", "artifacts.yml"),
        ("lost_void_investigation_strategy.yml", "strategies.yml"),
    ] {
        let relative = format!("assets/game_data/hollow_zero/lost_void/{source}");
        od_core::config::reject_links(&reference.join(&relative))?;
        let bytes = crate::mail::read_within(&reference, &relative)?;
        if target == "artifacts.yml" {
            let list: Vec<od_core::lost_void::LostVoidArtifact> = serde_yaml::from_slice(&bytes)?;
            ensure!(!list.is_empty() && list.len() <= 4096, "藏品目录数量无效");
        } else {
            let list: Vec<serde_json::Value> = serde_yaml::from_slice(&bytes)?;
            ensure!(
                !list.is_empty() && list.iter().all(|s| s["strategy_name"].is_string()),
                "战略目录无效"
            );
        }
        sources.insert(relative, hex::encode(Sha256::digest(&bytes)));
        files.push((target, bytes));
    }
    std::fs::create_dir_all(output)?;
    for (target, bytes) in files {
        std::fs::write(output.join(target), bytes)?;
    }
    let lock = serde_json::json!({"schema": 1, "sources": sources});
    crate::write_json(&output.join("source-lock.json"), &lock)?;
    Ok(lock)
}
