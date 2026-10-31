//! 有界、非阻塞的战斗诊断。热路径不等待磁盘；逐帧统计独立于 UI 的事件环。
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const MAX_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct PerformanceTrace {
    sender: SyncSender<Value>,
    dropped: Arc<AtomicU64>,
    started: Instant,
}
impl PerformanceTrace {
    pub fn span<'a>(
        &'a self,
        kind: &'static str,
        data: Value,
        threshold_us: u64,
    ) -> PerformanceSpan<'a> {
        PerformanceSpan {
            trace: self,
            kind,
            data,
            threshold_us,
            started: Instant::now(),
        }
    }
    pub fn stamp(&self, at: Instant) -> u64 {
        at.saturating_duration_since(self.started).as_micros() as u64
    }

    pub fn event(&self, kind: &str, data: Value) {
        let event = json!({"event": kind, "at_us": self.stamp(Instant::now()), "data": data});
        if self.sender.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) struct PerformanceSpan<'a> {
    trace: &'a PerformanceTrace,
    kind: &'static str,
    data: Value,
    threshold_us: u64,
    started: Instant,
}
impl Drop for PerformanceSpan<'_> {
    fn drop(&mut self) {
        let us = self.started.elapsed().as_micros() as u64;
        if us >= self.threshold_us {
            self.data["duration_us"] = json!(us);
            self.data["started_us"] = json!(self.trace.stamp(self.started));
            self.trace.event(self.kind, std::mem::take(&mut self.data));
        }
    }
}

pub(crate) struct PerformanceLog {
    trace: Option<PerformanceTrace>,
    writer: Option<JoinHandle<()>>,
}

pub(crate) struct LoopPerformance<'a> {
    trace: &'a PerformanceTrace,
    started: Instant,
    iterations: u64,
    sleep_us: u64,
    max_sleep_us: u64,
    deferred: BTreeMap<&'static str, u64>,
}
impl<'a> LoopPerformance<'a> {
    pub fn new(trace: &'a PerformanceTrace) -> Self {
        Self {
            trace,
            started: Instant::now(),
            iterations: 0,
            sleep_us: 0,
            max_sleep_us: 0,
            deferred: BTreeMap::new(),
        }
    }
    pub fn iteration(&mut self) {
        if self.started.elapsed() >= Duration::from_secs(1) {
            self.flush();
        }
        self.iterations += 1;
    }
    pub fn defer(&mut self, reason: &'static str) {
        *self.deferred.entry(reason).or_default() += 1;
    }
    pub fn sleep(&mut self, us: u64) {
        self.sleep_us += us;
        self.max_sleep_us = self.max_sleep_us.max(us);
    }
    fn flush(&mut self) {
        if self.iterations > 0 {
            self.trace.event(
                "loop_window",
                json!({
                    "duration_us": self.started.elapsed().as_micros() as u64,
                    "iterations": self.iterations, "sleep_us": self.sleep_us,
                    "max_sleep_us": self.max_sleep_us, "capture_deferred_iterations": self.deferred,
                }),
            );
        }
        self.started = Instant::now();
        self.iterations = 0;
        self.sleep_us = 0;
        self.max_sleep_us = 0;
        self.deferred.clear();
    }
}
impl Drop for LoopPerformance<'_> {
    fn drop(&mut self) {
        self.flush();
    }
}
impl PerformanceLog {
    pub fn start(data: &Path, id: uuid::Uuid, metadata: Value) -> Result<(Self, PathBuf)> {
        let directory = data.join("logs/combat-performance");
        od_core::config::reject_links(&directory)?;
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{id}.jsonl"));
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        let (sender, receiver) = mpsc::sync_channel(2048);
        let dropped = Arc::new(AtomicU64::new(0));
        let started = Instant::now();
        let trace = PerformanceTrace {
            sender,
            dropped: dropped.clone(),
            started,
        };
        // 文件打开和线程创建失败可在开始战斗前显示；写入错误只停诊断，不影响输入收尾。
        let writer = std::thread::Builder::new()
            .name("combat-performance".into())
            .spawn(move || {
                if let Err(error) = write_log(file, receiver, &dropped, started) {
                    eprintln!("战斗性能日志写入失败：{error}");
                }
            })?;
        trace.event(
            "session",
            json!({
                "schema_version": 1, "run_id": id, "started_at": chrono::Utc::now(),
                "pid": std::process::id(), "debug_assertions": cfg!(debug_assertions),
                "engine_version": env!("CARGO_PKG_VERSION"), "metadata": metadata,
                "max_bytes": MAX_BYTES, "queue_capacity": 2048,
                "clock": "monotonic microseconds; frame_id is capture start on this clock",
            }),
        );
        Ok((
            Self {
                trace: Some(trace),
                writer: Some(writer),
            },
            path,
        ))
    }

    pub fn trace(&self) -> PerformanceTrace {
        self.trace.as_ref().expect("诊断会话已关闭").clone()
    }
}
impl Drop for PerformanceLog {
    fn drop(&mut self) {
        self.trace.take();
        // 调用方须先结束识别线程并释放输入；所有 trace 克隆已退出后才收尾。
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

fn line(writer: &mut impl Write, value: &Value) -> std::io::Result<usize> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    writer.write_all(&bytes)?;
    Ok(bytes.len())
}

fn write_log(
    file: File,
    receiver: mpsc::Receiver<Value>,
    dropped: &AtomicU64,
    started: Instant,
) -> std::io::Result<()> {
    let mut writer = BufWriter::with_capacity(64 * 1024, file);
    let mut bytes = 0_u64;
    let mut omitted = 0_u64;
    let mut counts = BTreeMap::<String, u64>::new();
    let mut last_flush = Instant::now();
    loop {
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(event) => {
                *counts
                    .entry(event["event"].as_str().unwrap_or("unknown").into())
                    .or_default() += 1;
                // 为 footer 留出空间；达到上限后仍排空通道并报告省略数量。
                if bytes + 64 * 1024 < MAX_BYTES {
                    bytes += line(&mut writer, &event)? as u64;
                } else {
                    omitted += 1;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if last_flush.elapsed() >= Duration::from_secs(1) {
            writer.flush()?;
            last_flush = Instant::now();
        }
    }
    line(
        &mut writer,
        &json!({
            "event": "footer", "at_us": started.elapsed().as_micros() as u64,
            "data": {"events": counts, "queue_dropped": dropped.load(Ordering::Relaxed),
                "size_limit_omitted": omitted, "detail_bytes": bytes},
        }),
    )?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_queue_never_waits_and_reports_loss() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let trace = PerformanceTrace {
            sender,
            dropped: Arc::new(AtomicU64::new(0)),
            started: Instant::now(),
        };
        trace.event("first", json!({}));
        trace.event("second", json!({}));
        assert_eq!(trace.dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn log_survives_pause_and_flushes_complete_json_on_close() -> Result<()> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()?
            .join("artifacts/combat-performance-tests");
        let id = uuid::Uuid::new_v4();
        let (log, path) = PerformanceLog::start(&root, id, json!({"script": "test"}))?;
        {
            let trace = log.trace();
            trace.event("paused", json!({"frame_id": 123}));
            trace.event("resumed", json!({"frame_id": 456}));
        }
        drop(log);
        let events: Vec<Value> = std::fs::read_to_string(path)?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        assert_eq!(events[0]["data"]["run_id"], id.to_string());
        assert_eq!(events.last().unwrap()["data"]["events"]["resumed"], 1);
        assert_eq!(events.last().unwrap()["data"]["queue_dropped"], 0);
        assert_eq!(events.last().unwrap()["event"], "footer");
        Ok(())
    }
}
