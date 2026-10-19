use anyhow::{Context, Result, ensure};
use od_core::engagement_reward::{
    AREAS, BUNDLE_SCHEMA_VERSION, DAILY_TAB, DAILY_TAB_RGB_RANGE, DAILY_TAB_SOURCE, MASK_SHA256,
    RAW_SHA256,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

/// 先验证固定来源和输出，再生成没有输入动作的活跃度奖励资源。
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
    let source_path = "assets/game_data/screen_info/compendium.yml";
    od_core::config::reject_links(&reference.join(source_path))?;
    let source = crate::mail::read_within(&reference, source_path)?;
    let screen: crate::resources::LegacyScreen = serde_yaml::from_slice(&source)?;
    ensure!(
        screen.screen_id == "compendium" && screen.screen_name == "快捷手册" && !screen.pc_alt,
        "活跃度奖励来源画面已变化，请重新核对"
    );
    let mut conversions = Vec::new();
    for expected in AREAS {
        let actual = crate::mail::area(&screen, expected.legacy_name)?;
        ensure!(
            actual.pc_rect == expected.rect
                && actual.text == expected.text
                && actual.lcs_percent == Some(0.5)
                && actual.color_range.as_ref().is_none_or(Value::is_null)
                && !actual.id_mark
                && actual.goto_list.is_empty()
                && if expected.text.is_empty() {
                    actual.template_sub_dir == "compendium"
                        && actual.template_id == "completed"
                        && actual.template_match_threshold == Some(0.7)
                } else {
                    actual.template_sub_dir.is_empty() && actual.template_id.is_empty()
                },
            "活跃度奖励区域已变化：{}",
            expected.legacy_name
        );
        let [x, y, x2, y2] = expected.rect;
        conversions.push(json!({
            "source":source_path,"area":expected.legacy_name,"node":format!("EngagementReward.ReadOnly.{}",expected.alias),
            "legacy_rect":expected.rect,"roi":[x,y,x2-x,y2-y],
            "matching":if expected.text.is_empty(){"binary_mask_to_green_mask"}else{"exact_regex_not_legacy_lcs"}
        }));
    }
    let mut sources =
        vec![json!({"path":source_path,"sha256":hex::encode(Sha256::digest(&source))})];
    od_core::config::reject_links(&reference.join(DAILY_TAB_SOURCE))?;
    let daily_source = crate::mail::read_within(&reference, DAILY_TAB_SOURCE)?;
    let daily_screen: crate::resources::LegacyScreen = serde_yaml::from_slice(&daily_source)?;
    ensure!(
        daily_screen.screen_id == "compendium_errands"
            && daily_screen.screen_name == "快捷手册-日常"
            && !daily_screen.pc_alt,
        "日常页签来源画面已变化，请重新核对"
    );
    let daily_tab = crate::mail::area(&daily_screen, DAILY_TAB.legacy_name)?;
    ensure!(
        daily_tab.pc_rect == DAILY_TAB.rect
            && daily_tab.text == DAILY_TAB.text
            && daily_tab.lcs_percent == Some(1.0)
            && daily_tab.color_range == Some(json!(DAILY_TAB_RGB_RANGE))
            && daily_tab.id_mark
            && daily_tab.goto_list == ["快捷手册-日常"]
            && daily_tab.template_sub_dir.is_empty()
            && daily_tab.template_id.is_empty()
            && daily_tab.template_match_threshold == Some(0.7),
        "日常页签区域、颜色或识别规则已变化，请重新核对"
    );
    let [x, y, x2, y2] = DAILY_TAB.rect;
    conversions.push(json!({
        "source":DAILY_TAB_SOURCE,"area":DAILY_TAB.legacy_name,
        "node":format!("EngagementReward.ReadOnly.{}",DAILY_TAB.alias),
        "legacy_rect":DAILY_TAB.rect,"roi":[x,y,x2-x,y2-y],
        "color_range":DAILY_TAB_RGB_RANGE,"matching":"rgb_color_filter_and_exact_regex_not_legacy_lcs"
    }));
    sources
        .push(json!({"path":DAILY_TAB_SOURCE,"sha256":hex::encode(Sha256::digest(&daily_source))}));
    let mut inputs = Vec::new();
    for (name, pinned) in [("raw.png", RAW_SHA256), ("mask.png", MASK_SHA256)] {
        let relative = format!("assets/template/compendium/completed/{name}");
        od_core::config::reject_links(&reference.join(&relative))?;
        let bytes = crate::mail::read_within(&reference, &relative)?;
        ensure!(
            hex::encode(Sha256::digest(&bytes)) == pinned,
            "活跃度奖励模板或掩码已变化：{name}"
        );
        sources.push(json!({"path":relative,"sha256":pinned}));
        inputs.push(bytes);
    }
    let mut template = image::load_from_memory(&inputs[0])?.to_rgb8();
    let mask = image::load_from_memory(&inputs[1])?.to_luma8();
    ensure!(
        template.dimensions() == (40, 40)
            && template.dimensions() == mask.dimensions()
            && mask.pixels().all(|pixel| matches!(pixel[0], 0 | 255))
            && mask.pixels().any(|pixel| pixel[0] == 255),
        "活跃度奖励模板尺寸或二值掩码无效"
    );
    for (pixel, mask) in template.pixels_mut().zip(mask.pixels()) {
        if mask[0] == 0 {
            *pixel = image::Rgb([0, 255, 0]);
        } else {
            ensure!(pixel.0 != [0, 255, 0], "保留区域不能使用绿色掩码颜色");
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    template.write_to(&mut png, image::ImageFormat::Png)?;
    ensure!(
        hex::encode(Sha256::digest(png.get_ref())) == od_core::engagement_reward::TEMPLATE_SHA256,
        "活跃度奖励模板转换结果已变化，请重新核对"
    );
    let mut files = BTreeMap::from([
        (
            "image/engagement-reward/completed.png".to_owned(),
            png.into_inner(),
        ),
        (
            "pipeline/engagement-reward-readonly.json".to_owned(),
            serde_json::to_vec_pretty(&od_core::engagement_reward::pipeline())?,
        ),
    ]);
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
                "活跃度奖励 OCR 模型或字典不匹配"
            );
            files.insert(destination.clone(), bytes);
        }
    }
    let manifest = json!({
        "schema_version":BUNDLE_SCHEMA_VERSION,"sdk_version":lock.sdk_version,"base_size":[1920,1080],
        "read_only":true,"accepted":false,"sources":sources,"areas":conversions,
        "files":files.iter().map(|(path,bytes)|json!({"path":path,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>(),
        "limitations":["固定 OCR 精确正则与旧 LCS 包装器不等价；日常选中态要求黄色页签和标题同时命中",
        "真实素材只有已满日常页；领取确认、奖励预览、未满和可领取状态待补",
        "只报告可见文字及标记，不判断本次领取成功，不写每日记录或投递输入"]
    });
    files.insert(
        "engagement-reward-manifest.json".into(),
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
            } else {
                let relative = entry
                    .path()
                    .strip_prefix(&output)?
                    .to_string_lossy()
                    .replace('\\', "/");
                ensure!(
                    files
                        .keys()
                        .any(|name| name.starts_with(&format!("{relative}/"))),
                    "输出含非本工具管理的目录：{relative}"
                );
            }
        }
    }
    for (relative, bytes) in files {
        crate::resources::safe_relative(&relative)?;
        let path = output.join(relative);
        std::fs::create_dir_all(path.parent().context("活跃度奖励输出路径无效")?)?;
        std::fs::write(path, bytes)?;
    }
    Ok(manifest)
}
