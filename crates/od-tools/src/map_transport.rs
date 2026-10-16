use anyhow::{Context, Result, ensure};
use od_core::map_transport::{AREA_RECT, BACK_RECT, CONFIRM_RECT, POINT_RECT};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

/// 生成固定路线资源；先校验所有输入，再写入受管目录。
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
    let mut sources = Vec::new();
    let mut screens = BTreeMap::new();
    for id in ["map", "normal_world_basic"] {
        let path = format!("assets/game_data/screen_info/{id}.yml");
        od_core::config::reject_links(&reference.join(&path))?;
        let bytes = crate::mail::read_within(&reference, &path)?;
        let screen: crate::resources::LegacyScreen = serde_yaml::from_slice(&bytes)?;
        ensure!(screen.screen_id == id, "地图来源画面 ID 变化");
        sources.push(json!({"path":path,"sha256":hex::encode(Sha256::digest(bytes))}));
        screens.insert(id, screen);
    }
    ensure!(
        !screens["map"].pc_alt && screens["map"].area_list.len() == 4,
        "地图画面定义已变化"
    );
    for (name, rect, text) in [
        ("主区域名称", AREA_RECT, ""),
        ("传送点名称", POINT_RECT, ""),
        ("确认", CONFIRM_RECT, "确认"),
    ] {
        let area = crate::mail::area(&screens["map"], name)?;
        ensure!(
            area.pc_rect == rect
                && area.text == text
                && area.template_id.is_empty()
                && area.color_range.as_ref().is_none_or(Value::is_null),
            "地图区域 {name} 已变化"
        );
    }
    let mut files = BTreeMap::new();
    for (screen, name, rect, folder, template) in [
        ("map", "左上角返回", BACK_RECT, "menu", "back"),
        (
            "normal_world_basic",
            "功能导览",
            [1776, 28, 1869, 117],
            "normal_world",
            "function_menu",
        ),
        (
            "normal_world_basic",
            "预备编队",
            [1225, 44, 1505, 102],
            "normal_world",
            "preset_config",
        ),
    ] {
        let area = crate::mail::area(&screens[screen], name)?;
        ensure!(
            area.pc_rect == rect
                && area.text.is_empty()
                && area.template_sub_dir == folder
                && area.template_id == template
                && area.template_match_threshold.unwrap_or(0.7) == 0.7
                && area.color_range.as_ref().is_none_or(Value::is_null),
            "地图模板区域 {name} 已变化"
        );
        let path = format!("assets/template/{folder}/{template}");
        let mut read_source = |file: &str| -> Result<Vec<u8>> {
            let path = format!("{path}/{file}");
            od_core::config::reject_links(&reference.join(&path))?;
            let bytes = crate::mail::read_within(&reference, &path)?;
            sources.push(json!({"path":path,"sha256":hex::encode(Sha256::digest(&bytes))}));
            Ok(bytes)
        };
        let mut image = image::load_from_memory(&read_source("raw.png")?)?.to_rgb8();
        let mask = image::load_from_memory(&read_source("mask.png")?)?.to_luma8();
        ensure!(
            image.dimensions() == mask.dimensions(),
            "地图模板掩码尺寸不一致"
        );
        ensure!(
            mask.pixels().all(|pixel| matches!(pixel[0], 0 | 255))
                && mask.pixels().any(|pixel| pixel[0] == 255),
            "地图模板掩码无效"
        );
        for (pixel, mask) in image.pixels_mut().zip(mask.pixels()) {
            if mask[0] == 0 {
                *pixel = image::Rgb([0, 255, 0]);
            } else {
                ensure!(pixel.0 != [0, 255, 0], "地图模板包含掩码颜色");
            }
        }
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png)?;
        files.insert(format!("image/map/{template}.png"), png.into_inner());
    }
    files.insert(
        "pipeline/map.json".into(),
        serde_json::to_vec_pretty(&od_core::map_transport::pipeline())?,
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
                "地图 OCR 模型或字典哈希不匹配"
            );
            files.insert(destination.clone(), bytes);
        }
    }
    let manifest = json!({"schema_version":1,"sdk_version":lock.sdk_version,"base_size":[1920,1080],"fixture_only":true,"accepted":false,"route":{"area":od_core::map_transport::AREA_NAME,"point":od_core::map_transport::POINT_NAME},"sources":sources,"files":files.iter().map(|(path,bytes)|json!({"path":path,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>(),"limitations":["从已打开普通地图开始，只处理可见目标，不滚动","固定文字精确匹配及区域裁剪 OCR，未声称旧版全文识别兼容","不接入实机或每日记录；落地画面不能证明具体地点"]});
    files.insert(
        "map-manifest.json".into(),
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
        let path = output.join(relative);
        std::fs::create_dir_all(path.parent().context("地图输出路径无效")?)?;
        std::fs::write(path, bytes)?;
    }
    Ok(manifest)
}
