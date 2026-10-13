use crate::{InputProbeConfig, InputProbeResult, NativeSession};
use anyhow::{Result, ensure};
use maa_framework::MaaStatus;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn observe_cancel(cancel: &AtomicBool, stopped_at: &mut Option<Instant>) -> bool {
    if cancel.load(Ordering::Acquire) && stopped_at.is_none() {
        *stopped_at = Some(Instant::now());
    }
    stopped_at.is_some()
}

fn wait_delay(duration: Duration, cancel: &AtomicBool, stopped_at: &mut Option<Instant>) {
    let until = Instant::now() + duration;
    while !observe_cancel(cancel, stopped_at) {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        std::thread::sleep(left.min(Duration::from_millis(5)));
    }
}

impl NativeSession {
    pub(crate) fn wait_input_job(
        &self,
        id: i64,
        cancel: &AtomicBool,
        stopped_at: &mut Option<Instant>,
    ) -> Result<MaaStatus> {
        ensure!(id > 0, "Maa 拒绝输入请求");
        loop {
            observe_cancel(cancel, stopped_at);
            let status = self.controller.status(id);
            ensure!(status != MaaStatus::INVALID, "Maa 返回无效输入 ID");
            if status.done() {
                return Ok(self.controller.wait(id));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(crate) fn release_inputs_observed(
        &mut self,
        cancel: &AtomicBool,
        stopped_at: &mut Option<Instant>,
    ) -> Result<()> {
        self.input_cleanup_pending = true;
        let mut errors = Vec::new();
        for key in self.held_keys.clone() {
            let result = self
                .controller
                .post_key_up(key)
                .and_then(|id| self.wait_input_job(id, cancel, stopped_at));
            match result {
                Ok(status) if status.is_success() => {
                    self.held_keys.remove(&key);
                }
                _ => errors.push(format!("按键 {key} 释放失败")),
            }
        }
        // 恢复放在队列末端，并等完成；任何失败都保留，不能交出成功终态。
        let result = self
            .controller
            .post_inactive()
            .and_then(|id| self.wait_input_job(id, cancel, stopped_at));
        if !matches!(result, Ok(status) if status.is_success()) {
            errors.push("控制器恢复输入失败".into());
        }
        ensure!(errors.is_empty(), "{}", errors.join("；"));
        self.input_cleanup_pending = false;
        Ok(())
    }

    pub fn input_probe(
        &mut self,
        config: InputProbeConfig,
        cancel: Arc<AtomicBool>,
    ) -> Result<InputProbeResult> {
        config.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.input_mode == Some(config.mode),
            "输入模式与当前控制器不匹配"
        );
        ensure!(
            !self.tasker.running(),
            "Maa Tasker 仍在运行，不能投递输入探针"
        );
        // 上轮释放失败时先重试清理，未确认释放前不能产生新动作。
        if self.input_cleanup_pending || !self.held_keys.is_empty() {
            self.release_inputs()?;
        }
        let started = Instant::now();
        let mut stopped_at = None;
        let mut report = InputProbeResult {
            config,
            pressed: false,
            released: false,
            stopped: false,
            enqueue_us: 0,
            key_down_ms: 0,
            held_ms: 0,
            release_ms: 0,
            stop_ms: None,
            elapsed_ms: 0,
        };
        wait_delay(
            Duration::from_millis(config.start_delay_ms.into()),
            &cancel,
            &mut stopped_at,
        );
        let action: Result<()> = (|| {
            if observe_cancel(&cancel, &mut stopped_at) {
                return Ok(());
            }
            // 投递前登记；即使回调返回失败，也可能已经按下，必须尝试释放。
            self.held_keys.insert(config.key);
            self.input_cleanup_pending = true;
            let down_at = Instant::now();
            let id = self.controller.post_key_down(config.key)?;
            report.enqueue_us = down_at.elapsed().as_micros() as u64;
            // 已投递动作无法因取消标记而跳过完成等待。
            let status = self.wait_input_job(id, &cancel, &mut stopped_at)?;
            report.key_down_ms = down_at.elapsed().as_millis() as u64;
            ensure!(status.is_success(), "Maa 按下失败：{status}");
            report.pressed = true;
            let held_at = Instant::now();
            wait_delay(
                Duration::from_millis(config.hold_ms.into()),
                &cancel,
                &mut stopped_at,
            );
            report.held_ms = held_at.elapsed().as_millis() as u64;
            Ok(())
        })();
        let release_at = Instant::now();
        let cleanup = self.release_inputs_observed(&cancel, &mut stopped_at);
        report.release_ms = release_at.elapsed().as_millis() as u64;
        observe_cancel(&cancel, &mut stopped_at);
        report.stopped = stopped_at.is_some();
        report.stop_ms = stopped_at.map(|at| at.elapsed().as_millis() as u64);
        report.elapsed_ms = started.elapsed().as_millis() as u64;
        // 两种失败都保留，不能让按下失败掩盖释放失败。
        match (action, cleanup) {
            (Ok(()), Ok(())) => {
                report.released = true;
                Ok(report)
            }
            (Err(action), Err(cleanup)) => {
                Err(anyhow::anyhow!("{action:#}；输入收尾失败：{cleanup:#}"))
            }
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    }
}
