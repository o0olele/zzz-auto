use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

/// 随便观子流程包，不宣称尚未接齐的完整应用已可运行。
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
            "随便观资源来源与输出不能重叠"
        );
    }
    let sources: Value = serde_json::from_str(include_str!(
        "../../../resources/suibian-temple/source-lock.json"
    ))?;
    let adventure_sources: Value = serde_json::from_str(include_str!(
        "../../../resources/suibian-temple/adventure-source-lock.json"
    ))?;
    let craft_sources: Value = serde_json::from_str(include_str!(
        "../../../resources/suibian-temple/craft-source-lock.json"
    ))?;
    let yum_sources: Value = serde_json::from_str(include_str!(
        "../../../resources/suibian-temple/yum-source-lock.json"
    ))?;
    let sales_sources: Value = serde_json::from_str(include_str!(
        "../../../resources/suibian-temple/sales-source-lock.json"
    ))?;
    let good_goods_sources: Value = serde_json::from_str(include_str!(
        "../../../resources/suibian-temple/good-goods-source-lock.json"
    ))?;
    let boo_box_sources: Value = serde_json::from_str(include_str!(
        "../../../resources/suibian-temple/boo-box-source-lock.json"
    ))?;
    for source in sources
        .as_array()
        .context("随便观来源清单无效")?
        .iter()
        .chain(adventure_sources.as_array().context("游历来源清单无效")?)
        .chain(craft_sources.as_array().context("制造来源清单无效")?)
        .chain(yum_sources.as_array().context("饮茶仙来源清单无效")?)
        .chain(sales_sources.as_array().context("售卖来源清单无效")?)
        .chain(
            good_goods_sources
                .as_array()
                .context("好物铺来源清单无效")?,
        )
        .chain(boo_box_sources.as_array().context("邦巢来源清单无效")?)
    {
        let relative = source["path"].as_str().context("随便观来源路径无效")?;
        od_core::config::reject_links(&reference.join(relative))?;
        let bytes = crate::mail::read_within(&reference, relative)?;
        let normalized = std::str::from_utf8(&bytes)?.replace("\r\n", "\n");
        ensure!(
            source["sha256"] == hex::encode(Sha256::digest(normalized.as_bytes())),
            "随便观来源已变化：{relative}"
        );
    }
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    files.insert(
        "pipeline/suibian-temple.json".into(),
        serde_json::to_vec_pretty(&od_core::suibian_boo_box_pipeline::pipeline())?,
    );
    let back = od_core::basic_daily::templates()
        .into_iter()
        .find(|(name, _)| *name == "back")
        .context("缺少固定返回模板")?
        .1;
    files.insert("image/suibian-temple/back.png".into(), back.to_vec());
    for (name, bytes) in od_core::suibian_adventure_pipeline::templates() {
        files.insert(name.into(), bytes.to_vec());
    }
    let lock = crate::models::locked_models()?;
    let ocr = lock
        .models
        .iter()
        .find(|m| m.id == "ppocrv5")
        .context("缺少固定 OCR 模型")?;
    for model in &ocr.files {
        if let Some(destination) = &model.bundle_path {
            let relative = format!("ppocrv5/{}", model.path);
            od_core::config::reject_links(&models.join(&relative))?;
            let bytes = crate::mail::read_within(&models, &relative)?;
            ensure!(
                bytes.len() as u64 == model.bytes
                    && hex::encode(Sha256::digest(&bytes)) == model.sha256,
                "随便观 OCR 模型或字典不匹配"
            );
            files.insert(destination.clone(), bytes);
        }
    }
    let manifest = json!({"schema_version":7,"sdk_version":lock.sdk_version,"base_size":[1920,1080],
        "accepted":false,"capabilities":["auto_manage","adventure","craft","yum_cha_sin","sales","good_goods","boo_box"],"sources":sources,"adventure_sources":adventure_sources,"craft_sources":craft_sources,"yum_sources":yum_sources,"sales_sources":sales_sources,"good_goods_sources":good_goods_sources,"boo_box_sources":boo_box_sources,"image_sources":serde_json::from_str::<Value>(include_str!("../../../resources/suibian-temple/adventure-image-sources.json"))?,
        "files":files.iter().map(|(path,bytes)|json!({"path":path,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>(),
        "limitations":["已迁移托管、游历、制造、饮茶仙、售卖、好物铺与邦巢子流程，完整应用仍在开发；不能作为完整应用启动","固定 1080p 前台键鼠，真实游戏验收单独记录"]});
    files.insert(
        "suibian-temple-manifest.json".into(),
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
                "随便观输出含非受管文件或目录：{relative}"
            );
        }
    }
    for (relative, bytes) in files {
        crate::resources::safe_relative(&relative)?;
        let path = output.join(relative);
        std::fs::create_dir_all(path.parent().context("随便观资源缺少父目录")?)?;
        std::fs::write(path, bytes)?;
    }
    Ok(manifest)
}
