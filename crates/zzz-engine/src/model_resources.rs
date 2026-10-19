use od_core::model_resources::{
    CacheState, ModelResources, ResourceJob, ResourceJobKind, ResourceJobState, ResourceModel,
};
use od_platform::model_cache::{self, HttpsModelDownloader, ModelDownloader};
use od_protocol::ApiError;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::watch;
use uuid::Uuid;

struct State {
    snapshot: ModelResources,
    cancel: Option<Arc<AtomicBool>>,
    closed: bool,
    worker: Option<std::thread::JoinHandle<()>>,
}

pub struct ModelResourceService {
    cache: PathBuf,
    state: Arc<Mutex<State>>,
    done: watch::Sender<bool>,
}

impl ModelResourceService {
    pub fn new(cache: PathBuf) -> anyhow::Result<Self> {
        let models = model_cache::locked_models()?
            .models
            .into_iter()
            .map(|model| ResourceModel {
                name: match model.role.as_str() {
                    "ocr" => "文字识别".into(),
                    "classify" => "闪光分类".into(),
                    _ if model.id.contains("lost-void") => "迷失之地检测".into(),
                    _ => "空洞事件检测".into(),
                },
                id: model.id,
                archive_bytes: model.bytes,
                archive: None,
                files: None,
            })
            .collect();
        let (done, _) = watch::channel(true);
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                snapshot: ModelResources {
                    cache_directory: cache.display().to_string(),
                    models,
                    job: None,
                },
                cancel: None,
                closed: false,
                worker: None,
            })),
            cache,
            done,
        })
    }

    pub fn snapshot(&self) -> ModelResources {
        self.state.lock().expect("资源状态锁失效").snapshot.clone()
    }

    pub fn start(
        &self,
        kind: ResourceJobKind,
        models: Vec<String>,
        offline: bool,
    ) -> Result<ModelResources, ApiError> {
        self.start_with(kind, models, offline, HttpsModelDownloader)
    }

    pub fn start_with_proxy(
        &self,
        kind: ResourceJobKind,
        models: Vec<String>,
        offline: bool,
        proxy: od_core::download_proxy::DownloadProxy,
    ) -> Result<ModelResources, ApiError> {
        let downloader = od_platform::download::ConfiguredHttpsDownloader::new(proxy)
            .map_err(|error| ApiError::new("invalid_proxy", error.to_string()))?;
        self.start_with(kind, models, offline, downloader)
    }

    /// 测试可替换传输；IPC 不暴露来源、路径或下载器。
    pub fn start_with(
        &self,
        kind: ResourceJobKind,
        models: Vec<String>,
        offline: bool,
        downloader: impl ModelDownloader + Send + 'static,
    ) -> Result<ModelResources, ApiError> {
        let mut state = self.state.lock().expect("资源状态锁失效");
        if state.closed {
            return Err(ApiError::new(
                "resource_closed",
                "引擎正在退出，不能开始资源任务",
            ));
        }
        if state
            .snapshot
            .job
            .as_ref()
            .is_some_and(|job| !job.state.is_terminal())
        {
            return Err(ApiError::new(
                "resource_busy",
                "已有模型资源任务运行或正在取消",
            ));
        }
        let selected: BTreeSet<_> = models.iter().collect();
        if selected.len() != models.len()
            || models
                .iter()
                .any(|id| !state.snapshot.models.iter().any(|model| &model.id == id))
            || kind == ResourceJobKind::Check && offline
        {
            return Err(ApiError::new(
                "invalid_resources",
                "模型编号重复、未知或检查参数无效",
            ));
        }
        let models: Vec<_> = state
            .snapshot
            .models
            .iter()
            .filter(|model| selected.is_empty() || selected.contains(&model.id))
            .map(|model| model.id.clone())
            .collect();
        if let Some(worker) = state.worker.take() {
            let _ = worker.join();
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let job = ResourceJob {
            id: Uuid::new_v4(),
            kind,
            state: ResourceJobState::Running,
            models,
            offline,
            progress: None,
            prepared: Vec::new(),
            error: None,
        };
        let id = job.id;
        for model in &mut state.snapshot.models {
            if job.models.contains(&model.id) {
                model.archive = None;
                model.files = None;
            }
        }
        state.snapshot.job = Some(job.clone());
        state.cancel = Some(cancel.clone());
        self.done.send_replace(false);
        let shared = self.state.clone();
        let done = self.done.clone();
        let cache = self.cache.clone();
        let worker = std::thread::Builder::new()
            .name("model-resources".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let prepared = if job.kind == ResourceJobKind::Prepare {
                        model_cache::prepare_models(
                            &cache,
                            &job.models,
                            offline,
                            &downloader,
                            &cancel,
                            |progress| {
                                let mut state = shared.lock().expect("资源状态锁失效");
                                if progress.phase == "ready"
                                    && let Some(model) = state
                                        .snapshot
                                        .models
                                        .iter_mut()
                                        .find(|model| model.id == progress.id)
                                {
                                    model.files = Some(CacheState::Valid);
                                }
                                if let Some(job) = &mut state.snapshot.job {
                                    job.progress = Some(progress);
                                }
                            },
                        )?
                    } else {
                        Vec::new()
                    };
                    let status =
                        model_cache::model_cache_status_cancellable(&cache, &job.models, &cancel)?;
                    Ok::<_, anyhow::Error>((prepared, status))
                }))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("模型资源工作线程异常")));
                let mut state = shared.lock().expect("资源状态锁失效");
                let (prepared, error, final_state) = match result {
                    Ok((prepared, statuses)) => {
                        for status in statuses {
                            if let Some(model) = state
                                .snapshot
                                .models
                                .iter_mut()
                                .find(|model| model.id == status.id)
                            {
                                model.archive = Some(status.archive);
                                model.files = Some(status.files);
                            }
                        }
                        (prepared, None, ResourceJobState::Succeeded)
                    }
                    Err(error) => (
                        Vec::new(),
                        Some(format!("{error:#}")),
                        if cancel.load(Ordering::Acquire) {
                            ResourceJobState::Cancelled
                        } else {
                            ResourceJobState::Failed
                        },
                    ),
                };
                if let Some(job) = &mut state.snapshot.job {
                    job.prepared = prepared;
                    job.error = error;
                    job.state = final_state;
                }
                state.cancel = None;
                done.send_replace(true);
            });
        match worker {
            Ok(worker) => state.worker = Some(worker),
            Err(error) => {
                state.cancel = None;
                if let Some(job) = &mut state.snapshot.job {
                    job.state = ResourceJobState::Failed;
                    job.error = Some(error.to_string());
                }
                self.done.send_replace(true);
                return Err(ApiError::new("resource_worker", "无法启动模型资源工作线程"));
            }
        }
        debug_assert_eq!(state.snapshot.job.as_ref().map(|job| job.id), Some(id));
        Ok(state.snapshot.clone())
    }

    pub fn cancel(&self, id: Uuid) -> Result<ModelResources, ApiError> {
        let mut state = self.state.lock().expect("资源状态锁失效");
        let job = state
            .snapshot
            .job
            .as_mut()
            .filter(|job| job.id == id)
            .ok_or_else(|| ApiError::new("resource_job", "资源任务已变更，请重新读取状态"))?;
        if !job.state.is_terminal() {
            job.state = ResourceJobState::Cancelling;
            if let Some(cancel) = &state.cancel {
                cancel.store(true, Ordering::Release);
            }
        }
        Ok(state.snapshot.clone())
    }

    pub fn close(&self) {
        let mut state = self.state.lock().expect("资源状态锁失效");
        state.closed = true;
        if let Some(cancel) = &state.cancel {
            cancel.store(true, Ordering::Release);
        }
        if let Some(job) = &mut state.snapshot.job
            && !job.state.is_terminal()
        {
            job.state = ResourceJobState::Cancelling;
        }
    }

    pub async fn shutdown(&self) {
        let mut done = self.done.subscribe();
        self.close();
        loop {
            if *done.borrow_and_update() {
                break;
            }
            if done.changed().await.is_err() {
                break;
            }
        }
        let worker = self.state.lock().expect("资源状态锁失效").worker.take();
        if let Some(worker) = worker {
            let _ = tokio::task::spawn_blocking(move || worker.join()).await;
        }
    }
}

impl Drop for ModelResourceService {
    fn drop(&mut self) {
        self.close();
    }
}
