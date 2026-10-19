use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(deny_unknown_fields)]
pub struct ResourceSelection {
    pub schema_version: u32,
    pub current: Option<String>,
    pub previous: Option<String>,
    pub can_rollback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct InstalledResources {
    pub id: String,
    pub version: Option<String>,
    pub compatible: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RuntimeResourceCatalog {
    pub revision: String,
    pub selection: ResourceSelection,
    pub directory: String,
    pub versions: Vec<InstalledResources>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeResourceRequest {
    Discover {
        sha256: String,
    },
    Refresh,
    Download {
        #[ts(type = "number")]
        bytes: u64,
        sha256: String,
    },
    Install {
        archive: String,
        sha256: String,
    },
    Activate {
        id: String,
        revision: String,
    },
    Rollback {
        revision: String,
    },
}

impl RuntimeResourceRequest {
    pub fn changes_selection(&self) -> bool {
        matches!(self, Self::Activate { .. } | Self::Rollback { .. })
    }

    pub fn requires_engine_exit(&self) -> bool {
        matches!(
            self,
            Self::Install { .. } | Self::Activate { .. } | Self::Rollback { .. }
        )
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let hash = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        };
        let valid = match self {
            Self::Discover { sha256 } => hash(sha256),
            Self::Refresh => true,
            Self::Download { bytes, sha256 } => (1..=536870912).contains(bytes) && hash(sha256),
            Self::Install { archive, sha256 } => {
                let path = std::path::Path::new(archive);
                archive.len() <= 32768
                    && !archive.contains('\0')
                    && path.is_absolute()
                    && matches!(path.components().next(), Some(std::path::Component::Prefix(prefix))
                        if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
                    && path.components().skip(1).all(|component| match component {
                        std::path::Component::Normal(name) => !name.to_string_lossy().contains(':'),
                        std::path::Component::ParentDir => false,
                        _ => true,
                    })
                    && path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
                    && hash(sha256)
            }
            Self::Activate { id, revision } => hash(id) && hash(revision),
            Self::Rollback { revision } => hash(revision),
        };
        if valid {
            Ok(())
        } else {
            Err("资源参数无效：请核对 ZIP 大小、64 位小写 SHA-256、本地路径或版本修订")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeResourceJobState {
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

impl RuntimeResourceJobState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RuntimeResourceJob {
    pub id: Uuid,
    pub request: RuntimeResourceRequest,
    pub state: RuntimeResourceJobState,
    pub phase: String,
    pub reconnected: bool,
    pub error: Option<String>,
    pub error_code: Option<String>,
    pub progress: Option<RuntimeDownloadProgress>,
    pub downloaded: Option<RuntimeDownloadedArchive>,
}

/// 地址仅用于本次提交与子进程标准输入，不进入可查询任务快照。
#[derive(Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RuntimeDownloadSource {
    pub url: String,
    #[ts(type = "number")]
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RuntimeDownloadProgress {
    pub phase: String,
    #[ts(type = "number")]
    pub completed_bytes: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RuntimeDownloadedArchive {
    pub archive: String,
    pub sha256: String,
    #[ts(type = "number")]
    pub bytes: u64,
    pub reused: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum RuntimeDownloadEvent {
    Progress(RuntimeDownloadProgress),
    Completed(RuntimeDownloadedArchive),
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct DesktopRuntimeResources {
    pub catalog: Option<RuntimeResourceCatalog>,
    pub job: Option<RuntimeResourceJob>,
    #[serde(default)]
    #[ts(optional)]
    pub feed: Option<crate::runtime_feed::RuntimeFeedCatalog>,
}
