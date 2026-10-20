use crate::{InputProbeAction, InputProbeConfig, NativeSession};
use anyhow::{Context, Result, ensure};
use std::sync::atomic::{AtomicBool, Ordering};

impl NativeSession {
    /// 一次只投递一个动作并等待原生完成，持有集合在投递前登记。
    pub fn combat_input(
        &mut self,
        action: InputProbeAction,
        pressed: bool,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        let mode = self.input_mode.context("战斗控制器未设置输入方式")?;
        InputProbeConfig {
            mode,
            action,
            hold_ms: 1,
            start_delay_ms: 0,
        }
        .validate()
        .map_err(anyhow::Error::msg)?;
        ensure!(
            !self.tasker.running(),
            "Pipeline 尚未退出，不能交接战斗输入"
        );
        if pressed && cancel.load(Ordering::Acquire) {
            return Ok(false);
        }
        if pressed {
            if let Some(window) = &self.game_window {
                if mode.is_gamepad() {
                    let current = od_platform::parse_game_window(&window.hwnd)?;
                    ensure!(
                        current.pid == window.pid
                            && current.width == 1920
                            && current.height == 1080,
                        "战斗游戏窗口身份或尺寸已改变"
                    );
                } else {
                    od_platform::check_keyboard_target(
                        window,
                        matches!(
                            mode,
                            crate::InputMode::Foreground | crate::InputMode::ForegroundLegacy
                        ),
                    )?;
                }
            }
            if matches!(action, InputProbeAction::MouseButton { .. }) {
                let image = self.controller.cached_image()?;
                ensure!(
                    image.width() == 1920 && image.height() == 1080,
                    "战斗鼠标输入前需有有效 1080p 截图"
                );
            }
        }
        self.input_cleanup_pending = true;
        let id = match action {
            InputProbeAction::Key { key } => {
                if pressed {
                    self.held_keys.insert(key);
                    self.controller.post_key_down(key)?
                } else {
                    self.controller.post_key_up(key)?
                }
            }
            _ => {
                let (contact, x, y, pressure) =
                    crate::input::touch_args(action).context("战斗输入通道无效")?;
                if pressed {
                    let already = self.held_contacts.contains(&contact);
                    self.held_contacts.insert(contact);
                    if already {
                        self.controller.post_touch_move(contact, x, y, pressure)?
                    } else {
                        self.controller.post_touch_down(contact, x, y, pressure)?
                    }
                } else {
                    self.controller.post_touch_up(contact)?
                }
            }
        };
        let status = self.wait_input_job(id, cancel, &mut None)?;
        ensure!(status.is_success(), "战斗原生输入执行失败");
        if !pressed {
            match action {
                InputProbeAction::Key { key } => {
                    self.held_keys.remove(&key);
                }
                _ => {
                    let (contact, ..) = crate::input::touch_args(action).unwrap();
                    self.held_contacts.remove(&contact);
                }
            }
        }
        Ok(true)
    }
    pub fn combat_turn(&mut self, dx: i32, cancel: &AtomicBool) -> Result<bool> {
        ensure!(
            dx != 0 && dx.unsigned_abs() <= 10000,
            "转向位移须为非零且不超过 10000"
        );
        let mode = self.input_mode.context("战斗控制器未设置输入方式")?;
        ensure!(!mode.is_gamepad(), "手柄转向须使用右摇杆");
        ensure!(!self.tasker.running(), "Pipeline 尚未退出");
        if cancel.load(Ordering::Acquire) {
            return Ok(false);
        }
        if let Some(window) = &self.game_window {
            od_platform::check_keyboard_target(window, true)?;
        }
        let id = self.controller.post_relative_move(dx, 0)?;
        ensure!(
            self.wait_input_job(id, cancel, &mut None)?.is_success(),
            "Maa 相对转向失败，请检查控制方式"
        );
        Ok(true)
    }
}
