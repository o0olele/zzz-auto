use crate::{NativeSession, SDK_VERSION};
use anyhow::{Result, ensure};
use maa_framework::{MaaStatus, job::Job};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

// 由可信调用方声明全部路径可能持有的输入，不从可丢弃的展示事件推断。
#[derive(Debug, Clone, Default)]
pub struct PipelineInputPlan {
    pub keys: BTreeSet<i32>,
    pub contacts: BTreeSet<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineEnd {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Serialize)]
pub struct PipelineProbeResult {
    pub sdk_version: String,
    pub entry: String,
    pub end: PipelineEnd,
    pub native_status: String,
    pub stop_status: Option<String>,
    pub enqueue_us: u64,
    pub elapsed_ms: u64,
    pub release_ms: u64,
    pub stop_ms: Option<u64>,
}

impl NativeSession {
    // 仅供固定图与可反应模拟控制器验证。游戏入口待业务资源和输入清单验收后接入。
    pub fn probe_fixture_pipeline(
        &mut self,
        entry: &str,
        inputs: &PipelineInputPlan,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<PipelineProbeResult> {
        ensure!(
            self.game_window.is_none(),
            "离线 Pipeline 探针不能使用游戏控制器"
        );
        self.run_pipeline(entry, inputs, cancel, timeout)
    }

    fn wait_tasker_idle(&self) {
        // 任务状态可能先于 Tasker 收尾更新，不能只检查原任务的终态。
        while self.tasker.running() {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn stop_pipeline(&self) -> Result<MaaStatus> {
        let id = self.tasker.post_stop()?;
        ensure!(id > 0, "Maa 拒绝 Pipeline 停止请求");
        let status = Job::for_tasker(&self.tasker, id).wait();
        self.wait_tasker_idle();
        ensure!(status.is_success(), "Maa Pipeline 停止失败：{status}");
        Ok(status)
    }

    pub(crate) fn run_pipeline(
        &mut self,
        entry: &str,
        inputs: &PipelineInputPlan,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<PipelineProbeResult> {
        ensure!(!entry.is_empty(), "Pipeline 入口不能为空");
        ensure!(!timeout.is_zero(), "Pipeline 超时必须大于零");
        ensure!(
            inputs.keys.iter().all(|key| *key >= 0)
                && inputs.contacts.iter().all(|contact| *contact >= 0),
            "Pipeline 输入清单不能包含负数"
        );
        ensure!(
            !self.tasker.running(),
            "Maa Tasker 仍在运行，不能启动另一个 Pipeline"
        );
        if self.input_cleanup_pending
            || !self.held_keys.is_empty()
            || !self.held_contacts.is_empty()
        {
            self.release_inputs()?;
        }
        let started = Instant::now();
        let mut stopped_at = None;
        let mut report = PipelineProbeResult {
            sdk_version: SDK_VERSION.into(),
            entry: entry.into(),
            end: PipelineEnd::Succeeded,
            native_status: "NotStarted".into(),
            stop_status: None,
            enqueue_us: 0,
            elapsed_ms: 0,
            release_ms: 0,
            stop_ms: None,
        };
        let execution = (|| -> Result<()> {
            if cancel.load(Ordering::Acquire) {
                stopped_at = Some(Instant::now());
                report.end = PipelineEnd::Cancelled;
                return Ok(());
            }
            // 即使原生按下失败或 auto_up 未报告释放错误，也保留显式释放责任。
            self.held_keys.extend(&inputs.keys);
            self.held_contacts.extend(&inputs.contacts);
            self.input_cleanup_pending = true;
            self.pipeline_controller_pending = true;
            let enqueue_at = Instant::now();
            let job = self.tasker.post_task(entry, "{}")?;
            report.enqueue_us = enqueue_at.elapsed().as_micros() as u64;
            ensure!(job.id > 0, "Maa 拒绝 Pipeline 启动请求");
            loop {
                let status = job.status();
                ensure!(
                    status != MaaStatus::INVALID,
                    "Maa 返回无效 Pipeline 任务 ID"
                );
                if status.done() && !self.tasker.running() {
                    break;
                }
                if stopped_at.is_none() {
                    // 同时出现取消与超时时，优先保留主动停止的含义。
                    let cancelled = cancel.load(Ordering::Acquire);
                    if cancelled || started.elapsed() >= timeout {
                        stopped_at = Some(Instant::now());
                        report.end = if cancelled {
                            PipelineEnd::Cancelled
                        } else {
                            PipelineEnd::TimedOut
                        };
                        report.stop_status = Some(self.stop_pipeline()?.to_string());
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let status = job.wait();
            report.native_status = status.to_string();
            if stopped_at.is_none() {
                report.end = if status.is_success() {
                    PipelineEnd::Succeeded
                } else {
                    PipelineEnd::Failed
                };
            }
            Ok(())
        })();
        // SDK 接口错误也先收尾。原生阻塞时继续等待，不伪造已释放的终态。
        let settle = if execution.is_err() && self.tasker.running() {
            let result = self.stop_pipeline();
            self.wait_tasker_idle();
            result.map(|_| ())
        } else {
            Ok(())
        };
        let release_at = Instant::now();
        let cleanup = self.release_inputs_observed(&cancel, &mut stopped_at);
        report.release_ms = release_at.elapsed().as_millis() as u64;
        report.elapsed_ms = started.elapsed().as_millis() as u64;
        report.stop_ms = stopped_at.map(|at| at.elapsed().as_millis() as u64);
        if report.end == PipelineEnd::Succeeded && stopped_at.is_some() {
            report.end = PipelineEnd::Cancelled;
        }
        let mut errors: Vec<_> = [execution, settle, cleanup]
            .into_iter()
            .filter_map(Result::err)
            .map(|error| format!("{error:#}"))
            .collect();
        if !errors.is_empty() && report.end == PipelineEnd::Failed {
            errors.insert(0, format!("Maa Pipeline 失败：{}", report.native_status));
        }
        ensure!(errors.is_empty(), "{}", errors.join("；"));
        Ok(report)
    }
}
