use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

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
    let extra_sources: Value = serde_json::from_str(include_str!(
        "../../../resources/daily/remaining-source-lock.json"
    ))?;
    for source in extra_sources.as_array().context("后续日常来源清单无效")? {
        let relative = source["path"].as_str().context("来源路径无效")?;
        od_core::config::reject_links(&reference.join(relative))?;
        let bytes = crate::mail::read_within(&reference, relative)?;
        let normalized = std::str::from_utf8(&bytes)?.replace("\r\n", "\n");
        ensure!(
            source["sha256"] == hex::encode(Sha256::digest(normalized.as_bytes())),
            "后续日常来源已变化，请重新核对画面：{relative}"
        );
    }
    let dismantle_sources: Value = serde_json::from_str(include_str!(
        "../../../resources/daily/dismantle-source-lock.json"
    ))?;
    for source in dismantle_sources.as_array().context("驱动盘来源清单无效")? {
        let relative = source["path"].as_str().context("来源路径无效")?;
        od_core::config::reject_links(&reference.join(relative))?;
        let bytes = crate::mail::read_within(&reference, relative)?;
        let normalized = std::str::from_utf8(&bytes)?.replace("\r\n", "\n");
        ensure!(
            source["sha256"] == hex::encode(Sha256::digest(normalized.as_bytes())),
            "驱动盘来源已变化，请重新核对画面：{relative}"
        );
    }
    let redemption_sources: Value = serde_json::from_str(include_str!(
        "../../../resources/daily/redemption-source-lock.json"
    ))?;
    for source in redemption_sources
        .as_array()
        .context("兑换码来源清单无效")?
    {
        let relative = source["path"].as_str().context("来源路径无效")?;
        od_core::config::reject_links(&reference.join(relative))?;
        let bytes = crate::mail::read_within(&reference, relative)?;
        let normalized = std::str::from_utf8(&bytes)?.replace("\r\n", "\n");
        ensure!(
            source["sha256"] == hex::encode(Sha256::digest(normalized.as_bytes())),
            "兑换码来源已变化，请重新核对：{relative}"
        );
    }
    let shop_sources: Value = serde_json::from_str(include_str!(
        "../../../resources/daily/random-play-source-lock.json"
    ))?;
    for source in shop_sources.as_array().context("录像店来源清单无效")? {
        let relative = source["path"].as_str().context("录像店来源路径无效")?;
        let bytes = crate::mail::read_within(&reference, relative)?;
        let normalized = std::str::from_utf8(&bytes)?.replace("\r\n", "\n");
        ensure!(
            source["sha256"] == hex::encode(Sha256::digest(normalized.as_bytes())),
            "录像店来源已变化：{relative}"
        );
    }
    let team_sources: Value = serde_json::from_str(include_str!(
        "../../../resources/daily/team-check-source-lock.json"
    ))?;
    for source in team_sources.as_array().context("编队检查来源清单无效")? {
        let relative = source["path"].as_str().context("编队来源路径无效")?;
        let bytes = crate::mail::read_within(&reference, relative)?;
        let normalized = std::str::from_utf8(&bytes)?.replace("\r\n", "\n");
        ensure!(
            source["sha256"] == hex::encode(Sha256::digest(normalized.as_bytes())),
            "编队来源已变化：{relative}"
        );
    }
    let intel_sources: Value = serde_json::from_str(include_str!(
        "../../../resources/daily/intel-board-source-lock.json"
    ))?;
    for source in intel_sources.as_array().context("情报板来源清单无效")? {
        let relative = source["path"].as_str().context("情报板来源路径无效")?;
        let bytes = crate::mail::read_within(&reference, relative)?;
        let normalized = std::str::from_utf8(&bytes)?.replace("\r\n", "\n");
        ensure!(
            source["sha256"] == hex::encode(Sha256::digest(normalized.as_bytes())),
            "情报板来源已变化：{relative}"
        );
    }
    for (name, raw, mask) in od_core::intel_board::source_images() {
        let source = match name {
            "star" => "intel_board/Star",
            "refresh" => "intel_board/refresh",
            _ => "normal_world/function_menu",
        };
        for (file, expected) in [("raw.png", raw), ("mask.png", mask)] {
            ensure!(
                crate::mail::read_within(&reference, &format!("assets/template/{source}/{file}"))?
                    == expected,
                "情报板模板来源不匹配：{source}/{file}"
            );
        }
    }
    let (agents, _) = crate::combat_source::parse_agents(&String::from_utf8(
        crate::mail::read_within(&reference, "src/zzz_od/game_data/agent.py")?,
    )?)?;
    let parsed: Vec<_> = agents
        .into_iter()
        .map(|a| json!({"id":a.id,"name":a.name,"templates":a.templates}))
        .collect();
    ensure!(
        json!(parsed) == serde_json::to_value(od_core::random_play::agents())?,
        "录像店代理人清单不匹配"
    );
    for (name, raw, mask) in od_core::random_play::source_images() {
        let source = if name == "map" {
            "normal_world/map".to_string()
        } else {
            format!("predefined_team/{name}")
        };
        for (file, expected) in [("raw.png", raw), ("mask.png", mask)] {
            ensure!(
                crate::mail::read_within(&reference, &format!("assets/template/{source}/{file}"))?
                    == expected,
                "录像店模板来源不匹配：{source}/{file}"
            );
        }
    }
    let mut files = od_core::random_play::templates().clone();
    files.extend(od_core::intel_board::templates().clone());
    for (name, bytes) in od_core::basic_daily::templates() {
        files.insert(format!("image/daily/{name}.png"), bytes.to_vec());
    }
    files.insert(
        "pipeline/basic-daily.json".into(),
        serde_json::to_vec_pretty(&od_core::basic_daily::current_pipeline())?,
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
    let detector = lock
        .models
        .iter()
        .find(|m| m.id == od_core::intel_board::DETECTOR_ID)
        .context("缺少固定距离检测模型")?;
    for model in &detector.files {
        let destination = model.bundle_path.as_ref().context("距离检测目标路径无效")?;
        ensure!(
            od_core::intel_board::DETECTOR_FILES.contains(&destination.as_str()),
            "距离检测文件未登记"
        );
        let bytes = crate::mail::read_within(&models, &format!("{}/{}", detector.id, model.path))?;
        ensure!(
            bytes.len() as u64 == model.bytes
                && hex::encode(Sha256::digest(&bytes)) == model.sha256,
            "固定距离检测模型或标签不匹配"
        );
        files.insert(destination.clone(), bytes);
    }
    let manifest = json!({"schema_version":7,"sdk_version":lock.sdk_version,"base_size":[1920,1080],
        "accepted":false,"apps":od_core::basic_daily::RESOURCE_APP_IDS,"sources":sources,
        "remaining_sources":extra_sources,"dismantle_sources":dismantle_sources,"redemption_sources":redemption_sources,"random_play_sources":shop_sources,"team_check_sources":team_sources,"intel_board_sources":intel_sources,
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
