use crate::session::{EngineSession, offline};
use od_protocol::{ApiError, Command, EnginePrivilege, Payload};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

pub struct Host {
    pub screen_samples: std::sync::Arc<std::sync::Mutex<od_core::screen_samples::ScreenSampleBook>>,
    pub run_notifications: std::sync::Mutex<od_core::run_notifications::RunNotificationBook>,
    pub after_done: crate::after_done::DesktopAfterDone,
    session: crate::resources::SharedSession,
    pub resources: crate::resources::ResourceManager,
    pub config_recovery: crate::config_recovery::ConfigRecoveryManager,
    pub channel: std::sync::Mutex<Option<tauri::ipc::Channel<od_protocol::DesktopUpdate>>>,
    pub shortcuts: crate::shortcuts::DesktopShortcuts,
    pub logs: std::sync::Mutex<od_core::run_logs::LogBook>,
}

fn layout(app: &AppHandle) -> Result<od_platform::DesktopLayout, ApiError> {
    let development_root: Option<PathBuf> = if cfg!(debug_assertions) {
        Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../..")
                .canonicalize()
                .map_err(offline)?,
        )
    } else {
        None
    };
    od_platform::desktop_layout(
        &std::env::current_exe().map_err(offline)?,
        &app.path().resource_dir().map_err(offline)?,
        &app.path().app_data_dir().map_err(offline)?,
        development_root.as_deref(),
    )
    .map_err(offline)
}

impl Host {
    pub fn new() -> Self {
        let session = std::sync::Arc::new(Mutex::new(None));
        let runtime = tauri::async_runtime::handle().inner().clone();
        let resources = crate::resources::ResourceManager::new(session.clone(), runtime.clone());
        Self {
            screen_samples: std::sync::Arc::new(std::sync::Mutex::new(
                od_core::screen_samples::ScreenSampleBook::default(),
            )),
            run_notifications: std::sync::Mutex::new(
                od_core::run_notifications::RunNotificationBook::default(),
            ),
            after_done: crate::after_done::DesktopAfterDone::default(),
            config_recovery: crate::config_recovery::ConfigRecoveryManager::new(
                resources.clone(),
                session.clone(),
                runtime,
            ),
            resources,
            session,
            channel: std::sync::Mutex::new(None),
            shortcuts: crate::shortcuts::DesktopShortcuts::new(),
            logs: std::sync::Mutex::new(od_core::run_logs::LogBook::default()),
        }
    }

    pub async fn start(&self, app: &AppHandle) -> Result<(), ApiError> {
        self.after_done.require_available()?;
        self.after_done.invalidate("引擎连接改变，结束后动作已取消");
        self.resources.require_available()?;
        let mut state = self.session.lock().await;
        self.resources.require_available()?;
        if let Some(session) = state.as_mut() {
            if session.connected() && !session.exited()? {
                return Ok(());
            }
            if !session.exited()? {
                return Err(ApiError::new(
                    "engine_stopping",
                    "旧引擎仍在收尾，退出前不能重连",
                ));
            }
            session.require_recovered()?;
        }
        state.take();
        self.notifications_disconnected();
        self.shortcuts.clear_target();
        *state = Some(EngineSession::launch(&layout(app)?, false).await?);
        Ok(())
    }

    pub async fn elevate(&self, app: &AppHandle) -> Result<(), ApiError> {
        self.after_done.require_available()?;
        self.after_done.invalidate("引擎权限改变，结束后动作已取消");
        self.resources.require_available()?;
        let mut state = self.session.lock().await;
        self.resources.require_available()?;
        let current = state.as_mut().ok_or_else(|| offline("请先重连普通引擎"))?;
        if !current.connected() || current.exited()? {
            return Err(offline("请先重连普通引擎"));
        }
        if current.privilege.engine_integrity >= 0x3000 {
            return Ok(());
        }
        let layout = layout(app)?;
        let candidate = current
            .switch_to(EngineSession::launch(&layout, true))
            .await?;
        *state = Some(candidate);
        self.notifications_disconnected();
        self.shortcuts.clear_target();
        Ok(())
    }

    pub async fn request(&self, command: Command) -> Result<Payload, ApiError> {
        self.after_done.require_command(&command)?;
        self.resources.require_available()?;
        let mut state = self.session.lock().await;
        self.after_done.require_command(&command)?;
        self.resources.require_available()?;
        let session = state
            .as_mut()
            .ok_or_else(|| offline("引擎未连接，使用“重连引擎”重新启动"))?;
        let after = if let Command::StartGroupTrial {
            revision,
            instance_id,
            ..
        } = &command
        {
            let Payload::AccountConfig(config) = session.request(Command::AccountConfig).await?
            else {
                return Err(offline("无法读取结束后动作设置"));
            };
            if config.revision != *revision {
                return Err(ApiError::new(
                    "config_conflict",
                    "配置已变化，请重新读取并预览",
                ));
            }
            let action = config.general.after_done;
            let notifications = config
                .instances
                .iter()
                .find(|instance| instance.id == *instance_id)
                .ok_or_else(|| ApiError::new("account_changed", "账号不存在，请重新预览"))?
                .run_notifications;
            let Payload::Snapshot(snapshot) = session.request(Command::Snapshot).await? else {
                return Err(offline("无法确认运行状态"));
            };
            self.run_notifications.lock().expect("运行提醒锁").observe(
                session.id,
                &snapshot,
                std::time::SystemTime::now().into(),
            );
            let target = if action != od_core::after_done::AfterDoneAction::None {
                let window = snapshot.window.ok_or_else(|| offline("请先连接游戏窗口"))?;
                Some(od_platform::after_done::AfterDoneTarget::capture(window).map_err(offline)?)
            } else {
                None
            };
            Some((action, target, notifications))
        } else {
            None
        };
        if let Command::Stop { run_id } = &command {
            self.after_done.stop_requested(*run_id);
        }
        if matches!(&command, Command::LaunchGame { .. }) {
            self.after_done
                .invalidate("已请求启动游戏，上一任务的结束后动作已取消");
        }
        let result = session.request(command).await;
        if let (Some((action, target, settings)), Ok(Payload::Run(run))) = (after, &result) {
            self.after_done.arm(session.id, action, run, target);
            self.run_notifications.lock().expect("运行提醒锁").arm(
                session.id,
                run,
                settings,
                std::time::SystemTime::now().into(),
            );
        }
        if result
            .as_ref()
            .err()
            .is_some_and(|e| e.code == "engine_offline" || e.code.starts_with("input_recovery"))
        {
            self.after_done
                .invalidate("引擎连接或输入恢复异常，结束后动作已取消");
        }
        if let Ok(Payload::Snapshot(snapshot)) = &result {
            self.run_notifications.lock().expect("运行提醒锁").observe(
                session.id,
                snapshot,
                std::time::SystemTime::now().into(),
            );
        }
        self.observe_logs(session.id, &result);
        self.shortcuts.observe(&result);
        if let Ok(Payload::AccountConfig(config)) = &result {
            self.shortcuts
                .configure(&config.general.shortcuts, false)
                .await;
        }
        result
    }

    pub async fn control_shortcut(&self, command: Command, at: std::time::Instant) {
        let result = async {
            self.resources.require_available()?;
            let mut state = self.session.lock().await;
            self.resources.require_available()?;
            if at.elapsed() > std::time::Duration::from_secs(2) {
                return Err(ApiError::new(
                    "shortcut_expired",
                    "快捷键等待过久，已丢弃，请重新按键",
                ));
            }
            if let Command::Stop { run_id } = &command {
                self.after_done.stop_requested(*run_id);
            }
            state
                .as_mut()
                .ok_or_else(|| offline("引擎未连接"))?
                .request(command)
                .await
        }
        .await;
        self.shortcuts.observe(&result);
        self.shortcuts
            .control_result(result.err().map(|error| error.message));
    }

    pub async fn retry_shortcuts(&self) -> Result<od_core::shortcuts::ShortcutStatus, ApiError> {
        self.resources.require_available()?;
        let mut state = self.session.lock().await;
        self.resources.require_available()?;
        let session = state.as_mut().ok_or_else(|| offline("引擎未连接"))?;
        // 在同一会话锁内预检与注册，不能在预检后插入新任务。
        crate::session::require_idle(session.request(Command::Snapshot).await?)?;
        let Payload::AccountConfig(config) = session.request(Command::AccountConfig).await? else {
            return Err(ApiError::new("config_error", "无法读取任务快捷键设置"));
        };
        self.shortcuts
            .configure(&config.general.shortcuts, true)
            .await;
        Ok(self.shortcuts.snapshot())
    }

    pub async fn poll(
        &self,
        app: &AppHandle,
    ) -> (Option<EnginePrivilege>, Result<Payload, ApiError>) {
        let mut state = self.session.lock().await;
        let Some(session) = state.as_mut() else {
            self.notifications_disconnected();
            self.after_done.invalidate("引擎未连接，结束后动作已取消");
            self.shortcuts.clear_target();
            let recovery = layout(app).and_then(|layout| {
                crate::session::recovery::RecoveryJournal::open(&layout.data)?.require_clean()
            });
            if let Err(error) = recovery {
                return (None, Err(error));
            }
            return (None, Err(offline("引擎未连接，使用“重连引擎”重新启动")));
        };
        // 快照与权限来自同一会话，不能跨权限切换混合读取。
        let privilege = session.privilege.clone();
        let result = session.request(Command::Snapshot).await;
        if let Ok(Payload::Snapshot(snapshot)) = &result {
            self.run_notifications.lock().expect("运行提醒锁").observe(
                session.id,
                snapshot,
                std::time::SystemTime::now().into(),
            );
            let began = self.after_done.observe(session.id, snapshot);
            if self.after_done.require_available().is_err() {
                let models_idle = matches!(session.request(Command::ModelResources).await, Ok(Payload::ModelResources(resources)) if resources.job.as_ref().is_none_or(|job| job.state.is_terminal()));
                if self.resources.require_available().is_err()
                    || session.require_recovered().is_err()
                    || !models_idle
                {
                    self.after_done
                        .invalidate("资源工作或输入收尾未完成，结束后动作已取消");
                } else {
                    if began && let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                    self.after_done
                        .submit_due(tauri::async_runtime::handle().inner());
                }
            }
        } else {
            self.notifications_disconnected();
            self.after_done.invalidate("引擎连接中断，结束后动作已取消");
        }
        self.observe_logs(session.id, &result);
        self.shortcuts.observe(&result);
        if result.is_ok() {
            if let Ok(Payload::AccountConfig(config)) =
                session.request(Command::AccountConfig).await
            {
                self.shortcuts
                    .configure(&config.general.shortcuts, false)
                    .await;
            }
        } else {
            self.shortcuts.clear_target();
        }
        (Some(privilege), result)
    }

    pub async fn recover(&self, app: &AppHandle) -> Result<String, ApiError> {
        self.after_done.require_available()?;
        self.after_done.invalidate("正在恢复输入，结束后动作已取消");
        self.resources.require_available()?;
        let mut state = self.session.lock().await;
        self.resources.require_available()?;
        let layout = layout(app)?;
        let journal = crate::session::recovery::RecoveryJournal::open(&layout.data)?;
        if journal.plan.is_none() {
            if let Some(session) = state.as_mut() {
                session.reload_recovery(&layout)?;
            }
            return Ok("已确认没有待恢复的输入记录，请重新检查游戏状态".into());
        }
        if let Some(session) = state.as_mut()
            && !session.exited()?
        {
            return Err(crate::session::recovery::recovery_error(
                "旧引擎尚未退出，请先停止任务或等待断线收尾",
            ));
        }
        let message = EngineSession::recover(&layout).await?;
        if let Some(session) = state.as_mut() {
            session.reload_recovery(&layout)?;
        }
        Ok(message)
    }

    pub fn report_error(&self, error: ApiError) {
        if let Ok(channel) = self.channel.lock()
            && let Some(channel) = channel.as_ref()
        {
            let _ = channel.send(od_protocol::DesktopUpdate::Offline(error));
        }
    }

    fn observe_logs(&self, session_id: uuid::Uuid, result: &Result<Payload, ApiError>) {
        if let Ok(Payload::Snapshot(snapshot)) = result
            && let Ok(mut logs) = self.logs.lock()
        {
            logs.observe(session_id, snapshot);
        }
    }

    pub fn log_state(
        &self,
        session_id: Option<uuid::Uuid>,
    ) -> Result<od_core::run_logs::DesktopLogs, ApiError> {
        self.logs
            .lock()
            .map_err(|_| ApiError::new("log_error", "日志缓存不可用"))?
            .view(session_id)
            .map_err(|error| ApiError::new("log_error", error.to_string()))
    }

    pub async fn export_logs(
        &self,
        app: &AppHandle,
        session_id: uuid::Uuid,
    ) -> Result<od_core::run_logs::LogExport, ApiError> {
        let session = self
            .log_state(Some(session_id))?
            .selected
            .ok_or_else(|| ApiError::new("log_error", "该会话没有可导出的日志"))?;
        let data = layout(app)?.data;
        tokio::task::spawn_blocking(move || od_core::run_logs::export(&data, &session))
            .await
            .map_err(|error| ApiError::new("log_export_failed", error.to_string()))?
            .map_err(|error| ApiError::new("log_export_failed", format!("{error:#}")))
    }

    pub async fn capture_sample(
        &self,
        hwnd: &str,
        method: od_core::CaptureMethod,
    ) -> Result<od_core::screen_samples::ScreenSampleState, ApiError> {
        self.after_done.require_available()?;
        self.resources.require_available()?;
        let mut state = self.session.lock().await;
        self.after_done.require_available()?;
        self.resources.require_available()?;
        let session = state.as_mut().ok_or_else(|| offline("请先连接游戏窗口"))?;
        crate::screen_samples::capture(session, &self.screen_samples, hwnd, method).await
    }

    pub async fn save_sample(
        &self,
        app: &AppHandle,
        id: uuid::Uuid,
        kind: od_core::screen_samples::ScreenSampleKind,
        note: String,
    ) -> Result<od_core::screen_samples::ScreenSampleState, ApiError> {
        let data = layout(app)?.data;
        let cache = self.screen_samples.clone();
        tokio::task::spawn_blocking(move || {
            cache
                .lock()
                .map_err(|_| crate::screen_samples::failed("素材缓存不可用"))?
                .save(&data, id, kind, &note)
                .map_err(crate::screen_samples::failed)
        })
        .await
        .map_err(crate::screen_samples::failed)?
    }

    pub async fn shutdown(&self) -> Result<(), ApiError> {
        self.after_done.invalidate("桌面正在退出，结束后动作已取消");
        self.after_done.require_available()?;
        self.shortcuts.clear_target();
        self.resources.close_and_wait().await;
        let mut state = self.session.lock().await;
        if let Some(session) = state.as_mut()
            && let Err(error) = session.shutdown().await
        {
            self.resources.reopen();
            return Err(error);
        }
        state.take();
        // 等待正在写入的本机素材收尾，并使排队保存的旧标识失效。
        crate::screen_samples::clear_and_wait(self.screen_samples.clone()).await?;
        self.run_notifications.lock().expect("运行提醒锁").closing();
        Ok(())
    }

    pub fn notification_state(&self) -> od_core::run_notifications::RunNotifications {
        self.run_notifications
            .lock()
            .expect("运行提醒锁")
            .snapshot()
    }
    pub fn notifications_disconnected(&self) {
        self.run_notifications
            .lock()
            .expect("运行提醒锁")
            .disconnected(std::time::SystemTime::now().into());
    }
    pub fn update_notifications(
        &self,
        ids: &[uuid::Uuid],
        remove: bool,
    ) -> Result<od_core::run_notifications::RunNotifications, ApiError> {
        self.run_notifications
            .lock()
            .expect("运行提醒锁")
            .update(ids, remove)
            .map_err(|error| ApiError::new("notification_error", error))
    }

    pub fn start_resources(
        &self,
        app: &AppHandle,
        request: od_core::runtime_resources::RuntimeResourceRequest,
    ) -> Result<od_core::runtime_resources::DesktopRuntimeResources, ApiError> {
        self.after_done.require_available()?;
        self.after_done.invalidate("正在操作资源，结束后动作已取消");
        self.resources.start(layout(app)?, request)
    }

    pub fn download_resources(
        &self,
        app: &AppHandle,
        source: od_core::runtime_resources::RuntimeDownloadSource,
    ) -> Result<od_core::runtime_resources::DesktopRuntimeResources, ApiError> {
        self.after_done.require_available()?;
        self.after_done.invalidate("正在下载资源，结束后动作已取消");
        self.resources.start_download(layout(app)?, source)
    }

    pub fn start_config_recovery(
        &self,
        app: &AppHandle,
        request: od_core::config::ConfigRecoveryRequest,
    ) -> Result<od_core::config::DesktopConfigRecovery, ApiError> {
        self.after_done.require_available()?;
        self.after_done.invalidate("正在恢复配置，结束后动作已取消");
        self.config_recovery.start(layout(app)?, request)
    }
}
