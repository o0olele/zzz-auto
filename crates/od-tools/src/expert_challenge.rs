//! 固定专业挑战室来源和 OCR，独立于已有日常执行包。
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

pub fn build_bundle(reference: &Path, models: &Path, output: &Path) -> Result<Value> {
    for path in [reference, models, output] {
        od_core::config::reject_links(path)?;
    }
    let reference = reference.canonicalize()?;
    let models = models.canonicalize()?;
    let output = crate::mail::resolved_output(output)?;
    for source in [&reference, &models] {
        ensure!(
            !output.starts_with(source) && !source.starts_with(&output),
            "专业挑战室资源来源与输出不能重叠"
        );
    }
    let sources: Value = serde_json::from_str(include_str!(
        "../../../resources/expert-challenge/source-lock.json"
    ))?;
    for source in sources.as_array().context("专业挑战室来源清单无效")? {
        let name = source["path"].as_str().context("专业挑战室来源路径无效")?;
        od_core::config::reject_links(&reference.join(name))?;
        let bytes = crate::mail::read_within(&reference, name)?;
        ensure!(
            source["sha256"]
                == hex::encode(Sha256::digest(
                    std::str::from_utf8(&bytes)?
                        .replace("\r\n", "\n")
                        .as_bytes()
                )),
            "专业挑战室来源已变化：{name}"
        );
    }
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    files.insert(
        "pipeline/expert_challenge.json".into(),
        serde_json::to_vec_pretty(&od_core::expert_challenge_pipeline::pipeline())?,
    );
    let lock = crate::models::locked_models()?;
    let ocr = lock
        .models
        .iter()
        .find(|m| m.id == "ppocrv5")
        .context("缺少固定 OCR")?;
    for file in &ocr.files {
        if let Some(destination) = &file.bundle_path {
            let relative = format!("ppocrv5/{}", file.path);
            od_core::config::reject_links(&models.join(&relative))?;
            let bytes = crate::mail::read_within(&models, &relative)?;
            ensure!(
                bytes.len() as u64 == file.bytes
                    && hex::encode(Sha256::digest(&bytes)) == file.sha256,
                "专业挑战室 OCR 文件不匹配"
            );
            files.insert(destination.clone(), bytes);
        }
    }
    let manifest = json!({"schema_version":1,"sdk_version":lock.sdk_version,"base_size":[1920,1080],"accepted":false,
        "capabilities":["expert_challenge"],"sources":sources,
        "files":files.iter().map(|(name, bytes)| json!({"path":name,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>(),
        "limitations":["普通专业挑战室的燃竭、40电量、详情和结算","依赖本次完整传送及选队、出战与战斗子流程","明确OFF及关闭确认文案缺少真实留档，未知状态不消费","正式体力入口、指定等级与实机验收保持待办"]});
    files.insert(
        "expert_challenge-manifest.json".into(),
        serde_json::to_vec_pretty(&manifest)?,
    );
    if output.exists() {
        for entry in walkdir::WalkDir::new(&output).min_depth(1) {
            let entry = entry?;
            od_core::config::reject_links(entry.path())?;
            let name = entry
                .path()
                .strip_prefix(&output)?
                .to_string_lossy()
                .replace('\\', "/");
            ensure!(
                if entry.file_type().is_file() {
                    files.contains_key(&name)
                } else {
                    files.keys().any(|p| p.starts_with(&format!("{name}/")))
                },
                "专业挑战室输出含非受管文件或目录"
            );
        }
    }
    for (name, bytes) in files {
        crate::resources::safe_relative(&name)?;
        let path = output.join(name);
        std::fs::create_dir_all(path.parent().context("专业挑战室输出缺少父目录")?)?;
        std::fs::write(path, bytes)?;
    }
    Ok(manifest)
}
