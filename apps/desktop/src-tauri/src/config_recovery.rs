use crate::{
    resources::{MaintenanceGuard, ResourceManager, SharedSession},
    session::{EngineSession, recovery::RecoveryJournal},
};
use od_core::config::{
    ConfigError, ConfigRecovery, ConfigRecoveryJob, ConfigRecoveryJobState, ConfigRecoveryRequest,
    DesktopConfigRecovery,
};
use od_platform::DesktopLayout;
use od_protocol::{ApiError, Command, Payload};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct ConfigRecoveryManager {
    state: Arc<Mutex<DesktopConfigRecovery>>,
    resources: ResourceManager,
    session: SharedSession,
    runtime: tokio::runtime::Handle,
}

fn file_error(error: ConfigError) -> ApiError {
    let code = match &error {
        ConfigError::InUse => "config_recovery_in_use",
        ConfigError::Locked => "config_recovery_locked",
        _ => "config_recovery_failed",
    };
    ApiError::new(code, error.to_string())
}

impl ConfigRecoveryManager {
    pub fn new(
        resources: ResourceManager,
        session: SharedSession,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(DesktopConfigRecovery::default())),
            resources,
            session,
            runtime,
        }
    }

    pub fn snapshot(&self) -> DesktopConfigRecovery {
        self.state.lock().expect("配置恢复状态锁失效").clone()
    }

    pub fn start(
        &self,
        layout: DesktopLayout,
        request: ConfigRecoveryRequest,
    ) -> Result<DesktopConfigRecovery, ApiError> {
        request.validate().map_err(file_error)?;
        let guard = self.resources.maintenance()?;
        let mut state = self.state.lock().expect("配置恢复状态锁失效");
        // 恢复期间不可取消或重入；请求方关闭页面后宿主仍持有工作与退出栅栏。
        state.preview = None;
        state.restored = None;
        if matches!(request, ConfigRecoveryRequest::List) {
            state.catalog = None;
        }
        state.job = Some(ConfigRecoveryJob {
            id: Uuid::new_v4(),
            request: request.clone(),
            state: ConfigRecoveryJobState::Running,
            phase: "waiting_engine".into(),
            reconnected: false,
            reconnect_error: None,
            error: None,
            error_code: None,
        });
        let initial = state.clone();
        let manager = self.clone();
        self.runtime.spawn(async move {
            let result = manager.run(layout, request, &guard).await;
            let mut state = manager.state.lock().expect("配置恢复状态锁失效");
            let job = state.job.as_mut().expect("恢复任务已创建");
            job.phase = "finished".into();
            job.state = if result.is_ok() {
                ConfigRecoveryJobState::Succeeded
            } else {
                ConfigRecoveryJobState::Failed
            };
            if let Err(error) = result {
                job.error_code = Some(error.code);
                job.error = Some(error.message);
            }
            // 先保存可查询终态，再释放栅栏；退出等待不会提前结束。
            drop(state);
            drop(guard);
        });
        Ok(initial)
    }

    fn phase(&self, phase: &str) {
        if let Some(job) = &mut self.state.lock().expect("配置恢复状态锁失效").job {
            job.phase = phase.into();
        }
    }

    async fn run(
        &self,
        layout: DesktopLayout,
        request: ConfigRecoveryRequest,
        guard: &MaintenanceGuard,
    ) -> Result<(), ApiError> {
        let mut session = self.session.lock().await;
        guard.require_open()?;
        RecoveryJournal::open(&layout.data)?.require_clean()?;
        if let Some(current) = session.as_mut() {
            if !current.exited()? {
                if current.connected() {
                    match current.request(Command::Snapshot).await {
                        Ok(_) => {
                            return Err(ApiError::new(
                                "config_recovery_engine_connected",
                                "引擎可正常连接，请使用账号与应用组中的配置备份恢复",
                            ));
                        }
                        Err(error)
                            if matches!(
                                error.code.as_str(),
                                "engine_init_failed" | "engine_offline"
                            ) =>
                        {
                            current.disconnect();
                        }
                        Err(error) => return Err(error),
                    }
                }
                current.wait_exit(Duration::from_secs(15)).await?;
            }
            current.reload_recovery(&layout)?;
        }
        RecoveryJournal::open(&layout.data)?.require_clean()?;
        session.take();
        guard.require_open()?;
        self.phase("processing_files");
        let data = layout.data.clone();
        let result = tokio::task::spawn_blocking(move || {
            let recovery = ConfigRecovery::new(&data).map_err(file_error)?;
            match request {
                ConfigRecoveryRequest::List => recovery
                    .catalog()
                    .map(|catalog| (Some(catalog), None, None)),
                ConfigRecoveryRequest::Preview { id } => recovery
                    .preview(&id)
                    .map(|preview| (None, Some(preview), None)),
                ConfigRecoveryRequest::Restore {
                    id,
                    preview_id,
                    confirm: _,
                } => recovery
                    .restore(&id, &preview_id)
                    .map(|restored| (None, None, Some(restored))),
            }
            .map_err(file_error)
        })
        .await
        .map_err(|_| {
            ApiError::new(
                "config_recovery_failed",
                "配置恢复工作线程异常，请重新读取配置核对结果",
            )
        })??;
        let (catalog, preview, restored) = result;
        let config = restored.as_ref().map(|result| result.config.clone());
        {
            let mut state = self.state.lock().expect("配置恢复状态锁失效");
            if let Some(catalog) = catalog {
                state.catalog = Some(catalog);
            }
            state.preview = preview;
            state.restored = restored;
        }
        if let Some(config) = config
            && guard.require_open().is_ok()
        {
            self.phase("starting_engine");
            let restarted = async {
                let next = EngineSession::launch(&layout, false).await?;
                *session = Some(next);
                let current = session.as_mut().expect("新引擎已保存");
                current.request(Command::Snapshot).await?;
                match current.request(Command::AccountConfig).await? {
                    Payload::AccountConfig(loaded) if loaded == config => Ok(()),
                    _ => Err(ApiError::new(
                        "config_recovery_restart_failed",
                        "引擎读取的配置与恢复结果不同，请重新读取核对",
                    )),
                }
            }
            .await;
            let mut state = self.state.lock().expect("配置恢复状态锁失效");
            let job = state.job.as_mut().expect("恢复任务已创建");
            match restarted {
                Ok(()) => job.reconnected = true,
                Err(error) => {
                    job.reconnect_error = Some(format!(
                        "配置已恢复，引擎重连失败：{}。请手动重连，不要重复恢复",
                        error.message
                    ))
                }
            }
        }
        Ok(())
    }
}
