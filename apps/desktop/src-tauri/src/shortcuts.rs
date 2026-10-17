use od_core::{
    RunRecord,
    shortcuts::{ShortcutAction, ShortcutSettings, ShortcutStatus},
};
use od_platform::shortcuts::GlobalHotkeys;
use od_protocol::{Command, Payload};
use std::sync::{Mutex, OnceLock};

pub struct DesktopShortcuts {
    worker: OnceLock<GlobalHotkeys>,
    attempted: tokio::sync::Mutex<Option<ShortcutSettings>>,
    status: Mutex<ShortcutStatus>,
    target: Mutex<Option<RunRecord>>,
}

impl DesktopShortcuts {
    pub fn new() -> Self {
        Self {
            worker: OnceLock::new(),
            attempted: tokio::sync::Mutex::new(None),
            status: Mutex::new(ShortcutStatus::default()),
            target: Mutex::new(None),
        }
    }

    pub fn start(&self, callback: impl Fn(ShortcutAction) + Send + 'static) {
        match GlobalHotkeys::start(callback) {
            Ok(worker) => {
                let _ = self.worker.set(worker);
            }
            Err(error) => {
                self.status.lock().expect("快捷键状态锁").registration_error =
                    Some(error.to_string())
            }
        }
    }

    pub fn snapshot(&self) -> ShortcutStatus {
        self.status.lock().expect("快捷键状态锁").clone()
    }

    pub fn close(&self) {
        self.clear_target();
        if let Some(worker) = self.worker.get() {
            worker.close();
        }
    }

    pub async fn configure(&self, settings: &ShortcutSettings, retry: bool) {
        let mut attempted = self.attempted.lock().await;
        if !retry && attempted.as_ref() == Some(settings) {
            return;
        }
        *attempted = Some(settings.clone());
        let result = match self.worker.get() {
            Some(worker) => worker.configure(settings.clone()).await,
            None => Err("全局快捷键线程不可用，请重启桌面".into()),
        };
        let mut status = self.status.lock().expect("快捷键状态锁");
        match result {
            Ok(()) => {
                status.active = Some(settings.clone());
                status.registration_error = None;
                status.action_error = None;
            }
            Err(error) => {
                status.active = None;
                status.registration_error = Some(error);
            }
        }
    }

    pub fn observe(&self, result: &Result<Payload, od_protocol::ApiError>) {
        let mut target = self.target.lock().expect("快捷键目标锁");
        match result {
            Ok(Payload::Snapshot(snapshot)) => *target = snapshot.run.clone(),
            Ok(Payload::Run(run)) => *target = Some(run.clone()),
            Err(error)
                if error.code == "engine_offline" || error.code.starts_with("input_recovery") =>
            {
                *target = None
            }
            _ => {}
        }
    }

    pub fn clear_target(&self) {
        self.target.lock().expect("快捷键目标锁").take();
    }

    pub fn command(&self, action: ShortcutAction) -> Option<Command> {
        od_protocol::shortcut_command(action, self.target.lock().expect("快捷键目标锁").as_ref())
    }

    pub fn control_result(&self, error: Option<String>) {
        self.status.lock().expect("快捷键状态锁").action_error = error;
    }
}
