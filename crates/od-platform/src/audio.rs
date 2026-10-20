//! WASAPI 默认输出设备回录；COM 对象始终在专用线程创建与销毁。
use anyhow::{Context, Result, ensure};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT, IAudioCaptureClient,
};

struct Shared {
    samples: VecDeque<f64>,
    error: Option<String>,
    dropped: u64,
    last_packet: Option<Instant>,
}
pub struct AudioLoopback {
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<Shared>>,
    worker: Option<JoinHandle<()>>,
}

impl AudioLoopback {
    pub fn start() -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(Shared {
            samples: VecDeque::new(),
            error: None,
            dropped: 0,
            last_packet: None,
        }));
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let worker_stop = stop.clone();
        let worker_state = state.clone();
        let worker = std::thread::Builder::new()
            .name("od-output-audio".into())
            .spawn(move || {
                let result = capture(&worker_stop, &worker_state, &tx);
                if let Err(error) = result {
                    let message = format!("声音回录停止：{error:#}");
                    worker_state.lock().expect("音频锁失效").error = Some(message.clone());
                    let _ = tx.try_send(Err(message));
                }
            })?;
        let mut capture = Self {
            stop,
            state,
            worker: Some(worker),
        };
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(capture),
            result => {
                capture.stop.store(true, Ordering::Release);
                if let Some(worker) = capture.worker.take() {
                    let _ = worker.join();
                }
                anyhow::bail!("WASAPI 初始化失败：{result:?}");
            }
        }
    }
    pub fn drain(&self) -> Result<(Vec<f64>, u64)> {
        let mut state = self.state.lock().expect("音频锁失效");
        if let Some(error) = &state.error {
            anyhow::bail!("{error}");
        }
        // 停顿后不把缓存的旧提示音当作新事件。
        if state
            .last_packet
            .is_some_and(|at| at.elapsed() > Duration::from_millis(150))
        {
            state.dropped += state.samples.len() as u64 + 1;
            state.samples.clear();
            state.last_packet = None;
        }
        let samples = state.samples.drain(..).collect();
        let dropped = std::mem::take(&mut state.dropped);
        Ok((samples, dropped))
    }
}
impl Drop for AudioLoopback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn capture(
    stop: &AtomicBool,
    shared: &Mutex<Shared>,
    ready: &std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<()> {
    wasapi::initialize_mta().ok().context("初始化音频 COM")?;
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            wasapi::deinitialize();
        }
    }
    let _com = Com;
    let enumerator = DeviceEnumerator::new()?;
    let device = enumerator.get_default_device(&Direction::Render)?;
    let mut client = device.get_iaudioclient()?;
    // 由系统在共享模式下完成输出混音与 32 kHz Float32 双声道转换。
    let format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        od_core::audio_dodge::SAMPLE_RATE as usize,
        2,
        None,
    );
    client.initialize_client(
        &format,
        &Direction::Capture,
        &StreamMode::PollingShared {
            autoconvert: true,
            buffer_duration_hns: 1_000_000,
        },
    )?;
    // wasapi 0.26 的高层读取未处理 SILENT 的空数据指针；保留其客户端管理，
    // 通过正式暴露的 IAudioClient 读取数据包并显式处理静音与断续。
    let reader: IAudioCaptureClient = unsafe { client.as_iaudioclient().GetService()? };
    client.start_stream()?;
    let result = (|| -> Result<()> {
        let _ = ready.send(Ok(()));
        let mut bytes = VecDeque::new();
        while !stop.load(Ordering::Acquire) {
            while unsafe { reader.GetNextPacketSize()? } > 0 {
                let mut data = std::ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                unsafe {
                    reader.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                }
                let packet = (|| -> Result<()> {
                    ensure!(frames <= 32000, "WASAPI 数据包超过一秒");
                    if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                        bytes.clear();
                        let mut state = shared.lock().expect("音频锁失效");
                        state.dropped += state.samples.len() as u64 + 1;
                        state.samples.clear();
                    }
                    if frames == 0 {
                        return Ok(());
                    }
                    if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                        bytes.extend(std::iter::repeat_n(0, frames as usize * 8));
                    } else {
                        ensure!(!data.is_null(), "WASAPI 非静音包缺失数据");
                        bytes.extend(unsafe {
                            std::slice::from_raw_parts(data, frames as usize * 8)
                        });
                    }
                    Ok(())
                })();
                let released = unsafe { reader.ReleaseBuffer(frames) }.map_err(anyhow::Error::from);
                packet.and(released)?;
                ensure!(bytes.len() <= 32000 * 8, "音频积压超过一秒");
                if stop.load(Ordering::Acquire) {
                    break;
                }
            }
            if !bytes.is_empty() {
                ensure!(bytes.len().is_multiple_of(8), "WASAPI 返回不完整帧");
                let raw: Vec<_> = bytes.drain(..).collect();
                let floats: Vec<_> = raw
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
                let samples = od_core::audio_dodge::resample_mono(&floats, 2, 32000)?;
                let mut state = shared.lock().expect("音频锁失效");
                if state
                    .last_packet
                    .is_some_and(|at| at.elapsed() > Duration::from_millis(150))
                {
                    state.dropped += state.samples.len() as u64 + 1;
                    state.samples.clear();
                }
                state.samples.extend(samples);
                if state.samples.len() > 16000 {
                    let discard = state.samples.len() - 16000;
                    state.samples.drain(..discard);
                    state.dropped += discard as u64;
                }
                state.last_packet = Some(Instant::now());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    })();
    let stopped = client.stop_stream().map_err(anyhow::Error::from);
    result.and(stopped)
}
