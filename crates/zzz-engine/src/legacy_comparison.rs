use crate::recognition::{self, Observation, RecognitionCase};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Deserialize)]
struct CurrentReport {
    schema_version: u32,
    sdk_version: String,
    binding_version: String,
    model_lock_sha256: String,
    case_manifest_sha256: String,
    requested_samples: usize,
    warmup: Option<usize>,
    build_profile: Option<String>,
    boundary_timing_scope: Option<String>,
    #[serde(default)]
    known_differences: Value,
    devices: Vec<CurrentDevice>,
}

#[derive(Deserialize)]
struct CurrentDevice {
    device: String,
    raw_directml_configuration_confirmed: Option<bool>,
    cases: Vec<CurrentCase>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct CurrentCase {
    id: String,
    kind: String,
    image_sha256: String,
    observation: Option<Observation>,
    unfiltered_observation: Option<Observation>,
    adaptation: Option<Value>,
    first_call: Option<Value>,
    measurements: Vec<Value>,
    #[serde(default)]
    boundary_measurements_us: Vec<u64>,
    stable: bool,
    errors: Vec<String>,
}

#[derive(Deserialize)]
struct LegacyReport {
    schema_version: u32,
    provenance: Value,
    case_manifest_sha256: String,
    requested_samples: usize,
    warmup: Option<usize>,
    boundary_timing_scope: Option<String>,
    devices: Vec<LegacyDevice>,
}

#[derive(Deserialize)]
struct LegacyDevice {
    device: String,
    providers: Vec<String>,
    cases: Vec<LegacyCase>,
}

#[derive(Deserialize)]
struct LegacyCase {
    id: String,
    kind: String,
    image_sha256: String,
    model_sha256: String,
    #[serde(default)]
    model_files_sha256: BTreeMap<String, String>,
    observation: Observation,
    unfiltered_observation: Option<Observation>,
    measurements_us: Vec<u64>,
    stable: bool,
    errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Comparison {
    pub device: String,
    pub case_id: String,
    pub kind: String,
    pub equal_within_tolerance: bool,
    pub filtered_equal_within_tolerance: bool,
    pub unfiltered_equal_within_tolerance: Option<bool>,
    pub max_score_abs_difference: Option<f64>,
    pub legacy_expectation_passed: Option<bool>,
    pub current_expectation_passed: Option<bool>,
    pub legacy: Observation,
    pub current: Option<Observation>,
    pub legacy_unfiltered: Option<Observation>,
    pub current_unfiltered: Option<Observation>,
    pub performance: Option<PerformanceComparison>,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PerformanceComparison {
    pub legacy: recognition::Quantiles,
    pub current: recognition::Quantiles,
    pub p95_ratio: f64,
    pub within_110_percent: bool,
}

pub fn compare_performance(legacy: &[u64], current: &[u64]) -> Result<PerformanceComparison> {
    ensure!(
        legacy.len() >= 30 && current.len() >= 30,
        "性能比较每边至少需要 30 个正式样本"
    );
    let legacy = recognition::quantiles(legacy)?;
    let current = recognition::quantiles(current)?;
    ensure!(legacy.p95_us > 0, "旧版 P95 为零，不能计算比例");
    Ok(PerformanceComparison {
        p95_ratio: current.p95_us as f64 / legacy.p95_us as f64,
        within_110_percent: u128::from(current.p95_us) * 10 <= u128::from(legacy.p95_us) * 11,
        legacy,
        current,
    })
}

#[derive(Debug, Serialize)]
pub struct ComparisonReport {
    pub schema_version: u32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub case_manifest_sha256: String,
    pub legacy_report_sha256: String,
    pub current_report_sha256: String,
    pub runtime_lock_sha256: String,
    pub reference_commit: String,
    pub comparisons: Vec<Comparison>,
    pub all_outputs_equal: bool,
    pub all_human_expectations_passed: bool,
    pub legacy_performance_available: bool,
    pub performance_scope: Option<String>,
    pub all_performance_targets_passed: Option<bool>,
    pub acceptance: String,
}

fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path)?;
    ensure!(bytes.len() <= 32 * 1024 * 1024, "识别报告超过 32 MiB");
    serde_json::from_slice(&bytes).with_context(|| format!("无效报告：{}", path.display()))
}

fn same_ids<'a>(actual: impl Iterator<Item = &'a str>, expected: &BTreeSet<&str>) -> bool {
    let values: Vec<_> = actual.collect();
    values.len() == expected.len() && values.into_iter().collect::<BTreeSet<_>>() == *expected
}

fn validate_flash(observation: &Observation, case: &RecognitionCase) -> Result<()> {
    let scores = observation.raw_scores.as_ref().context("缺少原始概率")?;
    ensure!(
        scores.len() == 3
            && scores
                .iter()
                .all(|score| score.is_finite() && (0.0..=1.0).contains(score))
            && (scores.iter().sum::<f64>() - 1.0).abs() <= 0.001,
        "原始概率无效"
    );
    let index = (0..3)
        .reduce(|a, b| if scores[b] > scores[a] { b } else { a })
        .unwrap();
    let candidate = observation
        .classification_candidate
        .as_ref()
        .context("缺少分类候选")?;
    let roi: [i32; 4] = serde_json::from_value(
        case.params
            .get("roi")
            .cloned()
            .unwrap_or(serde_json::json!([0, 0, 1920, 1080])),
    )?;
    ensure!(
        candidate.class_index == Some(index as i32)
            && candidate.label.as_deref() == Some(["none", "red", "yellow"][index])
            && candidate.score == Some(scores[index])
            && candidate.box_rect == roi
            && candidate.text.is_none()
            && candidate.count.is_none(),
        "分类候选与概率或 ROI 不符"
    );
    let threshold = case
        .params
        .get("threshold")
        .map_or(Some(0.9), Value::as_f64)
        .context("分类阈值无效")?;
    ensure!(
        threshold.is_finite() && (0.0..=1.0).contains(&threshold),
        "分类阈值无效"
    );
    ensure!(
        observation.hit == (scores[index] >= f64::from(threshold as f32))
            && if observation.hit {
                observation.items.len() == 1
                    && serde_json::to_value(&observation.items[0])?
                        == serde_json::to_value(candidate)?
            } else {
                observation.items.is_empty()
            },
        "分类结果与原始概率或阈值不符"
    );
    Ok(())
}

fn validate_observation(observation: &Observation, case: &RecognitionCase) -> Result<()> {
    if case.kind == "LegacyFlashRaw" {
        return validate_flash(observation, case);
    }
    ensure!(
        observation.classification_candidate.is_none()
            && observation.raw_scores.is_none()
            && observation.items.len() <= 512
            && observation.hit != observation.items.is_empty(),
        "OCR / YOLO 命中状态或结果结构无效"
    );
    for item in &observation.items {
        ensure!(
            item.box_rect[2] > 0
                && item.box_rect[3] > 0
                && item.count.is_none()
                && item
                    .score
                    .is_some_and(|score| score.is_finite() && (0.0..=1.0).contains(&score)),
            "OCR / YOLO 结果框或分数无效"
        );
        if case.kind == "OCR" {
            ensure!(
                item.text.as_ref().is_some_and(|text| !text.is_empty())
                    && item.label.is_none()
                    && item.class_index.is_none(),
                "OCR 文字结构无效"
            );
        } else {
            let classes = if case.params["model"] == maa_adapter::yolo::HOLLOW_MODEL {
                83
            } else {
                16
            };
            ensure!(
                item.text.is_none()
                    && item
                        .class_index
                        .is_some_and(|index| (0..classes).contains(&index))
                    && item.label.as_ref().is_some_and(|label| !label.is_empty()),
                "YOLO 类别结构无效"
            );
            if let Some(labels) = case.params.get("labels") {
                let index = item.class_index.unwrap() as usize;
                ensure!(
                    labels.get(index).and_then(Value::as_str) == item.label.as_deref(),
                    "YOLO 标签与类别不符"
                );
            }
        }
    }
    Ok(())
}

pub fn run(
    cases_path: &Path,
    fixture_root: &Path,
    runtime_lock: &Path,
    legacy_path: &Path,
    current_path: &Path,
) -> Result<ComparisonReport> {
    let manifest = recognition::load_cases(cases_path, fixture_root)?;
    ensure!(
        manifest.cases.iter().all(|case| matches!(
            case.kind.as_str(),
            "LegacyFlashRaw" | "LegacyYoloDetect" | "OCR"
        )),
        "旧输出比较只接收 OCR / LegacyFlashRaw / LegacyYoloDetect"
    );
    let lock: Value = read(runtime_lock)?;
    let legacy: LegacyReport = read(legacy_path)?;
    let current: CurrentReport = read(current_path)?;
    let performance_requested = legacy.boundary_timing_scope.is_some();
    if performance_requested {
        ensure!(
            legacy.boundary_timing_scope.as_deref() == Some(recognition::BOUNDARY_TIMING_SCOPE)
                && current.boundary_timing_scope == legacy.boundary_timing_scope,
            "性能计时边界不同或缺失，须重新采样"
        );
        ensure!(
            current.build_profile.as_deref() == Some("release"),
            "性能比较要求新版 release 构建"
        );
        ensure!(
            legacy.warmup.is_some_and(|count| count >= 5)
                && current.warmup.is_some_and(|count| count >= 5),
            "性能比较每边至少预热 5 次"
        );
        ensure!(
            manifest
                .cases
                .iter()
                .all(|case| case.kind == "LegacyYoloDetect"),
            "当前统一性能比较仅覆盖原始 YOLO"
        );
    }
    let models: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let model_lock_sha256 =
        hex::encode(Sha256::digest(include_bytes!("../../../models.lock.json")));
    let manifest_sha256 = recognition::sha256(cases_path)?;
    let runtime_lock_sha256 = recognition::sha256(runtime_lock)?;
    ensure!(
        lock["schema_version"] == 1 && legacy.schema_version == 1 && current.schema_version == 1,
        "采样约定或报告版本不兼容"
    );
    ensure!(
        lock["reference_commit"] == models["reference_commit"]
            && lock["model_lock_sha256"] == model_lock_sha256
            && current.model_lock_sha256 == model_lock_sha256,
        "参考提交或模型清单不匹配"
    );
    ensure!(
        current.sdk_version == maa_adapter::SDK_VERSION
            && current.binding_version == maa_adapter::BINDING_VERSION,
        "当前报告使用不同 SDK / Rust 绑定"
    );
    ensure!(
        legacy.case_manifest_sha256 == manifest_sha256
            && current.case_manifest_sha256 == manifest_sha256,
        "案例清单哈希不匹配，须用相同案例重新采样"
    );
    let evidence = &legacy.provenance;
    for field in [
        "reference_commit",
        "source_sha256",
        "model_lock_sha256",
        "exporter_sha256",
    ] {
        ensure!(
            !lock[field].is_null() && evidence[field] == lock[field],
            "旧版采样来源不匹配：{field}"
        );
    }
    if let Some(harness) = lock.get("harness_sha256") {
        ensure!(
            harness.is_object() && evidence["harness_sha256"] == *harness,
            "采样辅助工具哈希不匹配"
        );
    }
    let sources = lock["source_sha256"]
        .as_object()
        .context("采样约定缺少源码哈希")?;
    ensure!(
        !sources.is_empty()
            && sources
                .values()
                .all(|value| value.as_str().is_some_and(|s| s.len() == 64)),
        "采样源码哈希无效"
    );
    ensure!(
        evidence["runtime_lock_sha256"] == runtime_lock_sha256
            && evidence["python_version"] == lock["python"]["version"]
            && evidence["python_exe_sha256"] == lock["python"]["exe_sha256"],
        "旧版采样环境不匹配"
    );
    let packages = lock["packages"]
        .as_array()
        .context("采样约定缺少依赖版本")?;
    ensure!(!packages.is_empty(), "采样依赖为空");
    let mut versions = serde_json::Map::new();
    for package in packages {
        let name = package["name"].as_str().context("采样依赖名称无效")?;
        ensure!(
            package["version"].is_string()
                && versions
                    .insert(name.to_owned(), package["version"].clone())
                    .is_none(),
            "采样依赖版本无效或重复"
        );
    }
    ensure!(
        evidence["packages"] == Value::Object(versions),
        "旧版依赖版本不匹配"
    );
    ensure!(
        (1..=200).contains(&legacy.requested_samples)
            && (1..=200).contains(&current.requested_samples),
        "正式样本数无效"
    );
    let devices: BTreeSet<_> = current
        .devices
        .iter()
        .map(|device| device.device.as_str())
        .collect();
    ensure!(
        !devices.is_empty()
            && devices.len() <= 2
            && devices.contains("Cpu")
            && devices
                .iter()
                .all(|device| matches!(*device, "Cpu" | "DirectMl(0)"))
            && same_ids(
                current.devices.iter().map(|device| device.device.as_str()),
                &devices
            )
            && same_ids(
                legacy.devices.iter().map(|device| device.device.as_str()),
                &devices
            ),
        "新旧设备集合缺失、重复或不匹配；目前采样支持 CPU / DirectML(0)"
    );
    let ids: BTreeSet<_> = manifest.cases.iter().map(|case| case.id.as_str()).collect();
    let mut comparisons = Vec::new();
    for device in &current.devices {
        let previous = legacy
            .devices
            .iter()
            .find(|old| old.device == device.device)
            .unwrap();
        ensure!(
            same_ids(device.cases.iter().map(|case| case.id.as_str()), &ids)
                && same_ids(previous.cases.iter().map(|case| case.id.as_str()), &ids),
            "{} 的案例集合缺失、重复或多余",
            device.device
        );
        let provider = if device.device == "Cpu" {
            "CPUExecutionProvider"
        } else {
            "DmlExecutionProvider"
        };
        ensure!(
            previous.providers.first().map(String::as_str) == Some(provider),
            "旧版执行提供程序不匹配"
        );
        for case in &manifest.cases {
            let old = previous
                .cases
                .iter()
                .find(|item| item.id == case.id)
                .unwrap();
            let new = device.cases.iter().find(|item| item.id == case.id).unwrap();
            ensure!(
                old.kind == case.kind
                    && new.kind == case.kind
                    && old.image_sha256 == case.image.sha256
                    && new.image_sha256 == case.image.sha256,
                "案例类型或图片哈希不匹配：{}",
                case.id
            );
            let model = case.params["model"].as_str().context("缺少模型名称")?;
            let role = match case.kind.as_str() {
                "OCR" => "ocr",
                "LegacyYoloDetect" => "detect",
                _ => "classify",
            };
            let fixed_model = models["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| {
                    entry["role"] == role
                        && entry["id"].as_str().is_some_and(|id| {
                            if role == "ocr" {
                                id == model
                            } else {
                                format!("{id}.onnx") == model
                            }
                        })
                })
                .context("旧版比较只接受类型匹配的固定模型")?;
            ensure!(
                old.model_sha256 == fixed_model["files"][0]["sha256"],
                "旧版模型哈希不匹配"
            );
            if case.kind != "LegacyFlashRaw" {
                let files: BTreeMap<_, _> = fixed_model["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|file| {
                        (
                            file["bundle_path"]
                                .as_str()
                                .unwrap_or(file["path"].as_str().unwrap())
                                .to_owned(),
                            file["sha256"].as_str().unwrap().to_owned(),
                        )
                    })
                    .collect();
                ensure!(
                    old.model_files_sha256 == files,
                    "旧版模型、字典或标签文件哈希不匹配"
                );
                ensure!(
                    current
                        .known_differences
                        .as_array()
                        .is_some_and(|entries| entries.contains(fixed_model)),
                    "当前报告缺少已校验模型文件清单"
                );
            }
            validate_observation(&old.observation, case)?;
            let mut errors = old.errors.clone();
            errors.extend(new.errors.clone());
            if let Some(error) = &device.error {
                errors.push(error.clone());
            }
            if !old.stable || !new.stable {
                errors.push("正式样本不稳定".into());
            }
            if old.measurements_us.len() != legacy.requested_samples
                || new.measurements.len() != current.requested_samples
                || new.first_call.is_none()
            {
                errors.push("正式样本或首次调用不完整".into());
            }
            if device.device != "Cpu"
                && case.kind != "OCR"
                && device.raw_directml_configuration_confirmed != Some(true)
            {
                errors.push("当前报告缺少 DirectML 配置证据".into());
            }
            if let Some(observation) = &new.observation {
                validate_observation(observation, case)?;
                if case.kind != "OCR" {
                    let adaptation = new.adaptation.as_ref().context("缺少当前原始推理信息")?;
                    ensure!(
                        adaptation["model_sha256"] == fixed_model["files"][0]["sha256"]
                            && adaptation["model"] == model
                            && adaptation["kind"] == case.kind
                            && adaptation["device"] == device.device,
                        "当前实际模型或设备不匹配"
                    );
                    if case.kind == "LegacyYoloDetect" {
                        ensure!(
                            adaptation["labels_sha256"] == fixed_model["files"][1]["sha256"],
                            "当前实际标签文件不匹配"
                        );
                    }
                }
            } else {
                errors.push("当前识别缺少结果".into());
            }
            let filtered_equal = new.observation.as_ref().is_some_and(|observed| {
                recognition::compare(&old.observation, observed, case.tolerance)
            });
            let unfiltered_equal = if case.kind == "OCR" {
                let old_all = old
                    .unfiltered_observation
                    .as_ref()
                    .context("旧 OCR 报告缺少过滤前结果")?;
                validate_observation(old_all, case)?;
                if let Some(new_all) = &new.unfiltered_observation {
                    validate_observation(new_all, case)?;
                    Some(recognition::compare(old_all, new_all, case.tolerance))
                } else {
                    errors.push("当前 OCR 报告缺少过滤前结果，须重新采样".into());
                    Some(false)
                }
            } else {
                None
            };
            let equal = errors.is_empty() && filtered_equal && unfiltered_equal != Some(false);
            let performance = if performance_requested {
                ensure!(
                    new.boundary_measurements_us.len() == current.requested_samples
                        && old.measurements_us.len() == legacy.requested_samples,
                    "性能样本缺失：{}",
                    case.id
                );
                if equal {
                    Some(compare_performance(
                        &old.measurements_us,
                        &new.boundary_measurements_us,
                    )?)
                } else {
                    None
                }
            } else {
                None
            };
            let max_score_abs_difference = new
                .observation
                .as_ref()
                .and_then(|observed| observed.raw_scores.as_ref())
                .map(|scores| {
                    scores
                        .iter()
                        .zip(old.observation.raw_scores.as_deref().unwrap_or_default())
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0, f64::max)
                });
            comparisons.push(Comparison {
                device: device.device.clone(),
                case_id: case.id.clone(),
                kind: case.kind.clone(),
                equal_within_tolerance: equal,
                filtered_equal_within_tolerance: filtered_equal,
                unfiltered_equal_within_tolerance: unfiltered_equal,
                max_score_abs_difference,
                legacy_expectation_passed: case
                    .expectation
                    .as_ref()
                    .map(|expected| recognition::matches_expectation(&old.observation, expected)),
                current_expectation_passed: case.expectation.as_ref().and_then(|expected| {
                    new.observation
                        .as_ref()
                        .map(|observed| recognition::matches_expectation(observed, expected))
                }),
                legacy: old.observation.clone(),
                current: new.observation.clone(),
                legacy_unfiltered: old.unfiltered_observation.clone(),
                current_unfiltered: new.unfiltered_observation.clone(),
                performance,
                errors,
            });
        }
    }
    let all_outputs_equal = comparisons.iter().all(|case| case.equal_within_tolerance);
    let all_human_expectations_passed = comparisons.iter().all(|case| {
        case.legacy_expectation_passed == Some(true)
            && case.current_expectation_passed == Some(true)
    });
    let legacy_performance_available =
        performance_requested && comparisons.iter().all(|case| case.performance.is_some());
    let all_performance_targets_passed = performance_requested.then(|| {
        legacy_performance_available
            && comparisons.iter().all(|case| {
                case.performance
                    .as_ref()
                    .is_some_and(|perf| perf.within_110_percent)
            })
    });
    Ok(ComparisonReport {
        schema_version: 1, created_at: chrono::Utc::now(), case_manifest_sha256: manifest_sha256,
        legacy_report_sha256: recognition::sha256(legacy_path)?,
        current_report_sha256: recognition::sha256(current_path)?, runtime_lock_sha256,
        reference_commit: models["reference_commit"].as_str().unwrap().to_owned(),
        comparisons, all_outputs_equal, all_human_expectations_passed,
        legacy_performance_available,
        performance_scope: performance_requested.then(|| recognition::BOUNDARY_TIMING_SCOPE.to_owned()),
        all_performance_targets_passed,
        acceptance: "pending: 结论仅覆盖本清单的固定图输出；算法和人工标注差异、真实边界、性能及其余 M1 条件分别验收".into(),
    })
}
