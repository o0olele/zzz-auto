//! 战斗工作线程只持有识别资源，不持有输入集合、游戏控制器或新的 GPU 租约。
use crate::{
    NativeSession, RawRecognitionControl, RecognitionImage, RecognitionMeasurement,
    RecognitionTiming,
};
use anyhow::{Context, Result, ensure};
use maa_framework::{MaaStatus, tasker::Tasker};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

pub trait CombatVision {
    fn match_template(
        &self,
        frame: &crate::combat_vision::CombatFrame,
        rect: [i32; 4],
        template: &crate::CombatTemplate,
    ) -> Result<Option<f64>> {
        template.score(frame, rect)
    }
    fn recognize_prepared_cancellable(
        &self,
        image: &RecognitionImage,
        kind: &str,
        params: &Value,
        control: &RawRecognitionControl,
    ) -> Result<RecognitionMeasurement>;
}
impl CombatVision for NativeSession {
    fn recognize_prepared_cancellable(
        &self,
        image: &RecognitionImage,
        kind: &str,
        params: &Value,
        control: &RawRecognitionControl,
    ) -> Result<RecognitionMeasurement> {
        NativeSession::recognize_prepared_cancellable(self, image, kind, params, control)
    }
}

pub struct CombatReader {
    raw_flash: Arc<Mutex<Option<crate::raw_inference::RawClassifier>>>,
    gate: Arc<Mutex<()>>,
    root: PathBuf,
    device: crate::InferenceDevice,
    cpu_threads: usize,
}
impl NativeSession {
    /// 调用者须先结束工作线程，再交接、释放本会话资源或 GPU 租约。
    pub fn combat_reader(
        &self,
        recognizer: &crate::combat_vision::CombatRecognizer,
    ) -> Result<CombatReader> {
        ensure!(
            !self.tasker.running(),
            "Pipeline 运行中不能启动战斗识别线程"
        );
        // 独立战斗直接用战斗包创建会话；应用内战斗则额外载入该包。
        // 两条路径都须匹配识别器已校验的包，不能只检查额外加载标记。
        let root = self.combat_bundle.as_ref().unwrap_or(&self.resource_root);
        ensure!(
            root == &recognizer.bundle.root,
            "战斗识别器与当前会话载入的资源不匹配"
        );
        Ok(CombatReader {
            raw_flash: self.raw_flash.clone(),
            gate: self.inference_gate.clone(),
            root: root.clone(),
            device: self.inference_device,
            cpu_threads: self.raw_cpu_threads,
        })
    }
}
impl CombatVision for CombatReader {
    fn recognize_prepared_cancellable(
        &self,
        image: &RecognitionImage,
        kind: &str,
        params: &Value,
        control: &RawRecognitionControl,
    ) -> Result<RecognitionMeasurement> {
        ensure!(kind == "LegacyFlashRaw", "战斗工作线程仅开放原始闪光推理");
        control.execute(|| {
            control.checkpoint(crate::RawRecognitionPhase::SessionSetup)?;
            let _guard = self
                .gate
                .lock()
                .map_err(|_| anyhow::anyhow!("战斗识别锁失效"))?;
            crate::raw_inference::recognize_raw_flash_cached(
                &self.raw_flash,
                &self.root,
                self.device,
                self.cpu_threads,
                image,
                params,
                control,
            )
        })
    }
}

pub(crate) fn recognize_maa(
    tasker: &Tasker,
    image: &RecognitionImage,
    kind: &str,
    params: &Value,
) -> Result<RecognitionMeasurement> {
    let started = Instant::now();
    ensure!(
        [
            "OCR",
            "TemplateMatch",
            "FeatureMatch",
            "ColorMatch",
            "NeuralNetworkClassify",
            "NeuralNetworkDetect"
        ]
        .contains(&kind),
        "不支持的识别类型"
    );
    let enqueue_at = Instant::now();
    let job = tasker.post_recognition(kind, &params.to_string(), &image.buffer)?;
    let enqueue_us = enqueue_at.elapsed().as_micros() as u64;
    let completion_at = Instant::now();
    let status = job.wait();
    ensure!(
        status.is_success() || status == MaaStatus::FAILED,
        "Maa 识别调用状态无效：{status}"
    );
    let completion_us = completion_at.elapsed().as_micros() as u64;
    let details_at = Instant::now();
    // SDK 返回 MaaTaskId；1.25.2 的 job.get 把它当成 MaaRecoId。
    // 按任务 → 节点 → 识别详情读取，保持使用固定官方绑定。
    let task = tasker
        .get_task_detail(job.id)?
        .context("Maa 未返回识别任务详情")?;
    let detail = task
        .nodes
        .into_iter()
        .flatten()
        .find_map(|node| node.recognition)
        .context("Maa 未返回识别详情")?;
    // 未命中可能返回 Failed，但有完整 recognition 详情，不能当作原生异常。
    ensure!(
        status.is_success() || !detail.hit,
        "Maa 识别任务失败但详情报告命中"
    );
    // 独立的只读 Tasker 按队列执行下一项。此任务已终结且详情已取完，
    // 不必为其 running 标记的尾部清理逐模板再休眠一次。
    Ok(RecognitionMeasurement {
        detail: serde_json::to_value(detail)?,
        native_status: status.to_string(),
        timing: RecognitionTiming {
            session_setup_us: 0,
            prepare_us: 0,
            enqueue_us,
            completion_us,
            details_us: details_at.elapsed().as_micros() as u64,
            total_us: started.elapsed().as_micros() as u64,
        },
    })
}
