//! 固定体力副本传送来源和 OCR，独立于已有日常执行包。
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
            "体力副本传送资源来源与输出不能重叠"
        );
    }
    let sources: Value = serde_json::from_str(include_str!(
        "../../../resources/charge-transport/source-lock.json"
    ))?;
    for source in sources.as_array().context("体力副本传送来源清单无效")? {
        let name = source["path"]
            .as_str()
            .context("体力副本传送来源路径无效")?;
        od_core::config::reject_links(&reference.join(name))?;
        let bytes = crate::mail::read_within(&reference, name)?;
        let digest = if source["encoding"] == "binary" {
            hex::encode(Sha256::digest(&bytes))
        } else {
            hex::encode(Sha256::digest(
                std::str::from_utf8(&bytes)?
                    .replace("\r\n", "\n")
                    .as_bytes(),
            ))
        };
        ensure!(source["sha256"] == digest, "体力副本传送来源已变化：{name}");
    }
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    for (name, bytes) in od_core::charge_transport_pipeline::templates() {
        files.insert(format!("image/charge-transport/{name}.png"), bytes.to_vec());
    }
    files.insert(
        "pipeline/charge_transport.json".into(),
        serde_json::to_vec_pretty(&od_core::charge_transport_pipeline::pipeline())?,
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
                "体力副本传送 OCR 文件不匹配"
            );
            files.insert(destination.clone(), bytes);
        }
    }
    let manifest = json!({"schema_version":1,"sdk_version":lock.sdk_version,"base_size":[1920,1080],"accepted":false,
        "capabilities":["charge_transport"],"sources":sources,
        "files":files.iter().map(|(name, bytes)| json!({"path":name,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>(),
        "limitations":["训练分类和完整副本名称必须同帧核对","传送到达不提供消费或次数证明","到达后仍须确认等级、燃竭或深度模式及完整挑战"]});
    files.insert(
        "charge_transport-manifest.json".into(),
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
                "体力副本传送输出含非受管文件或目录"
            );
        }
    }
    for (name, bytes) in files {
        crate::resources::safe_relative(&name)?;
        let path = output.join(name);
        std::fs::create_dir_all(path.parent().context("体力副本传送输出缺少父目录")?)?;
        std::fs::write(path, bytes)?;
    }
    Ok(manifest)
}
