use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

/// 先验证所有来源与输出，再生成只读资源；不覆盖目录中的非受管文件。
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
    od_core::config::reject_links(&reference.join(source_path))?;
    let source = crate::mail::read_within(&reference, source_path)?;
    let screen: crate::resources::LegacyScreen = serde_yaml::from_slice(&source)?;
    ensure!(
        screen.screen_id == "coffee_shop"
            && !screen.pc_alt
            && screen.area_list.len() == od_core::coffee::AREAS.len(),
        "咖啡画面区域集合已变化，请重新核对"
    );
    for expected in od_core::coffee::AREAS {
        let area = crate::mail::area(&screen, expected.legacy_name)?;
        ensure!(
            area.pc_rect == expected.rect
                && area.text == expected.text
                && area.lcs_percent == Some(0.5)
                && area.color_range.as_ref().is_none_or(Value::is_null)
                && area.template_id.is_empty(),
            "咖啡区域 {} 已变化，请重新核对",
            expected.legacy_name
        );
    }
    let mut files = BTreeMap::new();
    files.insert(
        "pipeline/coffee-readonly.json".to_owned(),
        serde_json::to_vec_pretty(&od_core::coffee::pipeline())?,
    );
    let lock = crate::models::locked_models()?;
    let ocr = lock
        .models
        .iter()
        .find(|model| model.id == "ppocrv5")
        .context("缺少固定 OCR")?;
    for model in &ocr.files {
        if let Some(destination) = &model.bundle_path {
            let path = format!("ppocrv5/{}", model.path);
            od_core::config::reject_links(&models.join(&path))?;
            let bytes = crate::mail::read_within(&models, &path)?;
            ensure!(
                bytes.len() as u64 == model.bytes
                    && hex::encode(Sha256::digest(&bytes)) == model.sha256,
                "OCR 模型或字典哈希不匹配"
            );
            files.insert(destination.clone(), bytes);
        }
    }
    let manifest = json!({"schema_version":1,"sdk_version":lock.sdk_version,"base_size":[1920,1080],"read_only":true,"accepted":false,"source":{"path":source_path,"sha256":hex::encode(Sha256::digest(source))},"files":files.iter().map(|(path,bytes)|json!({"path":path,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>(),"limitations":["固定文字正则替代旧 LCS，咖啡列表白色遮罩及浓淡选择未迁移","只有对话选项两份真实截图，其他区域尚未验证","不判断已喝过或成功，不写每日记录，不包含传送或真实输入"]});
    files.insert(
        "coffee-manifest.json".into(),
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
                    "输出含非本工具管理的文件：{relative}"
                );
            }
        }
    }
    for (relative, bytes) in files {
        crate::resources::safe_relative(&relative)?;
        let path = output.join(relative);
        std::fs::create_dir_all(path.parent().context("输出路径无效")?)?;
        std::fs::write(path, bytes)?;
    }
    Ok(manifest)
}
