use anyhow::{Result, ensure};
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
            "资源来源与输出不能重叠"
        );
    }
    let source_path = "assets/game_data/screen_info/coffee_shop.yml";
    let source = crate::mail::read_within(&reference, source_path)?;
    let screen: crate::resources::LegacyScreen = serde_yaml::from_slice(&source)?;
    ensure!(
        screen.screen_id == "coffee_shop"
            && !screen.pc_alt
            && screen.area_list.len() == od_core::coffee::AREAS.len(),
        "咖啡来源画面已变化"
    );
    for expected in od_core::coffee::AREAS {
        let actual = crate::mail::area(&screen, expected.legacy_name)?;
        ensure!(
            actual.pc_rect == expected.rect
                && actual.text == expected.text
                && actual.template_id.is_empty(),
            "咖啡区域已变化：{}",
            expected.legacy_name
        );
    }
    let mut files = BTreeMap::new();
    files.insert(
        "pipeline/coffee-run.json".to_owned(),
        serde_json::to_vec_pretty(&od_core::coffee_approach::pipeline())?,
    );
    files.insert(
        "image/coffee/world_function.png".to_owned(),
        include_bytes!("../../../resources/source/coffee/world_function.png").to_vec(),
    );
    let lock = crate::models::locked_models()?;
    let ocr = lock
        .models
        .iter()
        .find(|model| model.id == "ppocrv5")
        .expect("锁定 OCR");
    for model in &ocr.files {
        if let Some(destination) = &model.bundle_path {
            let path = format!("ppocrv5/{}", model.path);
            od_core::config::reject_links(&models.join(&path))?;
            let bytes = crate::mail::read_within(&models, &path)?;
            ensure!(
                bytes.len() as u64 == model.bytes
                    && hex::encode(Sha256::digest(&bytes)) == model.sha256,
                "OCR 模型或字典不匹配"
            );
            files.insert(destination.clone(), bytes);
        }
    }
    let manifest = json!({"schema_version":2,"sdk_version":lock.sdk_version,"base_size":[1920,1080],"prototype_only":true,"accepted":false,"start":"world_or_dialogue_options","source":{"path":source_path,"sha256":hex::encode(Sha256::digest(source))},"files":files.iter().map(|(path,bytes)|json!({"path":path,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>()});
    files.insert(
        "coffee-run-manifest.json".into(),
        serde_json::to_vec_pretty(&manifest)?,
    );
    if output.exists() {
        for entry in walkdir::WalkDir::new(&output).min_depth(1) {
            let entry = entry?;
            od_core::config::reject_links(entry.path())?;
            if entry.file_type().is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(&output)?
                    .to_string_lossy()
                    .replace('\\', "/");
                ensure!(
                    files.contains_key(&relative),
                    "咖啡输出含非受管文件：{relative}"
                );
            }
        }
    }
    for (relative, bytes) in files {
        let path = output.join(relative);
        std::fs::create_dir_all(path.parent().expect("资源父目录"))?;
        std::fs::write(path, bytes)?;
    }
    Ok(manifest)
}
