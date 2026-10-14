use anyhow::{Result, ensure};
use maa_framework::buffer::MaaImageBuffer;
use serde::Serialize;
use std::sync::{Mutex, MutexGuard};
use std::{path::Path, time::Instant};

static GPU_SLOT: Mutex<()> = Mutex::new(());

pub struct RecognitionImage {
    pub(crate) buffer: MaaImageBuffer,
    pub prepare_us: u64,
}

impl RecognitionImage {
    pub fn load(image: &Path) -> Result<Self> {
        let started = Instant::now();
        let bytes = crate::fixture::read_png(image)?;
        let mut buffer = MaaImageBuffer::new()?;
        buffer.set_encoded(&bytes)?;
        ensure!(
            buffer.width() == od_core::BASE_WIDTH && buffer.height() == od_core::BASE_HEIGHT,
            "识别语料必须是 1920×1080"
        );
        Ok(Self {
            buffer,
            prepare_us: started.elapsed().as_micros() as u64,
        })
    }
}

// 同一进程的 Maa GPU 会话由一个持有者管理，留给补充识别复用。
pub struct GpuLease {
    _guard: MutexGuard<'static, ()>,
}

impl GpuLease {
    pub fn acquire(device_id: i32) -> Result<Self> {
        ensure!(device_id >= 0, "DirectML 设备编号必须为非负整数");
        Ok(Self {
            _guard: GPU_SLOT
                .try_lock()
                .map_err(|_| anyhow::anyhow!("已有 GPU 推理会话，请先等待原生会话销毁"))?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecognitionTiming {
    pub session_setup_us: u64,
    pub prepare_us: u64,
    pub enqueue_us: u64,
    pub completion_us: u64,
    pub details_us: u64,
    pub total_us: u64,
}

#[derive(Debug, Serialize)]
pub struct RecognitionMeasurement {
    pub detail: serde_json::Value,
    pub native_status: String,
    pub timing: RecognitionTiming,
}
