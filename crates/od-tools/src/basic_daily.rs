use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

const SOURCE_LOCKS: &[(&str, &str)] = &[
    (
        "日常",
        include_str!("../../../resources/daily/source-lock.json"),
    ),
    (
        "后续日常",
        include_str!("../../../resources/daily/remaining-source-lock.json"),
    ),
    (
        "驱动盘",
        include_str!("../../../resources/daily/dismantle-source-lock.json"),
    ),
    (
        "兑换码",
        include_str!("../../../resources/daily/redemption-source-lock.json"),
    ),
    (
        "录像店",
        include_str!("../../../resources/daily/random-play-source-lock.json"),
    ),
    (
        "编队",
        include_str!("../../../resources/daily/team-check-source-lock.json"),
    ),
    (
        "情报板",
        include_str!("../../../resources/daily/intel-board-source-lock.json"),
    ),
    (
        "编队槽位",
        include_str!("../../../resources/daily/team-slots-source-lock.json"),
    ),
    (
        "编队选择",
        include_str!("../../../resources/daily/team-selection-source-lock.json"),
    ),
];

fn uses_team_selection(reference: &Path) -> bool {
    let sources = od_core::team_selection_slots::sources();
    let source = sources
        .as_array()
        .expect("固定选队来源")
        .last()
        .expect("选择源码");
    source_hash(reference, source["path"].as_str().expect("固定路径"))
        .is_ok_and(|hash| source["sha256"] == hash)
}

fn uses_team_slots(reference: &Path) -> bool {
    od_core::team_slots::sources()
        .as_array()
        .expect("固定槽位来源")
        .iter()
        .take(3)
        .any(|s| {
            source_hash(reference, s["path"].as_str().expect("固定槽位路径"))
                .is_ok_and(|hash| s["sha256"] == hash)
        })
}
fn source_hash(reference: &Path, relative: &str) -> Result<String> {
    od_core::config::reject_links(&reference.join(relative))?;
    let bytes = crate::mail::read_within(reference, relative)?;
    Ok(hex::encode(Sha256::digest(
        std::str::from_utf8(&bytes)?
            .replace("\r\n", "\n")
            .as_bytes(),
    )))
}
fn accepted_hash(entry: &Value, actual: &str) -> bool {
    entry["sha256"] == actual
        || entry["compatible_sha256"]
            .as_array()
            .is_some_and(|list| list.iter().any(|h| h == actual))
}
fn checked_sources(reference: &Path, index: usize) -> Result<Value> {
    let (label, lock) = SOURCE_LOCKS[index];
    let mut sources: Value = serde_json::from_str(lock)?;
    for source in sources.as_array_mut().context("来源清单无效")? {
        let relative = source["path"].as_str().context("来源路径无效")?;
        let hash = source_hash(reference, relative)?;
        ensure!(
            accepted_hash(source, &hash),
            "{label}来源已变化，请核对：{relative}（SHA-256：{hash}）"
        );
        let object = source.as_object_mut().context("来源条目无效")?;
        object.remove("compatible_sha256");
        object.insert("sha256".into(), json!(hash));
    }
    Ok(sources)
}
/// 与构建器共用判断，只读报告全部不符来源，不在首个错误处终止。
pub fn source_audit(reference: &Path) -> Result<Value> {
    od_core::config::reject_links(reference)?;
    let reference = reference.canonicalize()?;
    let mut rows: Vec<Value> = Vec::new();
    let mut indices: Vec<_> = (0..7).collect();
    if uses_team_slots(&reference) {
        indices.push(7);
    }
    if uses_team_selection(&reference) {
        indices.push(8);
    }
    for index in indices {
        let (label, lock) = SOURCE_LOCKS[index];
        let sources: Value = serde_json::from_str(lock)?;
        for source in sources.as_array().context("来源清单无效")? {
            let relative = source["path"].as_str().context("来源路径无效")?;
            let hash = source_hash(&reference, relative);
            let row = match hash {
                Ok(hash) => {
                    json!({"source_set":label,"path":relative,"expected_sha256":source["sha256"],"actual_sha256":hash,"accepted":accepted_hash(source, &hash),"primary":source["sha256"]==hash})
                }
                Err(error) => {
                    json!({"source_set":label,"path":relative,"accepted":false,"issue":format!("{error:#}")})
                }
            };
            if let Some(old) = rows
                .iter_mut()
                .find(|row| matches!(label, "编队槽位" | "编队选择") && row["path"] == relative)
            {
                *old = row;
            } else {
                rows.push(row);
            }
        }
    }
    Ok(
        json!({"all_matched":rows.iter().all(|r|r["accepted"]==true),"input_sent":false,"real_game_accepted":false,"sources":rows}),
    )
}

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
    let sources = checked_sources(&reference, 0)?;
    let extra_sources = checked_sources(&reference, 1)?;
    let dismantle_sources = checked_sources(&reference, 2)?;
    let redemption_sources = checked_sources(&reference, 3)?;
    let shop_sources = checked_sources(&reference, 4)?;
    let team_sources = checked_sources(&reference, 5)?;
    let intel_sources = checked_sources(&reference, 6)?;
    let slot_sources = if uses_team_slots(&reference) {
        Some(checked_sources(&reference, 7)?)
    } else {
        None
    };
    let selection_sources = if uses_team_selection(&reference) {
        Some(checked_sources(&reference, 8)?)
    } else {
        None
    };
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
    let mut schema = if json!(parsed) == serde_json::to_value(od_core::random_play::agents())? {
        8
    } else if json!(parsed) == serde_json::to_value(od_core::random_play::agents_for_schema(7))? {
        7
    } else {
        anyhow::bail!("录像店代理人清单不匹配");
    };
    if slot_sources.is_some() {
        ensure!(schema == 8, "槽位扫描需要当前代理人资源");
        schema = 9;
    }
    if selection_sources.is_some() {
        ensure!(schema == 9, "槽位选择需要当前管理页与名册资源");
        schema = 10;
    }
    for (name, raw, mask) in od_core::random_play::source_images() {
        if !od_core::random_play::templates_for_schema(schema)
            .contains_key(&format!("image/random-play/{name}.png"))
        {
            continue;
        }
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
    let mut files = od_core::random_play::templates_for_schema(schema).clone();
    files.extend(od_core::intel_board::templates().clone());
    for (name, bytes) in od_core::basic_daily::templates() {
        files.insert(format!("image/daily/{name}.png"), bytes.to_vec());
    }
    files.insert(
        "pipeline/basic-daily.json".into(),
        serde_json::to_vec_pretty(&od_core::basic_daily::pipeline_for_schema(schema))?,
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
    let mut manifest = json!({"schema_version":schema,"sdk_version":lock.sdk_version,"base_size":[1920,1080],
        "accepted":false,"apps":od_core::basic_daily::RESOURCE_APP_IDS,"sources":sources,
        "remaining_sources":extra_sources,"dismantle_sources":dismantle_sources,"redemption_sources":redemption_sources,"random_play_sources":shop_sources,"team_check_sources":team_sources,"intel_board_sources":intel_sources,
        "files":files.iter().map(|(path,bytes)|json!({"path":path,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>(),
        "limitations":["固定 1080p、前台键鼠和可见地图目标；真实领取后态和版本差异待实机验收","分支测试使用真实已留档画面及合成状态，不代表缺失状态实机通过"]});
    if schema >= 8 {
        manifest["roster_sources"] = od_core::random_play::roster_sources();
    }
    if let Some(sources) = slot_sources {
        manifest["team_slot_sources"] = sources;
    }
    if let Some(sources) = selection_sources {
        manifest["team_selection_sources"] = sources;
    }
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
