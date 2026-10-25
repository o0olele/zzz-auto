use crate::{InferenceDevice, NativeSession};
use anyhow::{Result, ensure};
use od_core::{CaptureMethod, InputMode, WindowInfo};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub struct GameSessionConfig<'a> {
    pub window: &'a WindowInfo,
    pub resources: &'a Path,
    pub capture: CaptureMethod,
    pub input: InputMode,
    pub device: InferenceDevice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionHandoff {
    Reused,
    Created,
    Cancelled,
}

impl NativeSession {
    /// 停止并等候 Tasker 和控制器队列；释放失败时保留原实例，不可交给下个执行器。
    pub fn quiesce(&mut self) -> Result<()> {
        let stopped = if self.tasker.running() {
            let result = self.stop_pipeline();
            self.wait_tasker_idle();
            result.map(|_| ())
        } else {
            Ok(())
        };
        let released = self.release_inputs();
        let errors: Vec<_> = [stopped, released]
            .into_iter()
            .filter_map(Result::err)
            .map(|error| format!("{error:#}"))
            .collect();
        ensure!(
            errors.is_empty(),
            "控制器交接收尾失败：{}",
            errors.join("；")
        );
        ensure!(
            !self.tasker.running()
                && !self.recognition_tasker.running()
                && !self.input_cleanup_pending
                && !self.pipeline_controller_pending
                && self.held_keys.is_empty()
                && self.held_contacts.is_empty(),
            "控制器交接缺少完整释放证明"
        );
        Ok(())
    }

    /// 同一工作线程的唯一原生槽位；方式或设备不兼容时，先销毁旧实例再创建。
    pub fn handoff_game(
        slot: &mut Option<Self>,
        config: GameSessionConfig<'_>,
        cancel: &AtomicBool,
    ) -> Result<SessionHandoff> {
        if let Some(previous) = slot.as_mut() {
            previous.quiesce()?;
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(SessionHandoff::Cancelled);
        }
        let current = od_platform::parse_game_window(&config.window.hwnd)?;
        ensure!(
            current.pid == config.window.pid && current.width == 1920 && current.height == 1080,
            "交接时游戏窗口身份或尺寸已变化，请重连"
        );
        let root = config.resources.canonicalize()?;
        let compatible = slot.as_ref().is_some_and(|previous| {
            previous.input_mode == Some(config.input)
                && previous.capture_method == Some(config.capture)
                && previous.inference_device == config.device
                && previous.resource_root == root
                && previous
                    .game_window
                    .as_ref()
                    .is_some_and(|old| old.pid == current.pid && old.hwnd == current.hwnd)
        });
        if cancel.load(Ordering::Acquire) {
            return Ok(SessionHandoff::Cancelled);
        }
        if compatible {
            return Ok(SessionHandoff::Reused);
        }
        // Drop 同步结束原控制器、推理会话及设备租约，不能与新控制器或 GPU 路径重叠。
        drop(slot.take());
        if cancel.load(Ordering::Acquire) {
            return Ok(SessionHandoff::Cancelled);
        }
        *slot = Some(Self::for_game(
            &current,
            &root,
            config.capture,
            config.input,
            config.device,
        )?);
        Ok(if cancel.load(Ordering::Acquire) {
            SessionHandoff::Cancelled
        } else {
            SessionHandoff::Created
        })
    }
}
