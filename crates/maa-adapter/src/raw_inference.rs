use anyhow::{Context, Result, ensure};
use ort::{
    execution_providers::DirectMLExecutionProvider,
    session::Session,
    tensor::TensorElementType,
    value::{TensorRef, ValueType},
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    ffi::CStr,
    path::Path,
    sync::{Mutex, OnceLock},
    time::Instant,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

use crate::{
    InferenceDevice, NativeSession, RawRecognitionControl, RawRecognitionPhase, RecognitionImage,
    RecognitionMeasurement, RecognitionTiming,
    flash::{self, FLASH_LABELS, FLASH_SIZE, FlashConfig},
};

pub const RAW_FLASH_MODEL: &str = "yolov8n-640-flash-20250921.onnx";
pub const ORT_BINDING_VERSION: &str = "2.0.0-rc.10";
pub const ORT_RUNTIME_VERSION: &str = "1.29.0";
pub const DEFAULT_RAW_CPU_THREADS: usize = 0;

pub fn raw_threading(device: InferenceDevice, cpu_threads: usize) -> Result<Value> {
    ensure!(
        cpu_threads <= 64,
        "原始 CPU 线程数须为 0..64，0 表示 ORT 默认"
    );
    Ok(
        json!({"intra_op_threads":if matches!(device, InferenceDevice::Cpu) {cpu_threads} else {1},
        "inter_op_threads":1,"execution_mode":"sequential","intra_op_spinning":false,"inter_op_spinning":false}),
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct RawRuntimeInfo {
    pub version: String,
    pub binding_version: String,
    pub api_version: u32,
    pub dll: String,
}
static RUNTIME: Mutex<Option<RawRuntimeInfo>> = Mutex::new(None);

pub fn raw_runtime_info() -> Result<RawRuntimeInfo> {
    let mut runtime = RUNTIME
        .lock()
        .map_err(|_| anyhow::anyhow!("原始推理初始化锁失效"))?;
    if let Some(info) = runtime.as_ref() {
        return Ok(info.clone());
    }
    crate::sdk::verify_loaded_modules()?;
    let dll = crate::sdk::sdk_directory()?
        .join("onnxruntime_maa.dll")
        .canonicalize()?;
    let name: Vec<u16> = "onnxruntime_maa.dll"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    ensure!(!module.is_null(), "SDK ONNX Runtime 尚未加载");
    // 使用官方 ABI 定义先检查版本和 API，避免 ort 在不兼容 DLL 上 panic。
    let get_base = unsafe {
        std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "system" fn() -> *const ort::sys::OrtApiBase,
        >(
            GetProcAddress(module, c"OrtGetApiBase".as_ptr().cast())
                .context("SDK 缺少 OrtGetApiBase")?,
        )
    };
    let base = unsafe { get_base().as_ref() }.context("SDK OrtApiBase 为空")?;
    let version_ptr = unsafe { (base.GetVersionString)() };
    ensure!(!version_ptr.is_null(), "SDK ONNX Runtime 版本为空");
    let version = unsafe { CStr::from_ptr(version_ptr) }.to_str()?.to_owned();
    ensure!(
        version == ORT_RUNTIME_VERSION,
        "SDK ONNX Runtime {version} 不在当前固定兼容组合内"
    );
    let expected_api = unsafe { (base.GetApi)(ort::MINOR_VERSION) };
    ensure!(
        !expected_api.is_null(),
        "SDK 不支持原始推理 API {}",
        ort::MINOR_VERSION
    );
    ensure!(
        ort::init_from(dll.to_str().context("SDK 路径不是 UTF-8")?)
            .with_name("zzz-auto-raw")
            .commit()?,
        "原始推理环境已被其他入口初始化"
    );
    ensure!(
        std::ptr::eq(ort::api(), expected_api),
        "Rust ort 未使用已校验的 SDK API"
    );
    crate::sdk::verify_loaded_modules()?;
    let info = RawRuntimeInfo {
        version,
        binding_version: ORT_BINDING_VERSION.into(),
        api_version: ort::MINOR_VERSION,
        dll: dll.display().to_string(),
    };
    *runtime = Some(info.clone());
    Ok(info)
}

pub fn bgr_to_rgb_tensor(bgr: &[u8]) -> Result<Vec<f32>> {
    bgr_to_rgb_tensor_sized(bgr, FLASH_SIZE)
}

pub(crate) fn bgr_to_rgb_tensor_sized(bgr: &[u8], size: i32) -> Result<Vec<f32>> {
    bgr_to_rgb_tensor_dimensions(bgr, [size, size])
}

pub(crate) fn bgr_to_rgb_tensor_dimensions(bgr: &[u8], size: [i32; 2]) -> Result<Vec<f32>> {
    bgr_to_rgb_tensor_controlled(bgr, size, None)
}

pub(crate) fn bgr_to_rgb_tensor_controlled(
    bgr: &[u8],
    size: [i32; 2],
    control: Option<&RawRecognitionControl>,
) -> Result<Vec<f32>> {
    let mut tensor = Vec::new();
    bgr_to_rgb_tensor_into(bgr, size, control, &mut tensor)?;
    Ok(tensor)
}

fn bgr_to_rgb_tensor_into(
    bgr: &[u8],
    size: [i32; 2],
    control: Option<&RawRecognitionControl>,
    tensor: &mut Vec<f32>,
) -> Result<()> {
    let [width, height] = size;
    ensure!(
        (1..=1920).contains(&width) && (1..=1080).contains(&height),
        "模型输入尺寸无效"
    );
    let plane = (width * height) as usize;
    ensure!(
        bgr.len() == plane * 3,
        "原始张量须来自模型输入尺寸的 BGR 图像"
    );
    tensor.resize(plane * 3, 0.0);
    static NORMALIZED: OnceLock<[f32; 256]> = OnceLock::new();
    let normalized = NORMALIZED.get_or_init(|| std::array::from_fn(|v| (v as f64 / 255.0) as f32));
    for (position, pixel) in bgr.chunks_exact(3).enumerate() {
        if position % 4096 == 0
            && let Some(control) = control
        {
            control.checkpoint(RawRecognitionPhase::Preprocessing)?;
        }
        for (channel, source) in [2, 1, 0].into_iter().enumerate() {
            // 旧版先 NumPy float64 除法，最后 astype(float32)，不使用 f32 乘倒数。
            tensor[channel * plane + position] = normalized[pixel[source] as usize];
        }
    }
    Ok(())
}

pub fn raw_flash_decision(scores: [f32; 3], config: &FlashConfig) -> Result<Value> {
    ensure!(
        scores
            .iter()
            .all(|score| score.is_finite() && (0.0..=1.0).contains(score)),
        "原始闪光概率无效"
    );
    ensure!(
        (scores.iter().sum::<f32>() - 1.0).abs() <= 0.0001,
        "原始闪光概率总和不为 1"
    );
    let mut index = 0;
    for next in 1..scores.len() {
        if scores[next] > scores[index] {
            index = next;
        }
    }
    // 参考仓锁定 NumPy 2：float32 标量与 Python float 比较，阈值转为 float32。
    let threshold = config.threshold as f32;
    let accepted = scores[index] >= threshold;
    let candidate = json!({"cls_index":index,"label":FLASH_LABELS[index],"score":scores[index],"box":config.roi});
    Ok(
        json!({"hit":accepted,"detail":{"all":[candidate.clone()],"filtered":if accepted {vec![candidate.clone()]} else {vec![]},"best":if accepted {candidate.clone()} else {Value::Null}},"adaptation":{"kind":"LegacyFlashRaw","model":RAW_FLASH_MODEL,"threshold":config.threshold,"effective_threshold_f32":threshold,"class_index":if accepted {index as i32} else {-1},"candidate":candidate,"raw_scores":scores,"score_source":"SDK ONNX Runtime float32 probabilities","preprocessing":"SDK OpenCV INTER_LINEAR, top-left alignment, right/bottom padding 114, RGB NCHW f64 division then f32"}}),
    )
}

pub(crate) struct RawClassifier {
    session: Session,
    runtime: RawRuntimeInfo,
    sha256: String,
    device: InferenceDevice,
    bgr: Vec<u8>,
    tensor: Vec<f32>,
}

pub(crate) fn tensor_shape(ty: &ValueType, expected: &[i64]) -> bool {
    matches!(ty,ValueType::Tensor { ty:TensorElementType::Float32, shape,.. } if &**shape==expected)
}

pub(crate) fn create_raw_session(
    path: &Path,
    device: InferenceDevice,
    cpu_threads: usize,
    max_bytes: u64,
) -> Result<(Session, RawRuntimeInfo, String)> {
    let threading = raw_threading(device, cpu_threads)?;
    let runtime = raw_runtime_info()?;
    ensure!(
        (1..=max_bytes).contains(&std::fs::metadata(path)?.len()),
        "原始模型大小无效"
    );
    let bytes = std::fs::read(path)?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let mut builder = Session::builder()?
        .with_parallel_execution(false)?
        .with_intra_threads(threading["intra_op_threads"].as_u64().unwrap() as usize)?
        .with_inter_threads(1)?
        .with_config_entry("session.intra_op.allow_spinning", "0")?
        .with_config_entry("session.inter_op.allow_spinning", "0")?;
    if let InferenceDevice::DirectMl(id) = device {
        ensure!(id >= 0, "DirectML 设备编号必须非负");
        builder = builder
            .with_memory_pattern(false)?
            .with_execution_providers([DirectMLExecutionProvider::default()
                .with_device_id(id)
                .build()
                .error_on_failure()])?;
    }
    let session = builder
        .commit_from_memory(&bytes)
        .context("SDK 原始推理会话创建失败")?;
    crate::sdk::verify_loaded_modules()?;
    Ok((session, runtime, sha256))
}

impl RawClassifier {
    fn new(path: &Path, device: InferenceDevice, cpu_threads: usize) -> Result<Self> {
        let (session, runtime, sha256) =
            create_raw_session(path, device, cpu_threads, 16 * 1024 * 1024)?;
        ensure!(
            session.inputs.len() == 1
                && session.inputs[0].name == "images"
                && tensor_shape(&session.inputs[0].input_type, &[1, 3, 640, 640]),
            "原始闪光模型输入须为 images / float32 [1,3,640,640]"
        );
        ensure!(
            session.outputs.len() == 1
                && session.outputs[0].name == "output0"
                && tensor_shape(&session.outputs[0].output_type, &[1, 3]),
            "原始闪光模型输出须为 output0 / float32 [1,3]"
        );
        crate::sdk::verify_loaded_modules()?;
        Ok(Self {
            session,
            runtime,
            sha256,
            device,
            bgr: Vec::new(),
            tensor: Vec::new(),
        })
    }
}

impl NativeSession {
    pub(crate) fn recognize_raw_flash(
        &self,
        image: &RecognitionImage,
        params: &Value,
        control: &RawRecognitionControl,
    ) -> Result<RecognitionMeasurement> {
        recognize_raw_flash_cached(
            &self.raw_flash,
            self.combat_bundle.as_ref().unwrap_or(&self.resource_root),
            self.inference_device,
            self.raw_cpu_threads,
            image,
            params,
            control,
        )
    }
}

pub(crate) fn recognize_raw_flash_cached(
    cache: &Mutex<Option<RawClassifier>>,
    root: &Path,
    device: InferenceDevice,
    cpu_threads: usize,
    image: &RecognitionImage,
    params: &Value,
    control: &RawRecognitionControl,
) -> Result<RecognitionMeasurement> {
    let started = Instant::now();
    let config = FlashConfig::parse_raw(params)?;
    let mut cached = cache
        .lock()
        .map_err(|_| anyhow::anyhow!("原始闪光会话锁失效"))?;
    let setup_at = Instant::now();
    let session_setup_us = if cached.is_none() {
        let path = root
            .join("model/classify")
            .join(RAW_FLASH_MODEL)
            .canonicalize()
            .context("缺少固定原始闪光模型")?;
        ensure!(path.starts_with(root), "原始闪光模型指向资源目录之外");
        *cached = Some(RawClassifier::new(&path, device, cpu_threads)?);
        setup_at.elapsed().as_micros() as u64
    } else {
        0
    };
    let classifier = cached.as_mut().unwrap();
    control.checkpoint(RawRecognitionPhase::Preprocessing)?;
    let prepare_at = Instant::now();
    let scaled_size = flash::prepare_flash_image_into(image, config.roi, &mut classifier.bgr)?;
    bgr_to_rgb_tensor_into(
        &classifier.bgr,
        [640, 640],
        Some(control),
        &mut classifier.tensor,
    )?;
    // 同步推理借用会话内缓冲；run 返回之前不能重写或释放输入。
    let tensor = TensorRef::from_array_view(([1, 3, 640, 640], classifier.tensor.as_slice()))?;
    let prepare_us = prepare_at.elapsed().as_micros() as u64;
    let completion_at = Instant::now();
    let options = control.start_inference()?;
    let output = classifier
        .session
        .run_with_options(ort::inputs!["images"=>tensor], &options);
    control.end_inference(output.as_ref().err().map(ToString::to_string))?;
    control.checkpoint(RawRecognitionPhase::Postprocessing)?;
    let output = output?;
    let completion_us = completion_at.elapsed().as_micros() as u64;
    let details_at = Instant::now();
    let (shape, scores) = output["output0"].try_extract_tensor::<f32>()?;
    ensure!(
        **shape == [1, 3] && scores.len() == 3,
        "原始闪光输出张量尺寸无效"
    );
    let scores: [f32; 3] = scores.try_into()?;
    let mut detail = raw_flash_decision(scores, &config)?;
    detail["adaptation"]["runtime"] = serde_json::to_value(&classifier.runtime)?;
    detail["adaptation"]["threading"] = raw_threading(classifier.device, cpu_threads)?;
    detail["adaptation"]["model_sha256"] = json!(classifier.sha256);
    detail["adaptation"]["device"] = json!(format!("{:?}", classifier.device));
    detail["adaptation"]["directml_registration_confirmed"] = match classifier.device {
        InferenceDevice::Cpu => Value::Null,
        InferenceDevice::DirectMl(_) => json!(true),
    };
    detail["adaptation"]["scaled_size"] = json!(scaled_size);
    let details_us = details_at.elapsed().as_micros() as u64;
    Ok(RecognitionMeasurement {
        detail,
        native_status: "RawInferenceSucceeded".into(),
        timing: RecognitionTiming {
            session_setup_us,
            prepare_us,
            enqueue_us: 0,
            completion_us,
            details_us,
            total_us: started.elapsed().as_micros() as u64,
        },
    })
}

#[cfg(test)]
#[path = "raw_flash_buffer_tests.rs"]
mod tests;
