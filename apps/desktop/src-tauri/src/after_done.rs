use od_core::{
    RunRecord, Snapshot,
    after_done::{AfterDoneAction, AfterDoneStatus, AfterDoneTracker},
};
use od_protocol::{ApiError, Command};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use uuid::Uuid;

#[derive(Default)]
pub struct DesktopAfterDone {
    tracker: Arc<Mutex<AfterDoneTracker>>,
    target: Mutex<Option<od_platform::after_done::AfterDoneTarget>>,
}

impl DesktopAfterDone {
    pub fn snapshot(&self) -> Option<AfterDoneStatus> {
        self.tracker
            .lock()
            .expect("结束后动作锁")
            .snapshot(Instant::now())
    }
    pub fn require_available(&self) -> Result<(), ApiError> {
        if self.tracker.lock().expect("结束后动作锁").busy() {
            Err(ApiError::new(
                "after_done_busy",
                "结束后动作正在倒计时或执行，请先取消并等待完成",
            ))
        } else {
            Ok(())
        }
    }
    pub fn require_command(&self, command: &Command) -> Result<(), ApiError> {
        if matches!(
            command,
            Command::Snapshot
                | Command::AccountConfig
                | Command::ConfigBackups
                | Command::RunHistory { .. }
                | Command::QueryRunHistory { .. }
                | Command::ExportRunHistory { .. }
                | Command::Events { .. }
                | Command::Applications
                | Command::ModelResources
                | Command::RetryRunSave
                | Command::Stop { .. }
        ) {
            Ok(())
        } else {
            self.require_available()
        }
    }
    pub fn cancel(&self, id: Uuid) -> Result<Option<AfterDoneStatus>, ApiError> {
        self.tracker
            .lock()
            .expect("结束后动作锁")
            .cancel(id)
            .map_err(|e| ApiError::new("after_done_conflict", e))?;
        Ok(self.snapshot())
    }
    pub fn invalidate(&self, message: &str) {
        self.tracker
            .lock()
            .expect("结束后动作锁")
            .invalidate(message);
    }
    pub fn stop_requested(&self, run_id: Uuid) {
        let mut tracker = self.tracker.lock().expect("结束后动作锁");
        if tracker
            .snapshot(Instant::now())
            .is_some_and(|status| status.run_id == run_id)
        {
            tracker.invalidate("已请求停止任务，结束后动作已取消");
        }
    }
    pub fn arm(
        &self,
        session: Uuid,
        action: AfterDoneAction,
        run: &RunRecord,
        target: Option<od_platform::after_done::AfterDoneTarget>,
    ) {
        let mut tracker = self.tracker.lock().expect("结束后动作锁");
        if tracker.arm(session, action, run, Instant::now()).is_ok() {
            *self.target.lock().expect("结束后目标锁") = target;
        }
    }
    pub fn observe(&self, session: Uuid, snapshot: &Snapshot) -> bool {
        let mut tracker = self.tracker.lock().expect("结束后动作锁");
        let was_busy = tracker.busy();
        if self
            .target
            .lock()
            .expect("结束后目标锁")
            .as_ref()
            .is_some_and(|target| {
                snapshot.window.as_ref().is_none_or(|window| {
                    window.pid != target.window.pid || window.hwnd != target.window.hwnd
                })
            })
        {
            tracker.invalidate("游戏连接目标已变化，结束后动作已取消");
            return false;
        }
        tracker.observe(session, snapshot, Instant::now());
        !was_busy && tracker.busy()
    }
    pub fn submit_due(&self, runtime: &tokio::runtime::Handle) {
        let target = self.target.lock().expect("结束后目标锁").clone();
        self.submit_due_with(runtime, Instant::now(), move |action| {
            let target = target.ok_or_else(|| "本次动作缺少游戏进程身份".to_string())?;
            od_platform::after_done::execute_after_done(action, &target)
                .map_err(|e| format!("{e:#}"))
        });
    }
    pub fn submit_due_with(
        &self,
        runtime: &tokio::runtime::Handle,
        now: Instant,
        execute: impl FnOnce(AfterDoneAction) -> Result<(), String> + Send + 'static,
    ) {
        let Some(status) = self.tracker.lock().expect("结束后动作锁").take_due(now) else {
            return;
        };
        let tracker = self.tracker.clone();
        runtime.spawn_blocking(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(status.action)))
                    .unwrap_or_else(|_| Err("结束后动作执行异常，请手动检查 Windows 状态".into()));
            tracker
                .lock()
                .expect("结束后动作锁")
                .finish(status.id, result);
        });
    }
}
