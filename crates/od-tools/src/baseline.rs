use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::path::Path;

pub fn prepare(reference: &Path, test_root: &Path) -> Result<()> {
    let mut screenshots = Vec::new();
    for state in ["列表", "列表-有可领", "确认弹窗"] {
        let path = test_root.join(format!("screens/邮件/{state}.webp"));
        ensure!(path.is_file(), "测试仓缺少存档截图 {}", path.display());
        screenshots.push(json!({"path": format!("../../../screens/邮件/{state}.webp"), "sha256": crate::sha256(&path)?, "role": state}));
    }
    let destination = test_root.join("test/zzz_auto/fixtures");
    let mut inputs = Vec::new();
    for (source, name, role) in [
        (
            "assets/template/dodge_audio/template_1.wav",
            "audio/template_1.wav",
            "声音闪避模板，仍缺触发、未触发与阈值附近录音",
        ),
        (
            "config/auto_battle/自动守护.sample.yml",
            "config/auto_battle.yml",
            "战斗 YAML 语法代表样例",
        ),
        (
            "config/lost_void_challenge/parameter_template.sample.yml",
            "config/lost_void.yml",
            "复杂应用配置代表样例",
        ),
        (
            "config/world_patrol_route_list/[空洞]职工社区旧址.sample.yml",
            "config/world_patrol.yml",
            "路线列表代表样例",
        ),
    ] {
        let path = reference.join(source);
        let output = destination.join(name);
        let hash = crate::sha256(&path)?;
        if output.exists() {
            ensure!(
                crate::sha256(&output)? == hash,
                "基线 {} 已有不同内容，请人工处理后再准备",
                output.display()
            );
        } else {
            std::fs::create_dir_all(output.parent().context("基线路径缺少目录")?)?;
            std::fs::copy(&path, &output)?;
        }
        inputs.push(json!({"source": source, "path": name, "sha256": hash, "role": role}));
    }
    let model_source =
        std::fs::read_to_string(reference.join("src/zzz_od/config/model_config.py"))?;
    let pattern = regex::Regex::new(r"(?m)^_DEFAULT_(\w+)\s*=\s*'([^']+)'")?;
    let model_lock = crate::models::locked_models()?;
    let test_directory = test_root.canonicalize()?;
    let model_cache = test_directory
        .parent()
        .context("测试仓缺少父目录")?
        .join("sdk/models");
    let models: Vec<_> = pattern.captures_iter(&model_source).map(|capture| {
        let present = model_lock.models.iter().find(|model| model.id == capture[2])
            .is_some_and(|model| crate::models::cached_files_valid(&model_cache, model));
        json!({"role":capture[1].to_lowercase(),"name":&capture[2],"present":present,"accepted":false})
    }).collect();
    let ocr_present = model_lock
        .models
        .iter()
        .find(|model| model.id == "ppocrv5")
        .is_some_and(|model| crate::models::cached_files_valid(&model_cache, model));
    let mut missing = vec![
        "完整旧识别输出及预处理对照",
        "红光 / 黄光、检测正样本和阈值附近语料",
        "声音边界录音",
        "同机完整输入与性能基线",
    ];
    if !ocr_present {
        missing.push("旧 OCR 模型与字典");
    }
    if models.iter().any(|model| model["present"] != true) {
        missing.push("闪光与 YOLO ONNX 模型");
    }
    crate::write_json(
        &destination.join("baseline.json"),
        &json!({
            "schema_version": 1, "base_size": [1920,1080], "random_seed": 42,
            "clock": {"calendar_time":"2026-10-12T00:00:00+08:00", "monotonic_ms":0},
            "screenshots": screenshots, "inputs": inputs, "models": models,
            "ocr": {"source":"src/one_dragon/base/matcher/ocr/onnx_ocr_matcher.py", "present":ocr_present, "accepted":false},
            "model_lock": {"path":"models.lock.json", "cache":"sdk/models", "availability":"缓存文件按锁文件校验；不代表旧行为已经验收"},
            "legacy_observations": {"source":"docs/game/screens/邮件.md", "title":"邮件", "claim_all":"全部领取", "back_box":[92,23,48,57], "kind":"文档快照，尚无同机旧实现性能结果"},
            "performance": {"warmup":5,"samples":30,"quantiles":[0.5,0.95],"legacy_measurements":null},
            "acceptance":"pending", "missing":missing
        }),
    )?;
    println!("已固定 3 张邮件画面和 4 份资源 / 配置，模型可用性与验收缺口见 baseline.json");
    Ok(())
}
