use anyhow::{Context, Result, ensure};
use od_core::{
    combat_vision::{Template, VisionSpec},
    conditional::Script,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

pub fn build_bundle(reference: &Path, models: &Path, output: &Path) -> Result<Value> {
    od_core::config::reject_links(reference)?;
    let reference = reference.canonicalize()?;
    od_core::config::reject_links(models)?;
    let models = models.canonicalize()?;
    od_core::config::reject_links(output)?;
    let source_path = "src/zzz_od/game_data/agent.py";
    let source = crate::mail::read_within(&reference, source_path)?;
    let (agents, common) = crate::combat_source::parse_agents(std::str::from_utf8(&source)?)?;
    let screen: crate::resources::LegacyScreen = serde_yaml::from_slice(
        &crate::mail::read_within(&reference, "assets/game_data/screen_info/battle.yml")?,
    )?;
    ensure!(
        screen.screen_id == "battle" && screen.screen_name == "战斗画面",
        "战斗画面定义不匹配"
    );
    let mut areas = BTreeMap::new();
    for area in screen.area_list {
        od_core::Roi::from_legacy(area.pc_rect)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("战斗区域 {}", area.area_name))?;
        ensure!(
            areas.insert(area.area_name, area.pc_rect).is_none(),
            "战斗区域重名"
        );
    }
    let mut spec = VisionSpec {
        schema: 1,
        agents,
        common,
        areas,
        templates: BTreeMap::new(),
    };
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    for category in ["battle", "agent_state", "target_state"] {
        for entry in std::fs::read_dir(reference.join("assets/template").join(category))? {
            let path = entry?.path();
            od_core::config::reject_links(&path)?;
            if !path.is_dir() || !path.join("raw.png").is_file() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .context("模板名不是 UTF-8")?;
            crate::resources::safe_relative(name)?;
            let key = format!("{category}/{name}");
            let relative = format!("assets/template/{key}");
            let raw = crate::mail::read_within(&reference, &format!("{relative}/raw.png"))?;
            let mut image = image::load_from_memory(&raw)?.to_rgb8();
            ensure!(
                image.width() > 0
                    && image.height() > 0
                    && image.width() <= 1920
                    && image.height() <= 1080,
                "模板尺寸无效 {key}"
            );
            let mut mask_path = None;
            if path.join("mask.png").is_file() {
                let mask = crate::mail::read_within(&reference, &format!("{relative}/mask.png"))?;
                let decoded = image::load_from_memory(&mask)?.to_luma8();
                ensure!(
                    decoded.dimensions() == image.dimensions(),
                    "模板掩码尺寸不匹配 {key}"
                );
                for (pixel, mask) in image.pixels_mut().zip(decoded.pixels()) {
                    if mask[0] == 0 {
                        *pixel = image::Rgb([0, 255, 0]);
                    } else {
                        ensure!(pixel.0 != [0, 255, 0], "有效模板像素与绿色掩码冲突 {key}");
                    }
                }
                let destination = format!("mask/{key}.png");
                files.insert(destination.clone(), mask);
                mask_path = Some(destination);
            }
            let rect = if path.join("config.yml").is_file() {
                let config: Value = serde_yaml::from_slice(&crate::mail::read_within(
                    &reference,
                    &format!("{relative}/config.yml"),
                )?)?;
                let points = config["point_list"].as_array().context("模板点列表缺失")?;
                ensure!(points.len() >= 2, "模板点列表不足 {key}");
                let parsed: Vec<Vec<i32>> = points
                    .iter()
                    .map(|p| {
                        p.as_str()
                            .context("模板点不是字符串")?
                            .split(',')
                            .map(|v| v.trim().parse().map_err(Into::into))
                            .collect::<Result<_>>()
                    })
                    .collect::<Result<_>>()?;
                ensure!(parsed.iter().all(|p| p.len() == 2), "模板点维度无效");
                let rect = if config["template_shape"] == "circle" {
                    ensure!(parsed.len() == 2, "圆形模板需要圆心与半径点");
                    let radius = ((parsed[0][0] - parsed[1][0]) as f64)
                        .hypot((parsed[0][1] - parsed[1][1]) as f64)
                        as i32;
                    [
                        parsed[0][0] - radius,
                        parsed[0][1] - radius,
                        parsed[0][0] + radius,
                        parsed[0][1] + radius,
                    ]
                } else {
                    [
                        parsed.iter().map(|p| p[0]).min().unwrap(),
                        parsed.iter().map(|p| p[1]).min().unwrap(),
                        parsed.iter().map(|p| p[0]).max().unwrap(),
                        parsed.iter().map(|p| p[1]).max().unwrap(),
                    ]
                };
                od_core::Roi::from_legacy(rect)
                    .map_err(anyhow::Error::msg)
                    .with_context(|| format!("模板 {key}"))?;
                Some(rect)
            } else {
                None
            };
            let image_path = format!("image/{key}.png");
            let mut encoded = std::io::Cursor::new(Vec::new());
            image.write_to(&mut encoded, image::ImageFormat::Png)?;
            files.insert(image_path.clone(), encoded.into_inner());
            spec.templates.insert(
                key,
                Template {
                    image: image_path,
                    mask: mask_path,
                    rect,
                },
            );
        }
    }
    let mut script_names = Vec::new();
    let mut dodge_scripts = Vec::new();
    for entry in std::fs::read_dir(reference.join("config/dodge"))? {
        let path = entry?.path();
        if let Some(name) = path
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_suffix(".sample.yml"))
        {
            Script::load_entry(&reference.join("config"), "dodge", name, false)?;
            dodge_scripts.push(name.to_string());
        }
    }
    for entry in std::fs::read_dir(reference.join("config/auto_battle"))? {
        let path = entry?.path();
        let Some(name) = path
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_suffix(".sample.yml"))
        else {
            continue;
        };
        Script::load(&reference.join("config"), name)?;
        script_names.push(name.to_string());
    }
    for category in [
        "auto_battle",
        "dodge",
        "auto_battle_operation",
        "auto_battle_state_handler",
    ] {
        for entry in std::fs::read_dir(reference.join("config").join(category))? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.ends_with(".sample.yml") || name.ends_with(".merged.yml") {
                files.insert(
                    format!("scripts/{category}/{name}"),
                    crate::mail::read_within(&reference, &format!("config/{category}/{name}"))?,
                );
            }
        }
    }
    let dodge = "auto_lock_interval: 0\nauto_turn_interval: 0\nscenes:\n  - triggers: [闪避识别-红光, 闪避识别-黄光, 闪避识别-声音]\n    interval: 0.3\n    priority: 99\n    handlers:\n      - states: ''\n        operations:\n          - op_name: 按键-闪避\n";
    files.insert(
        "scripts/auto_battle/闪避助手.yml".into(),
        dodge.as_bytes().to_vec(),
    );
    script_names.push("闪避助手".into());
    files.insert("vision.json".into(), serde_json::to_vec_pretty(&spec)?);
    files.insert(
        "pipeline/combat.json".into(),
        br#"{"Combat.NoInput":{"recognition":"DirectHit","action":"DoNothing"}}"#.to_vec(),
    );
    let wav = crate::mail::read_within(&reference, "assets/template/dodge_audio/template_1.wav")?;
    od_core::audio_dodge::read_wav(&wav)?;
    files.insert("audio/template_1.wav".into(), wav);
    for model in crate::models::locked_models()?.models {
        if model.role != "ocr" && model.role != "classify" {
            continue;
        }
        for source in model.files {
            if let Some(destination) = source.bundle_path {
                let bytes =
                    crate::mail::read_within(&models, &format!("{}/{}", model.id, source.path))?;
                ensure!(
                    bytes.len() as u64 == source.bytes
                        && hex::encode(Sha256::digest(&bytes)) == source.sha256,
                    "固定战斗模型不匹配"
                );
                files.insert(destination, bytes);
            }
        }
    }
    script_names.sort();
    dodge_scripts.sort();
    let manifest = json!({"schema":1,"sdk_version":"5.14.3","accepted":false,"base_size":[1920,1080],"scripts":script_names,"dodge_scripts":dodge_scripts,"agent_source":{"path":source_path,"sha256":hex::encode(Sha256::digest(source))},"agents":spec.agents.len(),"templates":spec.templates.len(),"files":files.iter().map(|(path,bytes)|json!({"path":path,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>()});
    files.insert(
        "combat-manifest.json".into(),
        serde_json::to_vec_pretty(&manifest)?,
    );
    if output.exists() {
        let resolved = output.canonicalize()?;
        ensure!(
            !resolved.starts_with(&reference) && !reference.starts_with(&resolved),
            "战斗输出与参考目录重叠"
        );
        for entry in walkdir::WalkDir::new(output).min_depth(1) {
            let entry = entry?;
            od_core::config::reject_links(entry.path())?;
            if entry.file_type().is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(output)?
                    .to_string_lossy()
                    .replace('\\', "/");
                ensure!(
                    files.contains_key(&relative),
                    "输出含非本工具文件 {relative}"
                );
            }
        }
    } else {
        let mut ancestor = output.to_path_buf();
        while !ancestor.exists() {
            ensure!(ancestor.pop(), "输出目录无有效父目录");
        }
        ensure!(
            !ancestor.canonicalize()?.starts_with(&reference),
            "不能在参考目录生成战斗包"
        );
    }
    for (relative, bytes) in files {
        crate::resources::safe_relative(&relative)?;
        let target = output.join(relative);
        od_core::config::reject_links(&target)?;
        std::fs::create_dir_all(target.parent().context("输出目录无效")?)?;
        std::fs::write(target, bytes)?;
    }
    Ok(manifest)
}
