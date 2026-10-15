use crate::resources::{LegacyArea, LegacyScreen};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn resolved_output(path: &Path) -> Result<PathBuf> {
    let mut ancestor = std::path::absolute(path)?;
    let mut missing = Vec::new();
    while !ancestor.exists() {
        missing.push(ancestor.file_name().context("资源输出路径无效")?.to_owned());
        ensure!(ancestor.pop(), "资源输出缺少祖先目录");
    }
    let mut result = ancestor.canonicalize()?;
    for part in missing.into_iter().rev() {
        result.push(part);
    }
    Ok(result)
}

fn read_within(root: &Path, relative: &str) -> Result<Vec<u8>> {
    crate::resources::safe_relative(relative)?;
    let source = root.join(relative).canonicalize()?;
    ensure!(source.starts_with(root), "来源文件指向来源目录之外");
    Ok(std::fs::read(source)?)
}

fn area<'a>(screen: &'a LegacyScreen, name: &str) -> Result<&'a LegacyArea> {
    let matches: Vec<_> = screen
        .area_list
        .iter()
        .filter(|area| area.area_name == name)
        .collect();
    ensure!(
        matches.len() == 1,
        "区域 {} / {name} 缺失或重复",
        screen.screen_id
    );
    Ok(matches[0])
}

// 只转换首个业务需要的区域；不把尚未对齐的通用 LCS / 色彩过滤静默转换为可用节点。
pub fn build_bundle(reference: &Path, models: &Path, output: &Path) -> Result<Value> {
    let reference = reference.canonicalize()?;
    let models = models.canonicalize()?;
    let output = resolved_output(output)?;
    for source in [&reference, &models] {
        ensure!(
            !output.starts_with(source) && !source.starts_with(&output),
            "资源来源与输出不能重叠"
        );
    }
    let mut sources = Vec::new();
    let mut screens = BTreeMap::new();
    for id in ["email", "menu", "normal_world_basic"] {
        let path = format!("assets/game_data/screen_info/{id}.yml");
        let bytes = read_within(&reference, &path)?;
        sources.push(json!({"path":path,"sha256":hex::encode(Sha256::digest(&bytes))}));
        let screen: LegacyScreen = serde_yaml::from_slice(&bytes)?;
        ensure!(screen.screen_id == id, "画面 ID 与文件名不一致");
        screens.insert(id, screen);
    }
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    let mut nodes = serde_json::Map::new();
    let mut conversions = Vec::new();
    for (screen_id, name, alias) in [
        ("email", "标题-邮件", "Title"),
        ("email", "全部领取", "ClaimText"),
        ("email", "确认", "ConfirmText"),
        ("email", "按钮-返回", "Back"),
        ("menu", "按钮-返回", "MenuBack"),
        ("menu", "菜单-动态壁纸", "MenuWallpaper"),
        ("menu", "底部-邮件", "MenuMail"),
        ("normal_world_basic", "功能导览", "WorldFunction"),
        ("normal_world_basic", "预备编队", "WorldPreset"),
        ("normal_world_basic", "按钮-菜单", "WorldMenu"),
    ] {
        let old = area(&screens[screen_id], name)?;
        ensure!(old.color_range.is_none(), "首批区域不支持隐式转换颜色过滤");
        let roi = od_core::Roi::from_legacy(old.pc_rect).map_err(anyhow::Error::msg)?;
        let mut node = json!({"roi":roi, "action":"DoNothing", "pre_delay":0, "post_delay":0, "rate_limit":25});
        let mut inputs = Vec::new();
        if !old.text.is_empty() {
            node["recognition"] = json!("OCR");
            node["model"] = json!("ppocrv5");
            node["expected"] = json!([format!("^{}$", regex::escape(&old.text))]);
            node["threshold"] = json!(0.6);
        } else if !old.template_id.is_empty() {
            crate::resources::safe_relative(&old.template_sub_dir)?;
            crate::resources::safe_relative(&old.template_id)?;
            let folder = format!(
                "assets/template/{}/{}",
                old.template_sub_dir, old.template_id
            );
            let raw = read_within(&reference, &format!("{folder}/raw.png"))?;
            let mut image = image::load_from_memory(&raw)?.to_rgb8();
            inputs.push(json!({"path":format!("{folder}/raw.png"),"sha256":hex::encode(Sha256::digest(&raw))}));
            let masked = reference.join(&folder).join("mask.png").exists();
            if masked {
                let mask = read_within(&reference, &format!("{folder}/mask.png"))?;
                inputs.push(json!({"path":format!("{folder}/mask.png"),"sha256":hex::encode(Sha256::digest(&mask))}));
                let mask = image::load_from_memory(&mask)?.to_luma8();
                ensure!(
                    mask.dimensions() == image.dimensions(),
                    "模板与掩码尺寸不一致"
                );
                ensure!(
                    mask.pixels().all(|pixel| matches!(pixel[0], 0 | 255)),
                    "模板掩码必须为二值图"
                );
                ensure!(
                    mask.pixels().any(|pixel| pixel[0] == 255),
                    "模板掩码不能全部为空"
                );
                for (pixel, mask) in image.pixels_mut().zip(mask.pixels()) {
                    if mask[0] == 0 {
                        *pixel = image::Rgb([0, 255, 0]);
                    } else {
                        ensure!(pixel.0 != [0, 255, 0], "保留区域含绿色掩码颜色");
                    }
                }
            }
            let image_path = format!("mail/{}/{}.png", old.template_sub_dir, old.template_id);
            let mut encoded = std::io::Cursor::new(Vec::new());
            image.write_to(&mut encoded, image::ImageFormat::Png)?;
            let bytes = encoded.into_inner();
            if let Some(previous) = files.insert(format!("image/{image_path}"), bytes.clone()) {
                ensure!(previous == bytes, "同名模板转换结果不一致");
            }
            node["recognition"] = json!("TemplateMatch");
            node["template"] = json!([image_path]);
            node["threshold"] = json!(old.template_match_threshold.unwrap_or(0.7));
            node["method"] = json!(5);
            node["green_mask"] = json!(masked);
        } else {
            ensure!(
                alias == "WorldMenu",
                "只有明确声明的点击区域可以转换为 DirectHit"
            );
            node["recognition"] = json!("DirectHit");
        }
        conversions.push(json!({"source_screen":screen_id,"area":name,"node":format!("Mail.Area.{alias}"),"legacy_rect":old.pc_rect,"roi":roi,"legacy_lcs":old.lcs_percent,"matching":"exact_regex_or_masked_template","inputs":inputs}));
        nodes.insert(format!("Mail.Area.{alias}"), node);
    }
    files.insert(
        "pipeline/mail-areas.json".into(),
        serde_json::to_vec_pretty(&nodes)?,
    );
    let mut pipeline: Value =
        serde_json::from_str(include_str!("../../../resources/mail/pipeline.json"))?;
    for (name, node) in pipeline.as_object_mut().context("邮件流程不是对象")? {
        if name.starts_with('$') {
            continue;
        }
        // Maa 的字符串 ROI / target 指已识别结果，不是节点配置；此处展开为固定 1080p 区域。
        for field in ["roi", "target"] {
            if let Some(alias) = node[field]
                .as_str()
                .filter(|alias| alias.starts_with("Mail.Area."))
            {
                let roi = nodes
                    .get(alias)
                    .and_then(|node| node.get("roi"))
                    .context("邮件坐标别名缺失")?
                    .clone();
                node[field] = roi;
            }
        }
        for (key, value) in [
            ("recognition", json!("DirectHit")),
            ("action", json!("DoNothing")),
            ("pre_delay", json!(0)),
            ("post_delay", json!(0)),
            ("rate_limit", json!(25)),
            ("timeout", json!(10_000)),
            ("next", json!([])),
            ("on_error", json!([])),
            // 固定 SDK 仅为非 null focus 节点发送 context 进度，不开启全局调试。
            ("focus", json!({})),
        ] {
            node.as_object_mut()
                .context("邮件节点不是对象")?
                .entry(key)
                .or_insert(value);
        }
    }
    files.insert(
        "pipeline/mail.json".into(),
        serde_json::to_vec_pretty(&pipeline)?,
    );
    let lock = crate::models::locked_models()?;
    let ocr = lock
        .models
        .iter()
        .find(|model| model.id == "ppocrv5")
        .context("缺少固定 OCR 模型")?;
    for model in &ocr.files {
        if let Some(destination) = &model.bundle_path {
            let bytes = read_within(&models, &format!("ppocrv5/{}", model.path))?;
            ensure!(
                bytes.len() as u64 == model.bytes
                    && hex::encode(Sha256::digest(&bytes)) == model.sha256,
                "OCR 模型或字典哈希不匹配"
            );
            files.insert(destination.clone(), bytes);
        }
    }
    let manifest = json!({"schema_version":1,"sdk_version":lock.sdk_version,"base_size":[1920,1080],"color_method":"RGB (Maa method=4)","pipeline_source_sha256":hex::encode(Sha256::digest(include_bytes!("../../../resources/mail/pipeline.json"))),"sources":sources,"areas":conversions,"files":files.iter().map(|(path,bytes)|json!({"path":path,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>(),"accepted":false,"limitations":["精确正则替代 LCS，仅限已验证文字","RGB 黄色与灰色按钮条件需实机验收","确定弹窗变体暂无真实截图","桌面前台键鼠试运行已接入，真实游戏验收待完成"]});
    files.insert(
        "mail-manifest.json".into(),
        serde_json::to_vec_pretty(&manifest)?,
    );
    if output.exists() {
        for entry in walkdir::WalkDir::new(&output).min_depth(1) {
            let entry = entry?;
            ensure!(!entry.file_type().is_symlink(), "输出包不能含符号链接");
            if entry.file_type().is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(&output)?
                    .to_string_lossy()
                    .replace('\\', "/");
                ensure!(
                    files.contains_key(&relative),
                    "输出目录含非本工具管理的文件：{relative}"
                );
            }
        }
    }
    // 来源、输出及全部模型验证完成后才写入，不删除用户目录。
    for (relative, bytes) in files {
        crate::resources::safe_relative(&relative)?;
        let destination = output.join(relative);
        std::fs::create_dir_all(destination.parent().context("输出缺少父目录")?)?;
        std::fs::write(destination, bytes)?;
    }
    Ok(manifest)
}
