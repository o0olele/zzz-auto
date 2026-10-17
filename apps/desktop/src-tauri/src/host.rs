use crate::session::{EngineSession, offline};
use od_protocol::{ApiError, Command, EnginePrivilege, Payload};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

pub struct Host {
    session: crate::resources::SharedSession,
    pub resources: crate::resources::ResourceManager,
    pub channel: std::sync::Mutex<Option<tauri::ipc::Channel<od_protocol::DesktopUpdate>>>,
    pub shortcuts: crate::shortcuts::DesktopShortcuts,
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
        Self {
            resources: crate::resources::ResourceManager::new(
                session.clone(),
                tauri::async_runtime::handle().inner().clone(),
            ),
            session,
            channel: std::sync::Mutex::new(None),
            shortcuts: crate::shortcuts::DesktopShortcuts::new(),
        }
    }

    pub async fn start(&self, app: &AppHandle) -> Result<(), ApiError> {
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
        self.shortcuts.clear_target();
        *state = Some(EngineSession::launch(&layout(app)?, false).await?);
        Ok(())
    }

    pub async fn elevate(&self, app: &AppHandle) -> Result<(), ApiError> {
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
        self.shortcuts.clear_target();
        Ok(())
    }

    pub async fn request(&self, command: Command) -> Result<Payload, ApiError> {
        self.resources.require_available()?;
        let mut state = self.session.lock().await;
        self.resources.require_available()?;
        let result = state
            .as_mut()
            .ok_or_else(|| offline("引擎未连接，使用“重连引擎”重新启动"))?
            .request(command)
            .await;
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

    pub async fn shutdown(&self) -> Result<(), ApiError> {
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
        Ok(())
    }

    pub fn start_resources(
        &self,
        app: &AppHandle,
        request: od_core::runtime_resources::RuntimeResourceRequest,
    ) -> Result<od_core::runtime_resources::DesktopRuntimeResources, ApiError> {
        self.resources.start(layout(app)?, request)
    }

    pub fn download_resources(
        &self,
        app: &AppHandle,
        source: od_core::runtime_resources::RuntimeDownloadSource,
    ) -> Result<od_core::runtime_resources::DesktopRuntimeResources, ApiError> {
        self.resources.start_download(layout(app)?, source)
    }
}
