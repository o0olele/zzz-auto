use anyhow::{Context, Result, ensure};
use ort::{
    session::Session,
    tensor::TensorElementType,
    value::{Tensor, ValueType},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, time::Instant};

use crate::{
    InferenceDevice, NativeSession, RawRecognitionControl, RawRecognitionPhase, RecognitionImage,
    RecognitionMeasurement, RecognitionTiming,
    flash::prepare_letterbox_bgr,
    raw_inference::{
        RawRuntimeInfo, bgr_to_rgb_tensor_controlled, create_raw_session, tensor_shape,
    },
};

pub const HOLLOW_MODEL: &str = "yolov8s-736-hollow-zero-event-0126.onnx";
pub const LOST_VOID_MODEL: &str = "yolov26n-736-lost-void-det-20260630.onnx";
pub const YOLO_SIZE: i32 = 736;
pub const AVATAR_RECT: [i32; 4] = [104, 40, 844, 110];

fn default_roi() -> [i32; 4] {
    [0, 0, 1920, 1080]
}
fn default_threshold() -> f64 {
    0.6
}
fn default_iou() -> f64 {
    0.5
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YoloConfig {
    pub model: String,
    #[serde(default = "default_roi")]
    pub roi: [i32; 4],
    #[serde(default = "default_threshold")]
    pub threshold: f64,
    #[serde(default = "default_iou")]
    pub iou: f64,
    pub labels: Option<Vec<String>>,
    pub label_list: Option<Vec<String>>,
    pub category_list: Option<Vec<String>>,
}

impl YoloConfig {
    pub fn input_size(&self) -> [i32; 2] {
        [
            YOLO_SIZE,
            if self.model == HOLLOW_MODEL {
                416
            } else {
                YOLO_SIZE
            },
        ]
    }
    pub fn parse(params: &Value) -> Result<Self> {
        let config: Self = serde_json::from_value(params.clone())?;
        ensure!(
            [HOLLOW_MODEL, LOST_VOID_MODEL].contains(&config.model.as_str()),
            "YOLO 适配须使用固定检测模型"
        );
        ensure!(
            config.roi == default_roi(),
            "YOLO 适配当前只接受完整 1920×1080 帧"
        );
        ensure!(
            [config.threshold, config.iou]
                .into_iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(&v)),
            "YOLO 阈值和 IoU 须为 0..1"
        );
        for filter in [&config.labels, &config.label_list, &config.category_list]
            .into_iter()
            .flatten()
        {
            ensure!(
                filter.len() <= 1024 && filter.iter().all(|v| !v.is_empty() && v.len() <= 256),
                "YOLO 类别参数无效"
            );
        }
        Ok(config)
    }
}

// OpenCV 填充矩形包含两个端点；只修改副本，不能污染之后的识别。
pub fn mask_battle_avatars(bgr: &mut [u8]) -> Result<()> {
    ensure!(
        bgr.len() == 1920 * 1080 * 3,
        "头像遮罩要求完整 1920×1080 BGR 帧"
    );
    let [x1, y1, x2, y2] = AVATAR_RECT;
    for y in y1..=y2 {
        bgr[((y * 1920 + x1) * 3) as usize..((y * 1920 + x2 + 1) * 3) as usize].fill(0);
    }
    Ok(())
}

pub fn prepare_yolo_bgr(bgr: &[u8], config: &YoloConfig) -> Result<crate::flash::FlashImage> {
    ensure!(
        bgr.len() == 1920 * 1080 * 3,
        "YOLO 适配要求完整 1920×1080 BGR 帧"
    );
    let mut source = bgr.to_vec();
    if config.model == LOST_VOID_MODEL {
        mask_battle_avatars(&mut source)?;
    }
    prepare_letterbox_bgr(&source, 1920, 1080, config.input_size())
}

#[derive(Debug, Clone, Serialize)]
pub struct YoloDetection {
    pub cls_index: usize,
    pub label: String,
    pub score: f32,
    // Maa 对照用 xywh，旧业务用向零取整后的 xyxy，浮点框用于检查 NMS。
    #[serde(rename = "box")]
    pub box_rect: [i32; 4],
    pub rect: [i32; 4],
    pub float_rect: [f32; 4],
    pub proposal_index: usize,
}

struct Candidate {
    index: usize,
    class: usize,
    score: f32,
    rect: [f32; 4],
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let intersection =
        (a[2].min(b[2]) - a[0].max(b[0])).max(0.0) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    intersection / ((a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - intersection)
}

pub fn decode_yolo(
    output: &[f32],
    shape: &[i64],
    scaled_size: [i32; 2],
    labels: &[String],
    config: &YoloConfig,
) -> Result<Vec<YoloDetection>> {
    decode_yolo_controlled(output, shape, scaled_size, labels, config, None)
}

fn decode_yolo_controlled(
    output: &[f32],
    shape: &[i64],
    scaled_size: [i32; 2],
    labels: &[String],
    config: &YoloConfig,
    control: Option<&RawRecognitionControl>,
) -> Result<Vec<YoloDetection>> {
    ensure!(
        !labels.is_empty() && labels.len() <= 1024,
        "YOLO 标签数量无效"
    );
    ensure!(
        config
            .labels
            .as_ref()
            .is_none_or(|requested| requested == labels),
        "YOLO 标签顺序与固定 CSV 不符"
    );
    ensure!(
        shape.len() == 3
            && shape[0] == 1
            && shape[1] == labels.len() as i64 + 4
            && (1..=20000).contains(&shape[2]),
        "YOLO 输出须为 float32 [1,4+C,N]"
    );
    let count = shape[2] as usize;
    ensure!(
        output.len() == (labels.len() + 4) * count,
        "YOLO 输出长度无效"
    );
    for (chunk_index, chunk) in output.chunks(4096).enumerate() {
        if let Some(control) = control {
            control.checkpoint(RawRecognitionPhase::Postprocessing)?;
        }
        for (offset, value) in chunk.iter().enumerate() {
            ensure!(value.is_finite(), "YOLO 输出数值无效");
            if chunk_index * 4096 + offset >= 4 * count {
                ensure!((0.0..=1.0).contains(value), "YOLO 类别分数无效");
            }
        }
    }
    ensure!(scaled_size == [736, 414], "YOLO 缩放尺寸不符合完整帧约定");
    let allowed: Vec<bool> = labels
        .iter()
        .map(|label| {
            config.label_list.is_none() && config.category_list.is_none()
                || config
                    .label_list
                    .as_ref()
                    .is_some_and(|filter| filter.contains(label))
            // 固定 CSV 无 category 列，字符串 category 不会匹配 None。
        })
        .collect();
    let mut by_class: BTreeMap<usize, Vec<Candidate>> = BTreeMap::new();
    for index in 0..count {
        if index % 128 == 0
            && let Some(control) = control
        {
            control.checkpoint(RawRecognitionPhase::Postprocessing)?;
        }
        let mut class = 0;
        let mut score = 0f32;
        for (next, keep) in allowed.iter().enumerate() {
            let value = if *keep {
                output[(next + 4) * count + index]
            } else {
                0.0
            };
            if value > score {
                class = next;
                score = value;
            }
        }
        // NumPy 2 使用 float32 数组和 Python float 阈值，且旧版为严格大于。
        if score <= config.threshold as f32 {
            continue;
        }
        let mut xywh = [0f32; 4];
        for axis in 0..4 {
            let normalized = output[axis * count + index] / scaled_size[axis % 2] as f32;
            // 原版 in-place 乘 int64 尺寸数组：计算后写回 float32。
            xywh[axis] = (f64::from(normalized) * f64::from([1920, 1080][axis % 2])) as f32;
        }
        let [x, y, w, h] = xywh;
        let rect = [x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0];
        ensure!(
            w > 0.0
                && h > 0.0
                && rect.iter().all(|v| v.is_finite() && v.abs() < 1_000_000.0)
                && rect[2] > rect[0]
                && rect[3] > rect[1],
            "YOLO 候选框无效"
        );
        by_class.entry(class).or_default().push(Candidate {
            index,
            class,
            score,
            rect,
        });
    }
    let mut results = Vec::new();
    for mut candidates in by_class.into_values() {
        candidates.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.index.cmp(&a.index)));
        while !candidates.is_empty() {
            if let Some(control) = control {
                control.checkpoint(RawRecognitionPhase::Postprocessing)?;
            }
            let picked = candidates.remove(0);
            for other in &candidates {
                ensure!(
                    picked.score != other.score
                        || iou(&picked.rect, &other.rect) < config.iou as f32,
                    "YOLO NMS 存在同类同分重叠框，尚未核对 NumPy 排序"
                );
            }
            candidates.retain(|other| iou(&picked.rect, &other.rect) < config.iou as f32);
            let rect = picked.rect.map(|v| v as i32);
            ensure!(rect[2] > rect[0] && rect[3] > rect[1], "YOLO 框取整后为空");
            results.push(YoloDetection {
                cls_index: picked.class,
                label: labels[picked.class].clone(),
                score: picked.score,
                box_rect: [rect[0], rect[1], rect[2] - rect[0], rect[3] - rect[1]],
                rect,
                float_rect: picked.rect,
                proposal_index: picked.index,
            });
            ensure!(results.len() <= 512, "YOLO 结果超过诊断上限");
        }
    }
    Ok(results)
}

pub(crate) struct RawDetector {
    session: Session,
    runtime: RawRuntimeInfo,
    sha256: String,
    labels_sha256: String,
    labels: Vec<String>,
}

fn labels_file(root: &Path, model: &str) -> Result<(Vec<String>, String)> {
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let id = model.strip_suffix(".onnx").context("检测模型名称无效")?;
    let expected = lock["models"]
        .as_array()
        .context("模型锁文件无效")?
        .iter()
        .find(|entry| entry["id"] == id)
        .and_then(|entry| entry["files"].as_array())
        .and_then(|files| files.iter().find(|file| file["path"] == "labels.csv"))
        .context("缺少锁定 YOLO 标签")?;
    let path = root
        .join(format!("model/detect/{id}.labels.csv"))
        .canonicalize()
        .context("缺少检测标签，请重新运行 od-tools models")?;
    ensure!(path.starts_with(root), "检测标签指向资源之外");
    ensure!(
        std::fs::metadata(&path)?.len() == expected["bytes"].as_u64().context("标签大小无效")?,
        "YOLO 标签大小不符"
    );
    let data = std::fs::read(&path)?;
    let hash = hex::encode(Sha256::digest(&data));
    ensure!(expected["sha256"] == hash, "YOLO 标签哈希不符");
    let mut labels = Vec::new();
    for line in std::str::from_utf8(&data)?.lines().skip(1) {
        let fields: Vec<_> = line.split(',').collect();
        ensure!(
            fields.len() == 2
                && fields[0].parse::<usize>()? == labels.len()
                && !fields[1].is_empty(),
            "固定 YOLO CSV 格式无效"
        );
        labels.push(fields[1].to_owned());
    }
    ensure!(!labels.is_empty(), "YOLO 标签为空");
    Ok((labels, hash))
}

impl RawDetector {
    fn new(
        root: &Path,
        config: &YoloConfig,
        device: InferenceDevice,
        cpu_threads: usize,
    ) -> Result<Self> {
        let (labels, labels_sha256) = labels_file(root, &config.model)?;
        ensure!(
            config
                .labels
                .as_ref()
                .is_none_or(|requested| requested == &labels),
            "YOLO 标签顺序与固定 CSV 不符"
        );
        let path = root
            .join("model/detect")
            .join(&config.model)
            .canonicalize()?;
        ensure!(path.starts_with(root), "检测模型指向资源之外");
        let (session, runtime, sha256) =
            create_raw_session(&path, device, cpu_threads, 64 * 1024 * 1024)?;
        ensure!(
            session.inputs.len() == 1
                && session.inputs[0].name == "images"
                && tensor_shape(
                    &session.inputs[0].input_type,
                    &[
                        1,
                        3,
                        i64::from(config.input_size()[1]),
                        i64::from(config.input_size()[0])
                    ]
                ),
            "YOLO 模型输入须为 images / float32，并符合固定模型的 736×416 或 736×736 尺寸"
        );
        ensure!(
            session.outputs.len() == 1
                && session.outputs[0].name == "output0"
                && matches!(&session.outputs[0].output_type, ValueType::Tensor { ty: TensorElementType::Float32, shape, .. } if shape.len() == 3 && shape[0] == 1 && shape[1] == labels.len() as i64 + 4 && (1..=20000).contains(&shape[2])),
            "YOLO 模型输出须为 output0 / float32 [1,4+C,N]"
        );
        Ok(Self {
            session,
            runtime,
            sha256,
            labels_sha256,
            labels,
        })
    }
}

impl NativeSession {
    pub(crate) fn recognize_yolo(
        &self,
        image: &RecognitionImage,
        params: &Value,
        control: &RawRecognitionControl,
    ) -> Result<RecognitionMeasurement> {
        let started = Instant::now();
        let config = YoloConfig::parse(params)?;
        let mut cached = self
            .raw_detectors
            .lock()
            .map_err(|_| anyhow::anyhow!("原始检测会话锁失效"))?;
        let setup_at = Instant::now();
        let session_setup_us = if !cached.contains_key(&config.model) {
            cached.insert(
                config.model.clone(),
                RawDetector::new(
                    &self.resource_root,
                    &config,
                    self.inference_device,
                    self.raw_cpu_threads,
                )?,
            );
            setup_at.elapsed().as_micros() as u64
        } else {
            0
        };
        let detector = cached.get_mut(&config.model).unwrap();
        control.checkpoint(RawRecognitionPhase::Preprocessing)?;
        ensure!(
            config
                .labels
                .as_ref()
                .is_none_or(|requested| requested == &detector.labels),
            "YOLO 标签顺序与固定 CSV 不符"
        );
        let prepare_at = Instant::now();
        ensure!(
            image.buffer.width() == 1920
                && image.buffer.height() == 1080
                && image.buffer.image_type() == 16,
            "YOLO 适配要求原始 1920×1080 BGR 图像"
        );
        let source = image.buffer.raw_data().context("YOLO 图像像素为空")?;
        let prepared = prepare_yolo_bgr(source, &config)?;
        let tensor = Tensor::from_array((
            [
                1,
                3,
                config.input_size()[1] as i64,
                config.input_size()[0] as i64,
            ],
            bgr_to_rgb_tensor_controlled(&prepared.bgr, config.input_size(), Some(control))?,
        ))?;
        let prepare_us = prepare_at.elapsed().as_micros() as u64;
        let completion_at = Instant::now();
        let options = control.start_inference()?;
        let output = detector
            .session
            .run_with_options(ort::inputs!["images" => tensor], &options);
        control.end_inference(output.as_ref().err().map(ToString::to_string))?;
        control.checkpoint(RawRecognitionPhase::Postprocessing)?;
        let output = output?;
        let completion_us = completion_at.elapsed().as_micros() as u64;
        let details_at = Instant::now();
        let (shape, values) = output["output0"].try_extract_tensor::<f32>()?;
        let detections = decode_yolo_controlled(
            values,
            shape,
            prepared.scaled_size,
            &detector.labels,
            &config,
            Some(control),
        )?;
        let detail = json!({"hit":!detections.is_empty(), "detail":{"all":detections,"filtered":detections,"best":detections.first()},
            "adaptation":{"kind":"LegacyYoloDetect","model":config.model,"model_sha256":detector.sha256,"labels_sha256":detector.labels_sha256,
                "runtime":detector.runtime,"device":format!("{:?}",self.inference_device),
                "threading":crate::raw_inference::raw_threading(self.inference_device, self.raw_cpu_threads)?,
                "directml_registration_confirmed":match self.inference_device { InferenceDevice::Cpu => Value::Null, InferenceDevice::DirectMl(_) => json!(true) },
                "output_shape":&**shape,"input_size":config.input_size(),"scaled_size":prepared.scaled_size,"detections":detections,"threshold":config.threshold,"effective_threshold_f32":config.threshold as f32,
                "iou":config.iou,"effective_iou_f32":config.iou as f32,"label_list":config.label_list,"category_list":config.category_list,
                "avatar_mask_xyxy_inclusive":if config.model == LOST_VOID_MODEL {Some(AVATAR_RECT)} else {None},
                "preprocessing":"SDK OpenCV INTER_LINEAR, top-left alignment, right/bottom padding 114, RGB NCHW f64 division then f32",
                "postprocessing":"filter before first argmax; strict confidence; per-class float32 NMS suppresses IoU equality; truncate xyxy without clipping"}});
        Ok(RecognitionMeasurement {
            detail,
            native_status: "RawInferenceSucceeded".into(),
            timing: RecognitionTiming {
                session_setup_us,
                prepare_us,
                enqueue_us: 0,
                completion_us,
                details_us: details_at.elapsed().as_micros() as u64,
                total_us: started.elapsed().as_micros() as u64,
            },
        })
    }
}
