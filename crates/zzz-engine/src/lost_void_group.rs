//! 统一原生线程持有实际迷失之地应用；摘要不能重建此对象。
use super::*;
use crate::lost_void_application::{LostVoidApplicationEnd, LostVoidApplicationRun};
use anyhow::{Context, Result, ensure};
use od_core::{
    lost_void_application::LostVoidAppEnd,
    pipeline::{Interrupt, PipelineControl},
};

pub(super) struct LostVoidWork {
    id: Uuid,
    run: LostVoidApplicationRun,
    control: PipelineControl,
    window: WindowInfo,
    capture: CaptureMethod,
    fixture: bool,
    error: Option<String>,
}
impl LostVoidWork {
    pub(super) fn new(
        run: LostVoidApplicationRun,
        control: PipelineControl,
        window: WindowInfo,
        capture: CaptureMethod,
        fixture: bool,
    ) -> Self {
        Self {
            id: run.snapshot().run_id,
            run,
            control,
            window,
            capture,
            fixture,
            error: None,
        }
    }
    pub(super) fn verify(&self) -> Result<()> {
        self.run.verify_prepared()
    }
    pub(super) fn control(&mut self, resume: bool) {
        if !resume || self.control.requested() == Some(Interrupt::Stop) {
            self.control.stop();
        } else if let Err(error) = self.run.resume_after_parent_pause() {
            self.error = Some(format!("迷失之地继续失败：{error:#}"));
        }
    }
}

pub(super) fn run_lost_void_work(
    mut work: Box<LostVoidWork>,
    paths: &EnginePaths,
    shared: &Arc<Mutex<State>>,
    native: &mut Option<NativeSession>,
) -> Option<Box<LostVoidWork>> {
    let id = work.id;
    let result = (|| -> Result<()> {
        ensure!(
            work.error.is_none(),
            "{}",
            work.error.as_deref().unwrap_or_default()
        );
        work.verify()?;
        // 初始或继续后尚未投递输入便收到请求，不为确认暂停而连接游戏。
        if work.control.requested().is_some() && work.run.snapshot().inputs_released {
            work.run.acknowledge_parent_interrupt()?;
            return Ok(());
        }
        if work.fixture {
            let session = native.as_mut().context("离线迷失之地控制器缺失")?;
            session.quiesce()?;
            session.check_combat_input_mode(InputMode::Foreground)?;
        } else if NativeSession::handoff_game(
            native,
            maa_adapter::GameSessionConfig {
                window: &work.window,
                resources: &paths.resources,
                capture: work.capture,
                input: InputMode::Foreground,
                device: InferenceDevice::Cpu,
            },
            &work.control.cancellation(),
        )? == maa_adapter::SessionHandoff::Cancelled
        {
            work.run.acknowledge_parent_interrupt()?;
            return Ok(());
        }
        work.run.execute_with_progress(
            native.as_mut().context("迷失之地控制器未连接")?,
            !work.fixture,
            Duration::from_millis(u64::from(
                od_core::lost_void_application::DEFAULT_APPLICATION_TIMEOUT_MS,
            )),
            |progress| {
                let mut state = shared.lock().expect("运行状态锁失效");
                let record = state
                    .slot
                    .update_lost_void(id, progress, "迷失之地执行中".into())
                    .map_err(anyhow::Error::msg)?;
                save_run(paths, &mut state, &record);
                ensure!(
                    state.record_error.is_none(),
                    "迷失之地检查点未保存，停止后续输入"
                );
                Ok(())
            },
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        work.error = Some(format!("迷失之地未完成：{error:#}"));
    }
    let released = match native.as_mut() {
        Some(session) => match session.quiesce() {
            Ok(()) => session.inputs_released(),
            Err(error) => {
                work.error = Some(format!("迷失之地输入收尾失败：{error:#}"));
                false
            }
        },
        None => work.run.snapshot().inputs_released,
    };
    // 请求处理也持有此锁；Ready 核验、撤销与最终分项交接共用一个临界区。
    let mut state = shared.lock().expect("运行状态锁失效");
    let current = state.slot.current()?;
    if current.id != id {
        return None;
    }
    match current.state {
        RunState::Stopping => {
            work.control.stop();
        }
        RunState::Pausing => {
            work.control.pause();
        }
        _ => {}
    }
    if work.control.requested().is_some()
        && work.error.is_none()
        && let Err(error) = work.run.acknowledge_parent_interrupt()
    {
        // 已经实际失败 / 超时的对象保留原性质，不能因并发请求恢复为可继续。
        if !work.run.progress().is_ok_and(|p| p.state.is_terminal()) {
            work.error = Some(format!("迷失之地父请求确认失败：{error:#}"));
        }
    }
    if work
        .run
        .progress()
        .is_ok_and(|p| p.end == Some(LostVoidAppEnd::Ready))
        && work.control.finish_if_uninterrupted().is_err()
        && let Err(error) = work.run.acknowledge_parent_interrupt()
    {
        work.error = Some(format!("迷失之地完成交接失败：{error:#}"));
    }
    let mut progress = match work.run.progress() {
        Ok(progress) => progress,
        Err(error) => {
            work.error = Some(format!("迷失之地摘要无效：{error:#}"));
            (**state.slot.current()?.lost_void.as_ref()?).clone()
        }
    };
    if work.error.is_some() || !released {
        progress.state = RunState::Failed;
        progress.end = Some(LostVoidAppEnd::Failed);
        progress.completion = None;
        progress.returned_world = false;
        progress.inputs_released = released;
    }
    let keep = progress.state == RunState::Paused;
    let message = work.error.clone().unwrap_or_else(|| {
        let ending = match progress.end {
            Some(LostVoidApplicationEnd::Ready) if progress.completed() => "完成",
            Some(LostVoidApplicationEnd::Paused) => "已暂停",
            Some(LostVoidApplicationEnd::Stopped) => "已停止",
            Some(LostVoidApplicationEnd::TimedOut) => "超时",
            _ => "失败",
        };
        format!(
            "迷失之地{ending}，已核实通关 {} 次",
            progress.successful_challenges
        )
    });
    match state.slot.update_lost_void(id, progress, message) {
        Ok(record) => {
            save_run(paths, &mut state, &record);
            state.event(
                Some(id),
                if record.state == RunState::Failed {
                    "error"
                } else {
                    "info"
                },
                record.message,
            );
        }
        Err(error) => {
            state.event(
                Some(id),
                "error",
                format!("迷失之地结果无法写入运行槽：{error}"),
            );
            if let Some(mut p) = state
                .slot
                .current()
                .and_then(|r| r.lost_void.as_deref())
                .cloned()
            {
                p.state = RunState::Failed;
                p.end = Some(LostVoidAppEnd::Failed);
                p.completion = None;
                p.returned_world = false;
                p.inputs_released = released;
                if let Ok(record) =
                    state
                        .slot
                        .update_lost_void(id, p, format!("迷失之地检查点无效：{error}"))
                {
                    save_run(paths, &mut state, &record);
                }
            }
            state.pipeline_control = None;
            return None;
        }
    }
    if keep {
        Some(work)
    } else {
        state.pipeline_control = None;
        None
    }
}
