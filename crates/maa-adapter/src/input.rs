use crate::{InputProbeAction, InputProbeConfig, InputProbeResult, NativeSession};
use anyhow::{Result, ensure};
use maa_framework::MaaStatus;
use od_core::{InputSide, MouseButton};
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

fn touch_args(action: InputProbeAction) -> Option<(i32, i32, i32, i32)> {
    match action {
        InputProbeAction::Key { .. } => None,
        InputProbeAction::MouseButton { button, x, y } => Some((
            match button {
                MouseButton::Left => 0,
                MouseButton::Right => 1,
                MouseButton::Middle => 2,
                MouseButton::X1 => 3,
                MouseButton::X2 => 4,
            },
            x,
            y,
            0,
        )),
        InputProbeAction::Stick { stick, x, y } => {
            Some((if stick == InputSide::Left { 0 } else { 1 }, x, y, 0))
        }
        InputProbeAction::Trigger { trigger, value } => {
            Some((if trigger == InputSide::Left { 2 } else { 3 }, 0, 0, value))
        }
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
        for contact in self.held_contacts.clone() {
            let result = self
                .controller
                .post_touch_up(contact)
                .and_then(|id| self.wait_input_job(id, cancel, stopped_at));
            match result {
                Ok(status) if status.is_success() => {
                    self.held_contacts.remove(&contact);
                }
                _ => errors.push(format!("鼠标 / 手柄通道 {contact} 释放失败")),
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
        if self.input_cleanup_pending
            || !self.held_keys.is_empty()
            || !self.held_contacts.is_empty()
        {
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
            if matches!(config.action, InputProbeAction::MouseButton { .. }) {
                // Maa 首次触摸会初始化坐标缩放；失败时可能使用 (0, 0)。
                // 先确认真实尺寸并初始化缓存，不能让截图失败变成错误位置的点击。
                let id = self.controller.post_screencap()?;
                ensure!(
                    self.wait_input_job(id, &cancel, &mut stopped_at)?
                        .is_success(),
                    "鼠标输入前截图失败，未发送输入"
                );
                let image = self.controller.cached_image()?;
                ensure!(
                    image.width() == od_core::BASE_WIDTH && image.height() == od_core::BASE_HEIGHT,
                    "鼠标输入前截图须为 1920×1080，未发送输入"
                );
            }
            if let Some(window) = &self.game_window
                && config.mode.is_gamepad()
            {
                let current = od_platform::parse_game_window(&window.hwnd)?;
                ensure!(
                    current.pid == window.pid && current.width == 1920 && current.height == 1080,
                    "游戏窗口身份或尺寸已变化，请重新连接"
                );
            }
            if let Some(window) = &self.game_window
                && !config.mode.is_gamepad()
            {
                od_platform::check_keyboard_target(
                    window,
                    matches!(
                        config.mode,
                        crate::InputMode::Foreground | crate::InputMode::ForegroundLegacy
                    ),
                )?;
            }
            // 权限与前台检查可能耗时；检查期间停止也不能再投递按下。
            if observe_cancel(&cancel, &mut stopped_at) {
                return Ok(());
            }
            // 投递前登记；即使回调返回失败，也可能已经按下，必须尝试释放。
            self.input_cleanup_pending = true;
            let down_at = Instant::now();
            let id = match config.action {
                InputProbeAction::Key { key } => {
                    self.held_keys.insert(key);
                    self.controller.post_key_down(key)?
                }
                action => {
                    let (contact, x, y, pressure) =
                        touch_args(action).expect("非按键动作有触摸参数");
                    self.held_contacts.insert(contact);
                    self.controller.post_touch_down(contact, x, y, pressure)?
                }
            };
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
