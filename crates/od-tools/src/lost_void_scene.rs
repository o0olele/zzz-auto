//! 固定迷失之地画面来源和 OCR，独立于已有日常执行包。
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

pub fn build_bundle(reference: &Path, models: &Path, output: &Path) -> Result<Value> {
    for path in [reference, output] {
        od_core::config::reject_links(path)?;
    }
    let reference = reference.canonicalize()?;
    let output = crate::mail::resolved_output(output)?;
    ensure!(
        !output.starts_with(&reference) && !reference.starts_with(&output),
        "迷失之地画面资源来源与输出不能重叠"
    );
    let sources: Value = serde_json::from_str(include_str!(
        "../../../resources/lost-void/screen-source-lock.json"
    ))?;
    for source in sources.as_array().context("迷失之地画面来源清单无效")? {
        let name = source["path"]
            .as_str()
            .context("迷失之地画面来源路径无效")?;
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
        ensure!(source["sha256"] == digest, "迷失之地画面来源已变化：{name}");
    }
    for (name, raw, mask) in od_core::lost_void_settlement::reward_sources() {
        for (kind, bytes) in [("raw", raw), ("mask", mask)] {
            ensure!(
                crate::mail::read_within(
                    &reference,
                    &format!("assets/template/hollow/{name}/{kind}.png")
                )? == bytes,
                "结算奖励模板或掩码偏离固定来源：{name}/{kind}"
            );
        }
    }
    for (name, raw, mask) in od_core::random_play::source_images()
        .into_iter()
        .filter(|(name, _, _)| name.starts_with("avatar_"))
    {
        for (file, bytes) in [("raw.png", raw), ("mask.png", mask)] {
            ensure!(
                crate::mail::read_within(
                    &reference,
                    &format!("assets/template/predefined_team/{name}/{file}")
                )? == bytes,
                "迷失之地头像原图或掩码与固定来源不匹配：{name}/{file}"
            );
        }
    }
    build_embedded_bundle(models, &output)
}

/// 使用本仓留档的模板、识别规则和来源清单；OCR 仍须匹配固定模型哈希。
pub fn build_embedded_bundle(models: &Path, output: &Path) -> Result<Value> {
    for path in [models, output] {
        od_core::config::reject_links(path)?;
    }
    let models = models.canonicalize()?;
    let output = crate::mail::resolved_output(output)?;
    ensure!(
        !output.starts_with(&models) && !models.starts_with(&output),
        "迷失之地画面资源来源与输出不能重叠"
    );
    // sources 记录迁移时的历史依据，不代表读取了开发者当前的参考仓。
    let sources: Value = serde_json::from_str(include_str!(
        "../../../resources/lost-void/screen-source-lock.json"
    ))?;
    let local_source = od_core::lost_void_loading::lore_source();
    ensure!(
        local_source["asset_sha256"]
            == hex::encode(Sha256::digest(od_core::lost_void_loading::lore_logo())),
        "加载标识与留档来源摘要不一致"
    );
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    for (name, bytes) in od_core::lost_void_scene::templates() {
        files.insert(format!("image/lost-void-scene/{name}.png"), bytes.to_vec());
    }
    for (name, bytes) in od_core::lost_void_scene::avatar_templates() {
        files.insert(name.into(), bytes.to_vec());
    }
    files.insert(
        "pipeline/lost_void_scene.json".into(),
        serde_json::to_vec_pretty(&od_core::lost_void_scene::pipeline())?,
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
                "迷失之地画面 OCR 文件不匹配"
            );
            files.insert(destination.clone(), bytes);
        }
    }
    let manifest = json!({"schema_version":1,"sdk_version":lock.sdk_version,"base_size":[1920,1080],"accepted":false,
        "capabilities":["lost_void_scene"],"sources":sources,"local_sources":[local_source,od_core::lost_void_settlement::settlement_source(),od_core::lost_void_deploy::count_source()],
        "files":files.iter().map(|(name, bytes)| json!({"path":name,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>(),
        "limitations":["同帧识别迷失之地画面及完整悬赏进度","观察不提供输入或次数证明","完整入口、选择、移动、战斗和结算须由业务流程确认"]});
    files.insert(
        "lost_void_scene-manifest.json".into(),
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
                "迷失之地画面输出含非受管文件或目录"
            );
        }
    }
    for (name, bytes) in files {
        crate::resources::safe_relative(&name)?;
        let path = output.join(name);
        std::fs::create_dir_all(path.parent().context("迷失之地画面输出缺少父目录")?)?;
        std::fs::write(path, bytes)?;
    }
    Ok(manifest)
}
