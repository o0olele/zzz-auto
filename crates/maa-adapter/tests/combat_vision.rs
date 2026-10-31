//! 固定截图对照 Maa 队列与直接 OpenCV；不连接游戏、不发送输入。
use maa_adapter::{
    InferenceDevice, NativeSession, combat_vision::CombatBundle, combat_vision::CombatRecognizer,
};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};

#[test]
#[ignore = "需要固定 Maa SDK、生成后的战斗资源和战斗画面留档"]
fn combat_reader_accepts_both_loaded_bundle_paths_and_rejects_unrelated_resources()
-> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/combat-startup-logs"),
    )?;
    let bundle = CombatBundle::load(&root.join("resources/generated/combat"))?;
    let image = root.join("zzz-od-test/screens/战斗画面/默认.webp");
    let native = NativeSession::for_fixture(&image, &bundle.root, InferenceDevice::Cpu)?;
    let recognizer = CombatRecognizer::new(bundle);
    native.combat_reader(&recognizer)?;

    let mut native =
        NativeSession::for_fixture(&image, &root.join("resources"), InferenceDevice::Cpu)?;
    let error = native
        .combat_reader(&recognizer)
        .err()
        .expect("通用资源会话不能冒充已载入战斗包");
    assert!(error.to_string().contains("资源不匹配"));
    let recognizer = native.load_combat_bundle(&recognizer.bundle.root)?;
    native.combat_reader(&recognizer)?;
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、生成后的战斗资源和战斗画面留档"]
fn cropped_templates_match_the_original_full_frame_roi() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/combat-crop-logs"),
    )?;
    let bundle = CombatBundle::load(&root.join("resources/generated/combat"))?;
    let native = NativeSession::for_fixture(
        &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
        &bundle.root,
        InferenceDevice::Cpu,
    )?;
    let frame = native.combat_frame(&AtomicBool::new(false))?.unwrap();
    for (key, area) in [
        ("battle/btn_normal_attack", "按键-普通攻击"),
        ("battle/avatar_1_yeshunguang", "头像-3-1"),
        ("battle/avatar_1_lucy", "头像-3-1"),
        ("battle/avatar_2_sunna", "头像-3-2"),
    ] {
        let template = &bundle.spec.templates[key];
        let rect = bundle.spec.areas[area];
        let mut params = serde_json::json!({"template":template.image.strip_prefix("image/").unwrap(),
            "method":5,"threshold":0.8,"green_mask":template.mask.is_some(),"order_by":"Score",
            "roi":od_core::Roi::from_legacy(rect).unwrap()});
        let full = native
            .recognize_prepared(frame.recognition_image(), "TemplateMatch", &params)?
            .detail;
        params["roi"] = serde_json::json!([0, 0, rect[2] - rect[0], rect[3] - rect[1]]);
        let crop = frame.template_crop(rect)?;
        let cropped = native
            .recognize_prepared(&crop, "TemplateMatch", &params)?
            .detail;
        assert_eq!(full["hit"], cropped["hit"], "{key}");
        let a = full["detail"]["all"][0]["score"].as_f64().unwrap();
        let b = cropped["detail"]["all"][0]["score"].as_f64().unwrap();
        assert!((a - b).abs() <= 0.00001, "{key}: 全帧 {a}, 裁图 {b}");
        let direct = bundle
            .template(key)
            .unwrap()
            .score(&frame, rect)?
            .unwrap_or(0.0);
        assert!(
            (a - direct).abs() <= 0.00001,
            "{key}: Maa {a}, 直接 OpenCV {direct}"
        );
        assert_eq!(full["hit"], direct > 0.8, "{key}: 阈值命中须一致");
    }
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、生成后的战斗资源和战斗画面留档"]
fn direct_opencv_matches_all_combat_templates_and_roi_boundaries() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/combat-direct-template-logs"),
    )?;
    let bundle = CombatBundle::load(&root.join("resources/generated/combat"))?;
    let native = NativeSession::for_fixture(
        &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
        &bundle.root,
        InferenceDevice::Cpu,
    )?;
    let frame = native.combat_frame(&AtomicBool::new(false))?.unwrap();
    let mut native_us = 0;
    let mut direct_us = 0;
    let mut empty_results = 0;
    for (key, definition) in &bundle.spec.templates {
        let (width, height) = image::image_dimensions(bundle.root.join(&definition.image))?;
        let (rw, rh) = ((width + 4).min(1920), (height + 3).min(1080));
        // 非零起点、跨整帧步长、紧贴右下边界，检查借用 ROI 与裁图结果一致。
        let rect = [(1920 - rw) as i32, (1080 - rh) as i32, 1920, 1080];
        let crop = frame.template_crop(rect)?;
        let params = serde_json::json!({
            "template":definition.image.strip_prefix("image/").unwrap(),
            "method":5,"threshold":0.8,"green_mask":definition.mask.is_some(),
            "order_by":"Score","roi":[0,0,rw,rh],
        });
        // 两条路径都排除首次模板载入/初始化，比较相同匹配的热态开销。
        native.recognize_prepared(&crop, "TemplateMatch", &params)?;
        let template = bundle.template(key).unwrap();
        template.score(&frame, rect)?;
        let previous = native.recognize_prepared(&crop, "TemplateMatch", &params)?;
        native_us += previous.timing.total_us;
        let started = std::time::Instant::now();
        let direct = template.score(&frame, rect)?.unwrap_or(0.0);
        direct_us += started.elapsed().as_micros() as u64;
        if let Some(score) = previous.detail["detail"]["all"][0]["score"].as_f64() {
            assert!(
                (score - direct).abs() <= 0.00002,
                "{key}: Maa {score}, 直接 {direct}"
            );
        } else {
            // 纯色画面可能全为 NaN；NMS 也会移除低于 0.1f 的候选。
            assert_eq!(direct, 0.0, "{key}: 空 Maa 结果不能变成直接匹配命中");
            empty_results += 1;
        }
        assert_eq!(previous.detail["hit"], direct > 0.8, "{key}");
    }
    let attack = bundle.template("battle/btn_normal_attack").unwrap();
    assert!(attack.score(&frame, [0, 0, 1, 1])?.is_none());
    assert!(attack.score(&frame, [-1, 0, 100, 100]).is_err());
    assert!(attack.score(&frame, [1800, 0, 1921, 100]).is_err());

    // 将每个模板原样放入拼图，补充所有模板的正样本；避免只比较低分背景。
    let mut batches = Vec::new();
    let mut canvas = image::RgbImage::new(1920, 1080);
    let mut placements = Vec::new();
    let (mut x, mut y, mut row_height) = (0, 0, 0);
    for (key, definition) in &bundle.spec.templates {
        let image = image::open(bundle.root.join(&definition.image))?.to_rgb8();
        let (width, height) = (
            (image.width() + 4).min(1920),
            (image.height() + 3).min(1080),
        );
        if x + width > 1920 {
            x = 0;
            y += row_height;
            row_height = 0;
        }
        if y + height > 1080 {
            batches.push((canvas, placements));
            canvas = image::RgbImage::new(1920, 1080);
            placements = Vec::new();
            x = 0;
            y = 0;
            row_height = 0;
        }
        image::imageops::replace(&mut canvas, &image, i64::from(x), i64::from(y));
        placements.push((
            key.clone(),
            [x as i32, y as i32, (x + width) as i32, (y + height) as i32],
        ));
        x += width;
        row_height = row_height.max(height);
    }
    batches.push((canvas, placements));
    let mut positive_samples = 0;
    for (index, (canvas, placements)) in batches.into_iter().enumerate() {
        let path = root.join(format!("artifacts/combat-template-montage-{index}.png"));
        canvas.save(&path)?;
        let native = NativeSession::for_fixture(&path, &bundle.root, InferenceDevice::Cpu)?;
        let frame = native.combat_frame(&AtomicBool::new(false))?.unwrap();
        for (key, rect) in placements {
            let definition = &bundle.spec.templates[&key];
            let crop = frame.template_crop(rect)?;
            let params = serde_json::json!({
                "template":definition.image.strip_prefix("image/").unwrap(),
                "method":5,"threshold":0.8,"green_mask":definition.mask.is_some(),
                "order_by":"Score","roi":[0,0,rect[2]-rect[0],rect[3]-rect[1]],
            });
            let previous = native
                .recognize_prepared(&crop, "TemplateMatch", &params)?
                .detail;
            let direct = bundle
                .template(&key)
                .unwrap()
                .score(&frame, rect)?
                .unwrap_or(0.0);
            let score = previous["detail"]["all"][0]["score"]
                .as_f64()
                .unwrap_or(0.0);
            assert!(
                (score - direct).abs() <= 0.00002,
                "正样本 {key}: Maa {score}, 直接 {direct}"
            );
            assert_eq!(previous["hit"], direct > 0.8, "正样本 {key}");
            positive_samples += 1;
        }
    }
    eprintln!(
        "direct_template_comparison {}",
        serde_json::json!({
            "templates":bundle.spec.templates.len(),"maa_total_us":native_us,"direct_total_us":direct_us,
            "empty_results":empty_results,
            "positive_samples":positive_samples,
        })
    );
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、生成后的战斗资源和战斗画面留档"]
fn ordinary_battle_recognition_fits_freshness_budget() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/combat-regression-logs"),
    )?;
    let bundle = CombatBundle::load(&root.join("resources/generated/combat"))?;
    let native = NativeSession::for_fixture(
        &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
        &bundle.root,
        InferenceDevice::Cpu,
    )?;
    let mut recognizer = CombatRecognizer::new(bundle);
    let cancel = AtomicBool::new(false);
    let intervals = BTreeMap::from([("flash_enabled".into(), 0.0)]);
    let mut complete = false;
    let mut ages = vec![];
    for index in 0..40 {
        let frame = native.combat_frame(&cancel)?.unwrap();
        let observation = recognizer.observe(
            &native,
            &frame,
            f64::from(index),
            &intervals,
            false,
            &cancel,
        )?;
        ages.push(frame.age_ms());
        eprintln!(
            "frame {index}: {}ms, team {:?}",
            frame.age_ms(),
            observation.team
        );
        assert!(observation.in_battle);
        complete |= observation.team == ["叶瞬光", "千夏", "照"];
    }
    assert!(complete, "必须在有界帧数内发现完整真实队伍");
    assert!(
        ages.iter().all(|age| *age <= 250),
        "识别不能让动作持续超期：{ages:?}"
    );
    // 模型首次载入不计入热态预算；随后开启闪光仍须满足同一帧龄限制。
    let intervals = BTreeMap::from([("flash_enabled".into(), 1.0)]);
    let frame = native.combat_frame(&cancel)?.unwrap();
    recognizer.observe(&native, &frame, 41.0, &intervals, false, &cancel)?;
    for index in 42..47 {
        let frame = native.combat_frame(&cancel)?.unwrap();
        let observation = recognizer.observe(
            &native,
            &frame,
            f64::from(index),
            &intervals,
            false,
            &cancel,
        )?;
        eprintln!("flash frame {index}: {}ms", frame.age_ms());
        assert_eq!(observation.team, ["叶瞬光", "千夏", "照"]);
        assert!(frame.age_ms() <= 250, "闪光开启后也不能持续丢帧");
    }
    // 合成后台遮挡复现支援动画漏识别：槽位不减少，缺失身份仍必须留空。
    let mut partial = image::open(root.join("zzz-od-test/screens/战斗画面/默认.webp"))?.to_rgb8();
    image::imageops::replace(&mut partial, &image::RgbImage::new(400, 70), 550, 40);
    let partial_path = root.join("artifacts/combat-hidden-backline.png");
    partial.save(&partial_path)?;
    let partial_native =
        NativeSession::for_fixture(&partial_path, &recognizer.bundle.root, InferenceDevice::Cpu)?;
    let frame = partial_native.combat_frame(&cancel)?.unwrap();
    let observation = recognizer.observe(
        &partial_native,
        &frame,
        48.0,
        &BTreeMap::from([("flash_enabled".into(), 0.0)]),
        false,
        &cancel,
    )?;
    assert_eq!(
        observation.team,
        ["叶瞬光", "", ""],
        "后台漏识别不能把三人队伍缩成单人"
    );
    assert!(
        !observation
            .states
            .iter()
            .any(|state| state.name.starts_with("后台-")),
        "缓存成员不能冒充本帧后台识别结果"
    );
    assert!(frame.age_ms() <= 250);
    let frame = native.combat_frame(&cancel)?.unwrap();
    let observation = recognizer.observe(
        &native,
        &frame,
        49.0,
        &BTreeMap::from([("flash_enabled".into(), 0.0)]),
        false,
        &cancel,
    )?;
    assert_eq!(
        observation.team,
        ["叶瞬光", "千夏", "照"],
        "头像恢复后应立即重新命中原成员"
    );
    assert!(frame.age_ms() <= 250);
    // 合成遮挡只用于检查缓存失效：新画面没有头像时不能沿用上帧身份。
    let mut hidden = image::open(root.join("zzz-od-test/screens/战斗画面/默认.webp"))?.to_rgb8();
    image::imageops::replace(&mut hidden, &image::RgbImage::new(1000, 110), 0, 0);
    let hidden_path = root.join("artifacts/combat-hidden-portraits.png");
    hidden.save(&hidden_path)?;
    let hidden_native =
        NativeSession::for_fixture(&hidden_path, &recognizer.bundle.root, InferenceDevice::Cpu)?;
    let frame = hidden_native.combat_frame(&cancel)?.unwrap();
    let observation = recognizer.observe(
        &hidden_native,
        &frame,
        50.0,
        &BTreeMap::from([("flash_enabled".into(), 0.0)]),
        false,
        &cancel,
    )?;
    assert!(observation.in_battle);
    assert!(
        observation.team.iter().all(String::is_empty),
        "新画面无头像时不能使用缓存身份"
    );
    assert!(frame.age_ms() <= 250, "缓存失配后也不能退回完整串行扫描");
    Ok(())
}
