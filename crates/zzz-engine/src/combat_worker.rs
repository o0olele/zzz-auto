//! 单个只读识别线程；在途一帧，待处理只保留最新一帧，不发送输入。
use crate::combat_performance::PerformanceTrace;
use anyhow::{Context, Result};
use maa_adapter::{
    CombatVision,
    combat_vision::{CombatFrame, CombatObservation, CombatRecognitionPlan, CombatRecognizer},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

struct Work {
    frame: CombatFrame,
    at: f64,
    epoch: u64,
    intervals: BTreeMap<String, f64>,
    dodge_only: bool,
    refresh_agents: bool,
    trace: Option<PerformanceTrace>,
    submitted: std::time::Instant,
    plan: Option<Arc<CombatRecognitionPlan>>,
}
pub(crate) struct Observed {
    pub frame: CombatFrame,
    pub at: f64,
    pub observation: CombatObservation,
    epoch: u64,
    completed: std::time::Instant,
}
pub(crate) struct EarlyFlash {
    pub states: Vec<od_core::conditional::StateUpdate>,
    pub at: f64,
    pub captured: std::time::Instant,
    epoch: u64,
}
enum Message {
    Flash(EarlyFlash),
    Complete(Result<Observed>),
}
pub(crate) struct CombatWorker {
    sender: Option<SyncSender<Work>>,
    receiver: Receiver<Message>,
    handle: Option<JoinHandle<CombatRecognizer>>,
    cancel: Arc<AtomicBool>,
    raw: Arc<std::sync::Mutex<Option<maa_adapter::RawRecognitionControl>>>,
    pending: Option<(CombatFrame, f64)>,
    busy: bool,
    epoch: u64,
    refresh_agents: bool,
    trace: Option<PerformanceTrace>,
    plan: Option<Arc<CombatRecognitionPlan>>,
}
impl CombatWorker {
    pub fn new(
        reader: impl CombatVision + Send + 'static,
        mut recognizer: CombatRecognizer,
    ) -> Result<Self> {
        let (sender, input) = mpsc::sync_channel::<Work>(1);
        let (output, receiver) = mpsc::sync_channel(2);
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let raw = recognizer.cancellation_handle();
        let handle = thread::Builder::new()
            .name("combat-recognition".into())
            .spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let work = match input.recv_timeout(Duration::from_millis(5)) {
                        Ok(work) => work,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    if work.refresh_agents {
                        recognizer.refresh_agents();
                    }
                    if let Some(plan) = work.plan {
                        recognizer.set_recognition_plan(plan);
                    }
                    let recognition_started = std::time::Instant::now();
                    if let Some(trace) = &work.trace {
                        trace.event("recognition_start", json!({
                            "frame_id": trace.stamp(work.frame.captured), "epoch": work.epoch,
                            "queue_wait_us": work.submitted.elapsed().as_micros() as u64,
                            "submit_age_us": work.submitted.saturating_duration_since(work.frame.captured).as_micros() as u64,
                            "frame_age_us": work.frame.captured.elapsed().as_micros() as u64,
                            "intervals": work.intervals, "refresh_agents": work.refresh_agents,
                        }));
                    }
                    let flash_output = output.clone();
                    let flash_trace = work.trace.clone();
                    let (at, epoch, captured) = (work.at, work.epoch, work.frame.captured);
                    recognizer.set_flash_observer(Some(Arc::new(move |states| {
                        if let Some(trace) = &flash_trace {
                            trace.event("flash_ready", json!({
                                "frame_id": trace.stamp(captured), "epoch": epoch,
                                "frame_age_us": captured.elapsed().as_micros() as u64,
                            }));
                        }
                        let _ = flash_output.send(Message::Flash(EarlyFlash {
                            states: states.to_vec(),
                            at,
                            epoch,
                            captured,
                        }));
                    })));
                    let result = recognizer
                        .observe(
                            &reader,
                            &work.frame,
                            work.at,
                            &work.intervals,
                            work.dodge_only,
                            &stop,
                        )
                        .map(|mut observation| {
                            // 闪光已经单独投递，同一帧的完整结果不能再次触发。
                            observation.states.retain(|state| {
                                state.name != "闪避识别-红光" && state.name != "闪避识别-黄光"
                            });
                            Observed {
                                frame: work.frame,
                                at: work.at,
                                epoch: work.epoch,
                                observation,
                                completed: std::time::Instant::now(),
                            }
                        });
                    recognizer.set_flash_observer(None);
                    if let Some(trace) = &work.trace {
                        match &result {
                            Ok(result) => trace.event("recognition_complete", json!({
                                "frame_id": trace.stamp(result.frame.captured), "epoch": result.epoch,
                                "recognition_us": recognition_started.elapsed().as_micros() as u64,
                                "frame_age_us": result.frame.captured.elapsed().as_micros() as u64,
                                "stages": result.frame.performance(),
                                "in_battle": result.observation.in_battle, "team": result.observation.team,
                                "states": result.observation.states.len(),
                            })),
                            Err(error) => trace.event("recognition_error", json!({"frame_id": trace.stamp(captured), "epoch": epoch, "error": format!("{error:#}")})),
                        }
                    }
                    if output.send(Message::Complete(result)).is_err() {
                        break;
                    }
                }
                recognizer
            })?;
        Ok(Self {
            sender: Some(sender),
            receiver,
            handle: Some(handle),
            cancel,
            raw,
            pending: None,
            busy: false,
            epoch: 0,
            refresh_agents: false,
            trace: None,
            plan: None,
        })
    }
    pub fn set_trace(&mut self, trace: Option<PerformanceTrace>) {
        self.trace = trace;
    }
    pub fn set_recognition_plan(&mut self, plan: Arc<CombatRecognitionPlan>) {
        self.plan = Some(plan);
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn capture(&mut self, mut frame: CombatFrame, at: f64) {
        if let Some(trace) = &self.trace {
            frame.enable_performance();
            if let Some((old, _)) = &self.pending {
                trace.event(
                    "pending_replaced",
                    json!({"frame_id": trace.stamp(old.captured)}),
                );
            }
        }
        self.pending = Some((frame, at));
    }
    pub fn needs_submission(&self) -> bool {
        !self.busy && self.pending.is_some()
    }
    pub fn submit(&mut self, intervals: BTreeMap<String, f64>, dodge_only: bool) -> Result<()> {
        if self.busy {
            return Ok(());
        }
        if let Some((frame, at)) = self.pending.take() {
            self.sender
                .as_ref()
                .context("战斗工作线程已关闭")?
                .try_send(Work {
                    frame,
                    at,
                    epoch: self.epoch,
                    intervals,
                    dodge_only,
                    refresh_agents: std::mem::take(&mut self.refresh_agents),
                    trace: self.trace.clone(),
                    submitted: std::time::Instant::now(),
                    plan: self.plan.clone(),
                })
                .map_err(|_| anyhow::anyhow!("战斗识别任务投递失败"))?;
            self.busy = true;
        }
        Ok(())
    }
    pub fn poll(&mut self, early: &mut Vec<EarlyFlash>) -> Result<Option<Observed>> {
        loop {
            match self.receiver.try_recv() {
                Ok(Message::Flash(flash)) => {
                    if flash.epoch == self.epoch {
                        early.push(flash);
                    }
                }
                Ok(Message::Complete(result)) => {
                    self.busy = false;
                    let observed = result?;
                    if let Some(trace) = &self.trace {
                        trace.event("recognition_consumed", json!({
                            "frame_id": trace.stamp(observed.frame.captured), "epoch": observed.epoch,
                            "accepted_epoch": observed.epoch == self.epoch,
                            "receive_wait_us": observed.completed.elapsed().as_micros() as u64,
                            "frame_age_us": observed.frame.captured.elapsed().as_micros() as u64,
                        }));
                    }
                    return Ok((observed.epoch == self.epoch).then_some(observed));
                }
                Err(TryRecvError::Empty) => return Ok(None),
                Err(TryRecvError::Disconnected) => anyhow::bail!("战斗识别工作线程已异常退出"),
            }
        }
    }
    pub fn refresh_agents(&mut self) {
        if let Some(trace) = &self.trace {
            trace.event("epoch_refresh", json!({"old_epoch": self.epoch}));
        }
        self.epoch += 1;
        self.pending = None;
        self.refresh_agents = true;
    }
    fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.pending = None;
        self.sender.take();
        if let Ok(raw) = self.raw.lock()
            && let Some(raw) = raw.as_ref()
        {
            let _ = raw.cancel();
        }
    }
    pub fn finish(mut self) -> Result<CombatRecognizer> {
        self.cancel();
        let handle = self.handle.take().context("战斗工作线程已收尾")?;
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("战斗识别工作线程发生异常"))
    }
}
impl Drop for CombatWorker {
    fn drop(&mut self) {
        self.cancel();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
