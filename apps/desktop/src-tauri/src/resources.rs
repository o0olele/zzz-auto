use crate::session::{EngineSession, offline, recovery::RecoveryJournal, require_idle};
use od_core::runtime_resources::{
    DesktopRuntimeResources, RuntimeDownloadSource, RuntimeResourceCatalog, RuntimeResourceJob,
    RuntimeResourceJobState, RuntimeResourceRequest,
};
use od_platform::DesktopLayout;
use od_protocol::{ApiError, Command, Payload};
use std::{
    os::windows::process::CommandExt,
    process::Command as ProcessCommand,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Mutex as AsyncMutex, watch};
use uuid::Uuid;

#[path = "resource_download.rs"]
mod download;
#[path = "resource_feed.rs"]
mod feed;

enum Source {
    Download(od_core::download_proxy::ManagedDownloadRequest),
    Feed(od_core::runtime_feed::ManagedFeedRequest),
}

pub fn require_switchable(snapshot: Payload) -> Result<(), ApiError> {
    require_idle(snapshot.clone())?;
    if let Payload::Snapshot(snapshot) = snapshot
        && snapshot.record_error.is_some()
    {
        return Err(ApiError::new(
            "record_unsaved",
            "运行结果尚未保存，请先重试保存",
        ));
    }
    Ok(())
}

pub fn require_models_idle(models: Payload) -> Result<(), ApiError> {
    match models {
        Payload::ModelResources(resources)
            if resources
                .job
                .as_ref()
                .is_none_or(|job| job.state.is_terminal()) =>
        {
            Ok(())
        }
        Payload::ModelResources(_) => {
            Err(ApiError::new("resource_busy", "请先等待或取消模型资源任务"))
        }
        _ => Err(offline("无法确认模型资源任务状态")),
    }
}

pub type SharedSession = Arc<AsyncMutex<Option<EngineSession>>>;

struct State {
    snapshot: DesktopRuntimeResources,
    closed: bool,
    maintenance: bool,
    cancel: Arc<AtomicBool>,
    feed_sources: Vec<od_core::runtime_feed::RuntimeReleaseSource>,
}

#[derive(Clone)]
pub struct ResourceManager {
    state: Arc<Mutex<State>>,
    done: watch::Sender<bool>,
    session: SharedSession,
    runtime: tokio::runtime::Handle,
}

impl ResourceManager {
    pub fn new(session: SharedSession, runtime: tokio::runtime::Handle) -> Self {
        let (done, _) = watch::channel(true);
        Self {
            state: Arc::new(Mutex::new(State {
                snapshot: DesktopRuntimeResources::default(),
                closed: false,
                maintenance: false,
                cancel: Arc::new(AtomicBool::new(false)),
                feed_sources: Vec::new(),
            })),
            done,
            session,
            runtime,
        }
    }

    pub fn snapshot(&self) -> DesktopRuntimeResources {
        self.state
            .lock()
            .expect("资源管理状态锁失效")
            .snapshot
            .clone()
    }

    pub fn switching(&self) -> bool {
        let state = self.state.lock().expect("资源管理状态锁失效");
        state.maintenance
            || state.snapshot.job.as_ref().is_some_and(|job| {
                job.state == RuntimeResourceJobState::Running && job.request.requires_engine_exit()
            })
    }

    pub fn require_available(&self) -> Result<(), ApiError> {
        let state = self.state.lock().expect("资源管理状态锁失效");
        if state.closed
            || state.maintenance
            || state.snapshot.job.as_ref().is_some_and(|job| {
                job.state == RuntimeResourceJobState::Running && job.request.requires_engine_exit()
            })
        {
            return Err(ApiError::new(
                "resource_busy",
                "配置正在离线恢复、运行资源正在切换或桌面正在退出，请等待收尾",
            ));
        }
        Ok(())
    }

    pub fn start(
        &self,
        layout: DesktopLayout,
        request: RuntimeResourceRequest,
    ) -> Result<DesktopRuntimeResources, ApiError> {
        if matches!(
            request,
            RuntimeResourceRequest::Download { .. } | RuntimeResourceRequest::Discover { .. }
        ) {
            return Err(ApiError::new(
                "invalid_resources",
                "请使用下载入口提交完整资源包信息",
            ));
        }
        self.begin(layout, request, None)
    }

    pub fn start_download_with_proxy(
        &self,
        layout: DesktopLayout,
        source: RuntimeDownloadSource,
        proxy: od_core::download_proxy::DownloadProxy,
    ) -> Result<DesktopRuntimeResources, ApiError> {
        proxy
            .validate()
            .map_err(|message| ApiError::new("invalid_proxy", message))?;
        od_platform::download::validate_https_url(&source.url)
            .map_err(|error| ApiError::new("invalid_resources", error.to_string()))?;
        self.begin(
            layout,
            RuntimeResourceRequest::Download {
                bytes: source.bytes,
                sha256: source.sha256.clone(),
            },
            Some(Source::Download(
                od_core::download_proxy::ManagedDownloadRequest {
                    source,
                    proxy,
                    expected_release: None,
                },
            )),
        )
    }

    pub fn discover_with_proxy(
        &self,
        layout: DesktopLayout,
        source: od_core::runtime_feed::RuntimeFeedSource,
        proxy: od_core::download_proxy::DownloadProxy,
    ) -> Result<DesktopRuntimeResources, ApiError> {
        od_platform::download::validate_https_url(&source.url)
            .map_err(|e| ApiError::new("invalid_resources", e.to_string()))?;
        proxy
            .validate()
            .map_err(|e| ApiError::new("invalid_proxy", e))?;
        self.begin(
            layout,
            RuntimeResourceRequest::Discover {
                sha256: source.sha256.clone(),
            },
            Some(Source::Feed(od_core::runtime_feed::ManagedFeedRequest {
                source,
                proxy,
            })),
        )
    }

    pub fn download_release(
        &self,
        layout: DesktopLayout,
        catalog_id: &str,
        sha256: &str,
        proxy: od_core::download_proxy::DownloadProxy,
    ) -> Result<DesktopRuntimeResources, ApiError> {
        proxy
            .validate()
            .map_err(|e| ApiError::new("invalid_proxy", e))?;
        let source = {
            let state = self.state.lock().expect("资源管理状态锁失效");
            let catalog = state
                .snapshot
                .feed
                .as_ref()
                .filter(|c| c.id == catalog_id)
                .ok_or_else(|| {
                    ApiError::new(
                        "resource_feed_changed",
                        "发布清单已变化，请重新读取并选择版本",
                    )
                })?;
            let option = catalog
                .releases
                .iter()
                .find(|o| o.release.sha256 == sha256 && o.compatible)
                .ok_or_else(|| {
                    ApiError::new("invalid_resources", "所选发布版本不存在或与当前引擎不兼容")
                })?;
            state
                .feed_sources
                .iter()
                .find(|s| s.release == option.release)
                .cloned()
                .ok_or_else(|| {
                    ApiError::new("resource_feed_changed", "发布来源已丢失，请重新读取清单")
                })?
        };
        let release = source.release;
        let archive = RuntimeDownloadSource {
            url: source.url,
            bytes: release.bytes,
            sha256: release.sha256.clone(),
        };
        self.begin(
            layout,
            RuntimeResourceRequest::Download {
                bytes: release.bytes,
                sha256: release.sha256.clone(),
            },
            Some(Source::Download(
                od_core::download_proxy::ManagedDownloadRequest {
                    source: archive,
                    proxy,
                    expected_release: Some(release),
                },
            )),
        )
    }

    fn begin(
        &self,
        layout: DesktopLayout,
        request: RuntimeResourceRequest,
        source: Option<Source>,
    ) -> Result<DesktopRuntimeResources, ApiError> {
        request
            .validate()
            .map_err(|error| ApiError::new("invalid_resources", error))?;
        let mut state = self.state.lock().expect("资源管理状态锁失效");
        if state.closed
            || state.maintenance
            || state
                .snapshot
                .job
                .as_ref()
                .is_some_and(|job| !job.state.is_terminal())
        {
            return Err(ApiError::new(
                "resource_busy",
                "已有运行资源任务或桌面正在退出",
            ));
        }
        if matches!(request, RuntimeResourceRequest::Discover { .. }) {
            state.snapshot.feed = None;
            state.feed_sources.clear();
        }
        state.snapshot.job = Some(RuntimeResourceJob {
            id: Uuid::new_v4(),
            request: request.clone(),
            state: RuntimeResourceJobState::Running,
            phase: "checking".into(),
            reconnected: false,
            error: None,
            error_code: None,
            progress: None,
            downloaded: None,
        });
        state.cancel = Arc::new(AtomicBool::new(false));
        let cancel = state.cancel.clone();
        let initial = state.snapshot.clone();
        self.done.send_replace(false);
        let manager = self.clone();
        // 同步 Tauri 命令可以来自没有 Tokio 上下文的 UI 线程。
        self.runtime.spawn(async move {
            if let Some(Source::Feed(request)) = &source {
                let request = request.clone();
                let result =
                    tokio::task::spawn_blocking(move || feed::run(&layout, request, &cancel))
                        .await
                        .map_err(offline)
                        .and_then(|r| r);
                let mut state = manager.state.lock().expect("资源管理状态锁失效");
                let (job_state, error) = match result {
                    Ok(Some((catalog, sources))) => {
                        state.snapshot.feed = Some(catalog);
                        state.feed_sources = sources;
                        (RuntimeResourceJobState::Succeeded, None)
                    }
                    Ok(None) => (RuntimeResourceJobState::Cancelled, None),
                    Err(error) => (RuntimeResourceJobState::Failed, Some(error)),
                };
                if let Some(job) = &mut state.snapshot.job {
                    job.phase = "finished".into();
                    job.state = job_state;
                    if let Some(error) = error {
                        job.error = Some(error.message);
                        job.error_code = Some(error.code);
                    }
                }
                manager.done.send_replace(true);
                return;
            }
            if let Some(Source::Download(source)) = source {
                let worker = manager.clone();
                let result = tokio::task::spawn_blocking(move || {
                    download::run_configured(
                        &layout,
                        &source.source,
                        source.proxy,
                        source.expected_release,
                        &cancel,
                        |progress| {
                            let mut state = worker.state.lock().expect("资源管理状态锁失效");
                            if let Some(job) = &mut state.snapshot.job {
                                job.phase = progress.phase.clone();
                                job.progress = Some(progress);
                            }
                        },
                    )
                })
                .await
                .map_err(offline)
                .and_then(|result| result);
                let mut state = manager.state.lock().expect("资源管理状态锁失效");
                if let Some(job) = &mut state.snapshot.job {
                    job.phase = "finished".into();
                    match result {
                        Ok(Some(archive)) => {
                            job.downloaded = Some(archive);
                            job.state = RuntimeResourceJobState::Succeeded;
                        }
                        Ok(None) => job.state = RuntimeResourceJobState::Cancelled,
                        Err(error) => {
                            job.state = RuntimeResourceJobState::Failed;
                            job.error = Some(error.message);
                            job.error_code = Some(error.code);
                        }
                    }
                }
                manager.done.send_replace(true);
                return;
            }
            let result = if request.requires_engine_exit() {
                manager.change(&layout, &request).await
            } else {
                Ok(())
            };
            let catalog = helper(&layout, "catalog", None).await.and_then(|value| {
                serde_json::from_value::<RuntimeResourceCatalog>(value).map_err(offline)
            });
            let mut state = manager.state.lock().expect("资源管理状态锁失效");
            let result = match catalog {
                Ok(catalog) => {
                    state.snapshot.catalog = Some(catalog);
                    result
                }
                Err(error) => {
                    state.snapshot.catalog = None;
                    result.and(Err(error))
                }
            };
            if let Some(job) = &mut state.snapshot.job {
                job.state = if result.is_ok() {
                    RuntimeResourceJobState::Succeeded
                } else {
                    RuntimeResourceJobState::Failed
                };
                job.phase = "finished".into();
                if let Err(error) = result {
                    job.error = Some(error.message);
                    job.error_code = Some(error.code);
                }
            }
            manager.done.send_replace(true);
        });
        Ok(initial)
    }

    pub fn cancel_download(&self, id: Uuid) -> Result<DesktopRuntimeResources, ApiError> {
        let mut state = self.state.lock().expect("资源管理状态锁失效");
        let job = state
            .snapshot
            .job
            .as_mut()
            .filter(|job| {
                job.id == id
                    && matches!(
                        job.request,
                        RuntimeResourceRequest::Download { .. }
                            | RuntimeResourceRequest::Discover { .. }
                    )
            })
            .ok_or_else(|| ApiError::new("resource_job_changed", "下载任务已变化，请刷新后重试"))?;
        if !job.state.is_terminal() {
            job.state = RuntimeResourceJobState::Cancelling;
            state.cancel.store(true, Ordering::Release);
        }
        Ok(state.snapshot.clone())
    }

    fn phase(&self, phase: &str) {
        if let Some(job) = &mut self.state.lock().expect("资源管理状态锁失效").snapshot.job
        {
            job.phase = phase.into();
        }
    }

    async fn change(
        &self,
        layout: &DesktopLayout,
        request: &RuntimeResourceRequest,
    ) -> Result<(), ApiError> {
        let mut session = self.session.lock().await;
        if let Some(current) = session.as_mut() {
            if !current.exited()? {
                if !current.connected() {
                    return Err(offline("旧引擎仍在收尾，请等待退出后重试"));
                }
                let snapshot = current.request(Command::Snapshot).await?;
                require_switchable(snapshot)?;
                let models = current.request(Command::ModelResources).await?;
                require_models_idle(models)?;
            }
            current.reload_recovery(layout)?;
        }
        RecoveryJournal::open(&layout.data)?.require_clean()?;
        let installing = matches!(request, RuntimeResourceRequest::Install { .. });
        let verification = helper(
            layout,
            if installing {
                "check"
            } else {
                "preview-change"
            },
            Some(request),
        );
        tokio::pin!(verification);
        loop {
            tokio::select! {
                result = &mut verification => { result?; break; }
                _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
                    if let Some(current) = session.as_mut() && !current.exited()? {
                        require_switchable(current.request(Command::Snapshot).await?)?;
                    }
                }
            }
        }
        if self.state.lock().expect("资源管理状态锁失效").closed {
            return Err(ApiError::new(
                "resource_closed",
                "桌面正在退出，未安装或切换资源",
            ));
        }
        self.phase("stopping_engine");
        if let Some(current) = session.as_mut() {
            current.shutdown().await?;
        }
        session.take();
        self.phase(if installing { "installing" } else { "applying" });
        let changed = helper(
            layout,
            if installing {
                "install"
            } else {
                "apply-change"
            },
            Some(request),
        )
        .await
        .map(|_| ());
        let closing = self.state.lock().expect("资源管理状态锁失效").closed;
        if !closing {
            self.phase("starting_engine");
            let restarted = async {
                let mut next = EngineSession::launch(layout, false).await?;
                let initialized = next.request(Command::Snapshot).await;
                *session = Some(next);
                initialized.map(|_| ())
            }
            .await;
            if let Err(error) = restarted {
                return Err(ApiError::new(
                    "resource_restart_failed",
                    format!(
                        "{}；引擎重新连接失败：{}。请手动重连",
                        changed.as_ref().err().map_or(
                            if installing {
                                "资源包已安装，当前选择保持不变"
                            } else {
                                "版本选择已保存"
                            }
                            .to_owned(),
                            |error| error.message.clone()
                        ),
                        error.message
                    ),
                ));
            }
            if let Some(job) = &mut self.state.lock().expect("资源管理状态锁失效").snapshot.job
            {
                job.reconnected = true;
            }
        }
        changed
    }

    pub async fn close_and_wait(&self) {
        let mut done = self.done.subscribe();
        {
            let mut state = self.state.lock().expect("资源管理状态锁失效");
            state.closed = true;
            state.cancel.store(true, Ordering::Release);
        }
        while !*done.borrow_and_update() {
            if done.changed().await.is_err() {
                break;
            }
        }
    }

    pub fn reopen(&self) {
        self.state.lock().expect("资源管理状态锁失效").closed = false;
    }

    // 离线配置恢复复用资源任务的生命周期栅栏；独立测试可能只复用资源管理部分。
    #[allow(dead_code)]
    pub fn maintenance(&self) -> Result<MaintenanceGuard, ApiError> {
        let mut state = self.state.lock().expect("资源管理状态锁失效");
        if state.closed
            || state.maintenance
            || state
                .snapshot
                .job
                .as_ref()
                .is_some_and(|job| !job.state.is_terminal())
        {
            return Err(ApiError::new(
                "resource_busy",
                "已有配置恢复、运行资源任务或桌面正在退出，请等待收尾",
            ));
        }
        state.maintenance = true;
        self.done.send_replace(false);
        Ok(MaintenanceGuard(self.clone()))
    }
}

pub struct MaintenanceGuard(ResourceManager);

impl MaintenanceGuard {
    #[allow(dead_code)]
    pub fn require_open(&self) -> Result<(), ApiError> {
        if self.0.state.lock().expect("资源管理状态锁失效").closed {
            Err(ApiError::new(
                "config_recovery_closed",
                "桌面正在退出，未开始新的恢复步骤",
            ))
        } else {
            Ok(())
        }
    }
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().expect("资源管理状态锁失效");
        state.maintenance = false;
        self.0.done.send_replace(true);
    }
}

async fn helper(
    layout: &DesktopLayout,
    operation: &'static str,
    request: Option<&RuntimeResourceRequest>,
) -> Result<serde_json::Value, ApiError> {
    let layout = layout.clone();
    let request = request.cloned();
    tokio::task::spawn_blocking(move || {
        let mut command = ProcessCommand::new(&layout.engine);
        command
            .creation_flags(0x08000000)
            .arg("--root")
            .arg(&layout.root)
            .arg("--data")
            .arg(&layout.data)
            .args(["runtime-resources", operation]);
        match request {
            Some(RuntimeResourceRequest::Install { archive, sha256 }) => {
                command
                    .arg("--archive")
                    .arg(archive)
                    .arg("--sha256")
                    .arg(sha256);
            }
            Some(RuntimeResourceRequest::Activate { id, revision }) => {
                command.args(["--id", &id, "--revision", &revision]);
            }
            Some(RuntimeResourceRequest::Rollback { revision }) => {
                command.args(["--rollback", "--revision", &revision]);
            }
            Some(
                RuntimeResourceRequest::Download { .. } | RuntimeResourceRequest::Discover { .. },
            ) => return Err(offline("下载入口无效")),
            Some(RuntimeResourceRequest::Refresh) | None => {}
        }
        let output = command.output().map_err(offline)?;
        if !output.status.success() {
            return Err(ApiError::new(
                "resource_error",
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(4096)
                    .collect::<String>(),
            ));
        }
        if output.stdout.len() > 256 * 1024 {
            return Err(offline("运行资源应答过大"));
        }
        serde_json::from_slice(&output.stdout).map_err(offline)
    })
    .await
    .map_err(offline)?
}
