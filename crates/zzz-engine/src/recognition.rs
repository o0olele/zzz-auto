use anyhow::{Context, Result, ensure};
use maa_adapter::{InferenceDevice, NativeSession, RecognitionTiming};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    time::Instant,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FixtureFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Expectation {
    pub hit: bool,
    pub text: Option<String>,
    pub class_index: Option<i32>,
    pub box_rect: Option<[i32; 4]>,
    #[serde(default)]
    pub box_tolerance_px: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Tolerance {
    pub box_px: u32,
    pub score_abs: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RecognitionCase {
    pub id: String,
    pub image: FixtureFile,
    pub kind: String,
    pub params: serde_json::Value,
    pub expectation: Option<Expectation>,
    pub legacy_evidence: String,
    pub tolerance: Tolerance,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CaseManifest {
    pub schema_version: u32,
    pub cases: Vec<RecognitionCase>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Item {
    pub text: Option<String>,
    pub class_index: Option<i32>,
    pub label: Option<String>,
    pub box_rect: [i32; 4],
    pub score: Option<f64>,
    pub count: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Observation {
    pub hit: bool,
    pub items: Vec<Item>,
    #[serde(default)]
    pub classification_candidate: Option<Item>,
}

#[derive(Debug, Serialize)]
pub struct Quantiles {
    pub samples: usize,
    pub min_us: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub max_us: u64,
}

#[derive(Debug, Serialize)]
pub struct CaseReport {
    pub id: String,
    pub kind: String,
    pub image_sha256: String,
    pub legacy_evidence: String,
    pub expectation_passed: Option<bool>,
    pub observation: Option<Observation>,
    pub adaptation: Option<serde_json::Value>,
    pub native_status: Option<String>,
    pub first_call: Option<RecognitionTiming>,
    pub fixture_prepare_us: Option<u64>,
    pub measurements: Vec<RecognitionTiming>,
    pub timing: BTreeMap<String, Quantiles>,
    pub stable: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DeviceReport {
    pub device: String,
    pub session_creation_us: u64,
    pub nn_dml_configuration_confirmed: Option<bool>,
    pub provider_note: String,
    pub cases: Vec<CaseReport>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeviceComparison {
    pub case_id: String,
    pub equal_within_tolerance: bool,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub schema_version: u32,
    pub engine_version: String,
    pub sdk_version: String,
    pub binding_version: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub warmup: usize,
    pub requested_samples: usize,
    pub model_lock_sha256: String,
    pub known_differences: serde_json::Value,
    pub devices: Vec<DeviceReport>,
    pub comparisons: Vec<DeviceComparison>,
    pub legacy_compatible: bool,
    pub legacy_performance_available: bool,
    pub acceptance: String,
}

impl CheckReport {
    pub fn basic_checks_passed(&self) -> bool {
        !self.devices.is_empty()
            && self.devices.iter().all(|device| {
                device.error.is_none()
                    && !device.cases.is_empty()
                    && device.nn_dml_configuration_confirmed != Some(false)
                    && device.cases.iter().all(|case| {
                        case.errors.is_empty()
                            && case.stable
                            && case.expectation_passed != Some(false)
                    })
            })
            && self
                .comparisons
                .iter()
                .all(|case| case.equal_within_tolerance)
    }
}

pub fn sha256(path: &Path) -> Result<String> {
    Ok(hex::encode(Sha256::digest(std::fs::read(path)?)))
}

pub fn relative_file(root: &Path, name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    ensure!(
        !name.is_empty()
            && !name.contains(['\\', ':'])
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "语料和模型须使用目录内的相对路径"
    );
    let root = root.canonicalize()?;
    let resolved = root
        .join(path)
        .canonicalize()
        .with_context(|| format!("缺少文件 {name}"))?;
    ensure!(
        resolved.starts_with(&root) && resolved.is_file(),
        "文件不在指定目录内：{name}"
    );
    Ok(resolved)
}

pub fn load_cases(path: &Path, fixture_root: &Path) -> Result<CaseManifest> {
    let bytes = std::fs::read(path)?;
    ensure!(bytes.len() <= 1024 * 1024, "识别案例清单过大");
    let manifest: CaseManifest = serde_json::from_slice(&bytes)?;
    ensure!(
        manifest.schema_version == 1 && !manifest.cases.is_empty() && manifest.cases.len() <= 100,
        "识别案例版本或数量无效"
    );
    let mut ids = BTreeSet::new();
    for case in &manifest.cases {
        ensure!(
            !case.id.trim().is_empty() && ids.insert(case.id.clone()),
            "识别案例 ID 为空或重复"
        );
        ensure!(
            [
                "OCR",
                "TemplateMatch",
                "FeatureMatch",
                "ColorMatch",
                "NeuralNetworkClassify",
                "LegacyFlashClassify",
                "NeuralNetworkDetect"
            ]
            .contains(&case.kind.as_str()),
            "识别类型无效"
        );
        ensure!(case.params.is_object(), "识别参数必须是对象");
        if let Some(roi) = case.params.get("roi") {
            let [x, y, width, height]: [i32; 4] =
                serde_json::from_value(roi.clone()).context("固定图 ROI 须为 x,y,width,height")?;
            ensure!(
                x >= 0
                    && y >= 0
                    && width > 0
                    && height > 0
                    && x.checked_add(width)
                        .is_some_and(|right| right <= od_core::BASE_WIDTH)
                    && y.checked_add(height)
                        .is_some_and(|bottom| bottom <= od_core::BASE_HEIGHT),
                "固定图 ROI 超出 1920×1080"
            );
        }
        ensure!(
            case.tolerance.score_abs.is_finite()
                && (0.0..=1.0).contains(&case.tolerance.score_abs)
                && case.tolerance.box_px <= 100,
            "识别容差无效"
        );
        ensure!(
            ["none", "document_snapshot", "same_model_capture"]
                .contains(&case.legacy_evidence.as_str()),
            "旧版证据类型无效"
        );
        let image = relative_file(fixture_root, &case.image.path)?;
        ensure!(
            sha256(&image)? == case.image.sha256,
            "语料哈希不匹配：{}",
            case.id
        );
    }
    Ok(manifest)
}

pub fn verify_model_bundle(root: &Path) -> Result<serde_json::Value> {
    let lock: serde_json::Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("model-manifest.json"))?)?;
    ensure!(
        manifest["schema_version"] == 2 && manifest["sdk_version"] == maa_adapter::SDK_VERSION,
        "模型资源清单版本不兼容"
    );
    let mut expected = BTreeMap::new();
    for model in lock["models"].as_array().context("固定模型清单无效")? {
        for file in model["files"].as_array().context("固定模型文件清单无效")? {
            if let Some(destination) = file["bundle_path"].as_str() {
                expected.insert(
                    destination.to_owned(),
                    (file["sha256"].clone(), file["bytes"].clone()),
                );
            }
        }
        if let Some(adaptation) = model.get("adaptation") {
            let name = adaptation["bundle_path"]
                .as_str()
                .context("衍生模型路径无效")?;
            ensure!(
                expected
                    .insert(
                        name.to_owned(),
                        (adaptation["sha256"].clone(), adaptation["bytes"].clone())
                    )
                    .is_none(),
                "衍生模型路径重复"
            );
        }
    }
    let files = manifest["files"].as_array().context("缺少模型文件清单")?;
    ensure!(
        files.len() == expected.len(),
        "模型资源文件数量与锁文件不符"
    );
    for file in files {
        let name = file["path"].as_str().context("模型路径无效")?;
        let (hash, size) = expected
            .remove(name)
            .context("模型清单包含重复或未知文件")?;
        ensure!(
            file["sha256"] == hash && file["bytes"] == size,
            "模型清单与锁文件不符：{name}"
        );
        let path = relative_file(root, name)?;
        ensure!(
            std::fs::metadata(&path)?.len() == size.as_u64().context("模型大小无效")?
                && sha256(&path)? == hash.as_str().context("模型哈希无效")?,
            "模型校验失败：{name}"
        );
    }
    ensure!(expected.is_empty(), "模型清单缺少文件");
    Ok(lock["models"].clone())
}

pub fn validate_model_requests(
    cases: &[RecognitionCase],
    models: &serde_json::Value,
) -> Result<()> {
    for case in cases {
        let role = match case.kind.as_str() {
            "OCR" => "ocr",
            "NeuralNetworkClassify" | "LegacyFlashClassify" => "classify",
            "NeuralNetworkDetect" => "detect",
            _ => continue,
        };
        let requested = case.params["model"]
            .as_str()
            .context("模型诊断须显式指定锁定模型名称")?;
        if case.kind == "LegacyFlashClassify" {
            maa_adapter::flash::FlashConfig::parse(&case.params)?;
            ensure!(
                models
                    .as_array()
                    .context("固定模型目录无效")?
                    .iter()
                    .any(|model| model["role"] == role
                        && model["adaptation"]["kind"] == "remove_output_softmax"
                        && model["adaptation"]["bundle_path"]
                            == format!("model/classify/{requested}")),
                "闪光适配请求未锁定的衍生模型"
            );
            continue;
        }
        ensure!(
            models
                .as_array()
                .context("固定模型目录无效")?
                .iter()
                .any(|model| {
                    let Some(id) = model["id"].as_str() else {
                        return false;
                    };
                    model["role"] == role
                        && requested
                            == if role == "ocr" {
                                id.to_owned()
                            } else {
                                format!("{id}.onnx")
                            }
                }),
            "案例 {} 请求未锁定或类型不匹配的模型：{requested}",
            case.id
        );
    }
    Ok(())
}

pub fn normalize(detail: &serde_json::Value) -> Result<Observation> {
    let hit = detail["hit"].as_bool().context("原生详情缺少 hit")?;
    let filtered = detail["detail"]["filtered"]
        .as_array()
        .context("原生详情缺少 filtered")?;
    ensure!(filtered.len() <= 512, "识别结果超过对照上限");
    let mut items = Vec::new();
    for result in filtered {
        let box_rect: [i32; 4] =
            serde_json::from_value(result["box"].clone()).context("识别框格式无效")?;
        ensure!(box_rect[2] > 0 && box_rect[3] > 0, "识别框宽高无效");
        let score = result
            .get("score")
            .map(|score| score.as_f64().context("分数格式无效"))
            .transpose()?;
        ensure!(
            score.is_none_or(|score| score.is_finite() && (0.0..=1.0).contains(&score)),
            "识别分数无效"
        );
        let class_index = result
            .get("cls_index")
            .map(|index| serde_json::from_value(index.clone()))
            .transpose()?;
        items.push(Item {
            text: result["text"].as_str().map(str::to_owned),
            class_index,
            label: result["label"].as_str().map(str::to_owned),
            box_rect,
            score,
            count: result
                .get("count")
                .map(|value| value.as_u64().context("匹配数量格式无效"))
                .transpose()?,
        });
    }
    ensure!(!hit || !items.is_empty(), "原生命中但没有可比较结果");
    let classification_candidate = detail
        .get("adaptation")
        .map(|adaptation| {
            let candidate = adaptation
                .get("candidate")
                .context("闪光适配缺少候选结果")?;
            let normalized =
                normalize(&serde_json::json!({"hit":true,"detail":{"filtered":[candidate]}}))?;
            Ok::<_, anyhow::Error>(normalized.items.into_iter().next().unwrap())
        })
        .transpose()?;
    Ok(Observation {
        hit,
        items,
        classification_candidate,
    })
}

fn item_equal(left: &Item, right: &Item, tolerance: Tolerance) -> bool {
    left.text == right.text
        && left.class_index == right.class_index
        && left.label == right.label
        && left.count == right.count
        && left.box_rect.iter().zip(right.box_rect).all(|(a, b)| {
            (i64::from(*a) - i64::from(b)).unsigned_abs() <= u64::from(tolerance.box_px)
        })
        && match (left.score, right.score) {
            (Some(a), Some(b)) => (a - b).abs() <= tolerance.score_abs,
            (None, None) => true,
            _ => false,
        }
}

fn pair(
    index: usize,
    left: &[Item],
    right: &[Item],
    tolerance: Tolerance,
    matched: &mut [Option<usize>],
    seen: &mut [bool],
) -> bool {
    for candidate in 0..right.len() {
        if seen[candidate] || !item_equal(&left[index], &right[candidate], tolerance) {
            continue;
        }
        seen[candidate] = true;
        if matched[candidate]
            .is_none_or(|previous| pair(previous, left, right, tolerance, matched, seen))
        {
            matched[candidate] = Some(index);
            return true;
        }
    }
    false
}

pub fn compare(left: &Observation, right: &Observation, tolerance: Tolerance) -> bool {
    if !tolerance.score_abs.is_finite()
        || !(0.0..=1.0).contains(&tolerance.score_abs)
        || tolerance.box_px > 100
    {
        return false;
    }
    if left.hit != right.hit || left.items.len() != right.items.len() || left.items.len() > 512 {
        return false;
    }
    if !match (
        &left.classification_candidate,
        &right.classification_candidate,
    ) {
        (Some(a), Some(b)) => item_equal(a, b, tolerance),
        (None, None) => true,
        _ => false,
    } {
        return false;
    }
    let mut matched = vec![None; right.items.len()];
    (0..left.items.len()).all(|index| {
        pair(
            index,
            &left.items,
            &right.items,
            tolerance,
            &mut matched,
            &mut vec![false; right.items.len()],
        )
    })
}

pub fn matches_expectation(observation: &Observation, expected: &Expectation) -> bool {
    if observation.hit != expected.hit {
        return false;
    }
    if !expected.hit {
        return true;
    }
    if expected.text.is_none() && expected.class_index.is_none() && expected.box_rect.is_none() {
        return true;
    }
    observation.items.iter().any(|item| {
        expected
            .text
            .as_ref()
            .is_none_or(|text| item.text.as_ref() == Some(text))
            && expected
                .class_index
                .is_none_or(|index| item.class_index == Some(index))
            && expected.box_rect.is_none_or(|rect| {
                rect.iter().zip(item.box_rect).all(|(a, b)| {
                    (i64::from(*a) - i64::from(b)).unsigned_abs()
                        <= u64::from(expected.box_tolerance_px)
                })
            })
    })
}

pub fn quantiles(values: &[u64]) -> Result<Quantiles> {
    ensure!(!values.is_empty(), "没有正式计时样本");
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Ok(Quantiles {
        samples: sorted.len(),
        min_us: sorted[0],
        p50_us: sorted[sorted.len().div_ceil(2) - 1],
        p95_us: sorted[(sorted.len() * 95).div_ceil(100) - 1],
        max_us: *sorted.last().unwrap(),
    })
}

fn check_case(
    native: &NativeSession,
    case: &RecognitionCase,
    root: &Path,
    warmup: usize,
    samples: usize,
) -> CaseReport {
    let mut report = CaseReport {
        id: case.id.clone(),
        kind: case.kind.clone(),
        image_sha256: case.image.sha256.clone(),
        legacy_evidence: case.legacy_evidence.clone(),
        expectation_passed: None,
        observation: None,
        adaptation: None,
        native_status: None,
        first_call: None,
        fixture_prepare_us: None,
        measurements: Vec::new(),
        timing: BTreeMap::new(),
        stable: true,
        errors: Vec::new(),
    };
    let checked = (|| -> Result<()> {
        let image = relative_file(root, &case.image.path)?;
        let image = maa_adapter::RecognitionImage::load(&image)?;
        report.fixture_prepare_us = Some(image.prepare_us);
        let first = native.recognize_prepared(&image, &case.kind, &case.params)?;
        let reference = normalize(&first.detail)?;
        report.adaptation = first.detail.get("adaptation").cloned();
        report.first_call = Some(first.timing);
        report.native_status = Some(first.native_status);
        report.observation = Some(reference.clone());
        report.expectation_passed = case
            .expectation
            .as_ref()
            .map(|expected| matches_expectation(&reference, expected));
        for sample in 0..warmup + samples {
            let result = native.recognize_prepared(&image, &case.kind, &case.params)?;
            let observed = normalize(&result.detail)?;
            if sample >= warmup {
                report.stable &= compare(&reference, &observed, case.tolerance);
                if let Some(expected) = &case.expectation {
                    report.expectation_passed = Some(
                        report.expectation_passed.unwrap_or(true)
                            && matches_expectation(&observed, expected),
                    );
                }
                report.measurements.push(result.timing);
            }
        }
        for (name, select) in [
            (
                "prepare",
                (|t: &RecognitionTiming| t.prepare_us) as fn(&RecognitionTiming) -> u64,
            ),
            ("enqueue", |t: &RecognitionTiming| t.enqueue_us),
            ("completion", |t: &RecognitionTiming| t.completion_us),
            ("details", |t: &RecognitionTiming| t.details_us),
            ("total", |t: &RecognitionTiming| t.total_us),
        ] {
            report.timing.insert(
                name.into(),
                quantiles(&report.measurements.iter().map(select).collect::<Vec<_>>())?,
            );
        }
        Ok(())
    })();
    if let Err(error) = checked {
        report.stable = false;
        report.errors.push(format!("{error:#}"));
    }
    report
}

fn log_checkpoint(logs: &Path) -> BTreeMap<PathBuf, u64> {
    let Ok(entries) = std::fs::read_dir(logs) else {
        return BTreeMap::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            entry
                .metadata()
                .ok()
                .map(|metadata| (entry.path(), metadata.len()))
        })
        .collect()
}

fn nn_dml_log_confirmed(logs: &Path, device_id: i32, checkpoint: &BTreeMap<PathBuf, u64>) -> bool {
    let marker = format!("[Px{}]", std::process::id());
    let Ok(entries) = std::fs::read_dir(logs) else {
        return false;
    };
    for entry in entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
    {
        let Ok(mut file) = std::fs::File::open(entry.path()) else {
            continue;
        };
        let Ok(size) = file.metadata().map(|metadata| metadata.len()) else {
            continue;
        };
        let offset = checkpoint.get(&entry.path()).copied().unwrap_or(0);
        if file
            .seek(SeekFrom::Start(if size >= offset { offset } else { 0 }))
            .is_err()
        {
            continue;
        }
        let mut content = Vec::new();
        if file.take(1024 * 1024).read_to_end(&mut content).is_err() {
            continue;
        }
        if String::from_utf8_lossy(&content).lines().any(|line| {
            line.contains(&marker)
                && line.contains("Using DML execution provider with device_id")
                && line.trim_end().ends_with(&format!("device_id {device_id}"))
        }) {
            return true;
        }
    }
    false
}

pub fn run(
    resources: &Path,
    fixture_root: &Path,
    cases_path: &Path,
    logs: &Path,
    directml: Option<i32>,
    warmup: usize,
    samples: usize,
) -> Result<CheckReport> {
    ensure!(
        warmup <= 100 && (1..=200).contains(&samples),
        "预热须为 0..100 次，正式样本须为 1..200 次"
    );
    ensure!(
        directml.is_none_or(|id| id >= 0),
        "DirectML 设备编号必须为非负整数"
    );
    let manifest = load_cases(cases_path, fixture_root)?;
    let models_needed = manifest.cases.iter().any(|case| {
        [
            "OCR",
            "NeuralNetworkClassify",
            "LegacyFlashClassify",
            "NeuralNetworkDetect",
        ]
        .contains(&case.kind.as_str())
    });
    let differences = if models_needed {
        verify_model_bundle(resources)?
    } else {
        serde_json::Value::Null
    };
    if models_needed {
        validate_model_requests(&manifest.cases, &differences)?;
    }
    let first_image = relative_file(fixture_root, &manifest.cases[0].image.path)?;
    let mut devices = Vec::new();
    for device in
        std::iter::once(InferenceDevice::Cpu).chain(directml.map(InferenceDevice::DirectMl))
    {
        let checkpoint = log_checkpoint(logs);
        let created = Instant::now();
        let mut report = DeviceReport {
            device: format!("{device:?}"),
            session_creation_us: 0,
            nn_dml_configuration_confirmed: None,
            provider_note: "设备选项已请求；SDK 不公开逐算子的实际执行位置".into(),
            cases: Vec::new(),
            error: None,
        };
        match NativeSession::for_fixture(&first_image, resources, device) {
            Ok(native) => {
                report.session_creation_us = created.elapsed().as_micros() as u64;
                if let InferenceDevice::DirectMl(id) = device {
                    // 先保留配置证据，避免样本日志把它移出日志尾部。
                    report.nn_dml_configuration_confirmed =
                        Some(nn_dml_log_confirmed(logs, id, &checkpoint));
                }
                for case in &manifest.cases {
                    report
                        .cases
                        .push(check_case(&native, case, fixture_root, warmup, samples));
                }
                // 此作用域结束并完成析构后，才创建下一种设备的原生会话。
            }
            Err(error) => report.error = Some(format!("{error:#}")),
        }
        if let InferenceDevice::DirectMl(id) = device {
            report.nn_dml_configuration_confirmed = Some(
                report.nn_dml_configuration_confirmed.unwrap_or(false)
                    || nn_dml_log_confirmed(logs, id, &checkpoint),
            );
            report.provider_note = "原生成功日志只确认 NN 会话配置了 DirectML；OCR 和每个算子的执行位置未由 SDK 公开，未确认时不得当作 DirectML 性能通过".into();
        }
        devices.push(report);
    }
    let mut comparisons = Vec::new();
    if devices.len() == 2 {
        for case in &manifest.cases {
            let cpu = devices[0].cases.iter().find(|result| result.id == case.id);
            let gpu = devices[1].cases.iter().find(|result| result.id == case.id);
            let equal = cpu.zip(gpu).is_some_and(|(a, b)| {
                a.errors.is_empty()
                    && b.errors.is_empty()
                    && a.stable
                    && b.stable
                    && a.observation
                        .as_ref()
                        .zip(b.observation.as_ref())
                        .is_some_and(|(a, b)| compare(a, b, case.tolerance))
            });
            comparisons.push(DeviceComparison {
                case_id: case.id.clone(),
                equal_within_tolerance: equal,
                reason: if equal {
                    "当前固定语料的输出在登记容差内一致"
                } else {
                    "缺少完整结果、样本不稳定或输出超出容差"
                }
                .into(),
            });
        }
    }
    Ok(CheckReport {
        schema_version: 1,
        engine_version: env!("CARGO_PKG_VERSION").into(),
        sdk_version: maa_adapter::SDK_VERSION.into(),
        binding_version: maa_adapter::BINDING_VERSION.into(),
        created_at: chrono::Utc::now(),
        warmup,
        requested_samples: samples,
        model_lock_sha256: hex::encode(Sha256::digest(include_bytes!("../../../models.lock.json"))),
        known_differences: differences,
        devices,
        comparisons,
        legacy_compatible: false,
        legacy_performance_available: false,
        acceptance: "pending: 固定图能力与设备输出对照，旧预处理、完整旧输出及同机性能基线尚未通过"
            .into(),
    })
}
