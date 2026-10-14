use anyhow::{Result, ensure};
use ort::session::RunOptions;
use serde::Serialize;
use std::{
    fmt,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RawRecognitionPhase {
    #[default]
    Pending,
    ImageLoad,
    SessionSetup,
    Preprocessing,
    Inference,
    Postprocessing,
    Finished,
}

#[derive(Debug, Clone, Serialize)]
pub struct RawRecognitionStatus {
    pub phase: RawRecognitionPhase,
    pub cancel_requested: bool,
    pub terminate_requested: bool,
    pub native_run_returned: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RawRecognitionCancelled {
    pub code: &'static str,
    pub requested_phase: RawRecognitionPhase,
    pub elapsed_us: u64,
    pub stop_us: u64,
    pub terminate_requested: bool,
    pub native_run_returned: bool,
    pub native_error: Option<String>,
    pub terminate_error: Option<String>,
    pub operation_error: Option<String>,
}

impl fmt::Display for RawRecognitionCancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "原始识别已取消，调用已收尾（{}）", self.code)
    }
}
impl std::error::Error for RawRecognitionCancelled {}

#[derive(Default)]
struct ControlState {
    phase: RawRecognitionPhase,
    started: Option<Instant>,
    requested: Option<(Instant, RawRecognitionPhase)>,
    options: Option<Arc<RunOptions>>,
    terminate_requested: bool,
    native_run_returned: bool,
    native_error: Option<String>,
    terminate_error: Option<String>,
}

// 一个对象只控制一次调用；克隆交给停止线程，终态之前不能释放原生会话。
#[derive(Clone, Default)]
pub struct RawRecognitionControl {
    state: Arc<Mutex<ControlState>>,
}

impl RawRecognitionControl {
    fn lock(&self) -> Result<MutexGuard<'_, ControlState>> {
        self.state
            .lock()
            .map_err(|_| anyhow::anyhow!("原始识别取消锁失效"))
    }

    /// 只请求取消；识别调用返回或状态为 Finished 才表示原生调用与数据已收尾。
    pub fn cancel(&self) -> Result<()> {
        let mut state = self.lock()?;
        if state.phase == RawRecognitionPhase::Finished || state.requested.is_some() {
            return Ok(());
        }
        state.requested = Some((Instant::now(), state.phase));
        if let Some(options) = state.options.as_ref() {
            let result = options.terminate();
            state.terminate_requested = true;
            if let Err(error) = result {
                state.terminate_error = Some(error.to_string());
                return Err(error.into());
            }
        }
        Ok(())
    }

    /// 观察当前阶段，不能用 terminate_requested 推断原生推理已经结束。
    pub fn status(&self) -> Result<RawRecognitionStatus> {
        let state = self.lock()?;
        Ok(RawRecognitionStatus {
            phase: state.phase,
            cancel_requested: state.requested.is_some(),
            terminate_requested: state.terminate_requested,
            native_run_returned: state.native_run_returned,
        })
    }

    pub(crate) fn checkpoint(&self, phase: RawRecognitionPhase) -> Result<()> {
        let mut state = self.lock()?;
        ensure!(state.requested.is_none(), "原始识别已收到取消请求");
        state.phase = phase;
        Ok(())
    }

    pub(crate) fn start_inference(&self) -> Result<Arc<RunOptions>> {
        let options = Arc::new(RunOptions::new()?);
        let mut state = self.lock()?;
        ensure!(state.requested.is_none(), "原始识别已收到取消请求");
        state.options = Some(options.clone());
        state.phase = RawRecognitionPhase::Inference;
        Ok(options)
    }

    pub(crate) fn end_inference(&self, native_error: Option<String>) -> Result<()> {
        let mut state = self.lock()?;
        // 只有 run_with_options 真正返回后才移除停止入口。
        state.options = None;
        state.native_run_returned = true;
        state.native_error = native_error;
        Ok(())
    }

    pub(crate) fn execute<T>(&self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        {
            let mut state = self.lock()?;
            ensure!(state.started.is_none(), "原始识别控制对象只能使用一次");
            state.started = Some(Instant::now());
        }
        let result = operation();
        let mut state = self.lock()?;
        state.options = None;
        state.phase = RawRecognitionPhase::Finished;
        if let Some((requested, requested_phase)) = state.requested {
            return Err(RawRecognitionCancelled {
                code: "raw_recognition_cancelled",
                requested_phase,
                elapsed_us: state.started.unwrap().elapsed().as_micros() as u64,
                stop_us: requested.elapsed().as_micros() as u64,
                terminate_requested: state.terminate_requested,
                native_run_returned: state.native_run_returned,
                native_error: state.native_error.clone(),
                terminate_error: state.terminate_error.clone(),
                operation_error: result.err().map(|error| format!("{error:#}")),
            }
            .into());
        }
        result
    }
}
