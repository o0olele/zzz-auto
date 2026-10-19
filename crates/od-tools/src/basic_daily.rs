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
            "日常资源来源与输出不能重叠"
        );
    }
    let sources: Value =
        serde_json::from_str(include_str!("../../../resources/daily/source-lock.json"))?;
    for source in sources.as_array().context("日常来源清单无效")? {
        let relative = source["path"].as_str().context("来源路径无效")?;
        od_core::config::reject_links(&reference.join(relative))?;
        let bytes = crate::mail::read_within(&reference, relative)?;
        let normalized = std::str::from_utf8(&bytes)?.replace("\r\n", "\n");
        ensure!(
            source["sha256"] == hex::encode(Sha256::digest(normalized.as_bytes())),
            "日常来源已变化，请重新核对画面：{relative}"
        );
    }
    let mut files = BTreeMap::new();
    for (name, bytes) in od_core::basic_daily::templates() {
        files.insert(format!("image/daily/{name}.png"), bytes.to_vec());
    }
    files.insert(
        "pipeline/basic-daily.json".into(),
        serde_json::to_vec_pretty(&od_core::basic_daily::pipeline())?,
    );
    let lock = crate::models::locked_models()?;
    let ocr = lock
        .models
        .iter()
        .find(|model| model.id == "ppocrv5")
        .context("缺少固定 OCR")?;
    for model in &ocr.files {
        if let Some(destination) = &model.bundle_path {
            let relative = format!("ppocrv5/{}", model.path);
            od_core::config::reject_links(&models.join(&relative))?;
            let bytes = crate::mail::read_within(&models, &relative)?;
            ensure!(
                bytes.len() as u64 == model.bytes
                    && hex::encode(Sha256::digest(&bytes)) == model.sha256,
                "日常 OCR 模型或字典不匹配"
            );
            files.insert(destination.clone(), bytes);
        }
    }
    let manifest = json!({"schema_version":1,"sdk_version":lock.sdk_version,"base_size":[1920,1080],
        "accepted":false,"apps":od_core::basic_daily::APP_IDS,"sources":sources,
        "files":files.iter().map(|(path,bytes)|json!({"path":path,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>(),
        "limitations":["固定 1080p、前台键鼠和可见地图目标；真实领取后态和版本差异待实机验收","分支测试使用真实已留档画面及合成状态，不代表缺失状态实机通过"]});
    files.insert(
        "basic-daily-manifest.json".into(),
        serde_json::to_vec_pretty(&manifest)?,
    );
    if output.exists() {
        for entry in walkdir::WalkDir::new(&output).min_depth(1) {
            let entry = entry?;
            od_core::config::reject_links(entry.path())?;
            let relative = entry
                .path()
                .strip_prefix(&output)?
                .to_string_lossy()
                .replace('\\', "/");
            ensure!(
                if entry.file_type().is_file() {
                    files.contains_key(&relative)
                } else {
                    files
                        .keys()
                        .any(|name| name.starts_with(&format!("{relative}/")))
                },
                "日常输出包含非受管文件或目录：{relative}"
            );
        }
    }
    for (relative, bytes) in files {
        let path = output.join(relative);
        std::fs::create_dir_all(path.parent().context("日常资源父目录无效")?)?;
        std::fs::write(path, bytes)?;
    }
    Ok(manifest)
}
