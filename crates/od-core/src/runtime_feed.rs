use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFeedSource {
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRelease {
    pub version: String,
    pub engine_version: String,
    pub sdk_version: String,
    pub binding_version: String,
    pub protocol_version: u32,
    pub resource_schema: u32,
    #[ts(type = "number")]
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RuntimeReleaseOption {
    pub release: RuntimeRelease,
    pub compatible: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RuntimeFeedCatalog {
    pub id: String,
    pub releases: Vec<RuntimeReleaseOption>,
}

/// 仅用于原生子进程私有通信，URL 不进入桌面任务快照。
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeReleaseSource {
    pub release: RuntimeRelease,
    pub url: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedFeedRequest {
    pub source: RuntimeFeedSource,
    pub proxy: crate::download_proxy::DownloadProxy,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum RuntimeFeedEvent {
    Completed {
        catalog: RuntimeFeedCatalog,
        sources: Vec<RuntimeReleaseSource>,
    },
    Failed(String),
    Cancelled,
}
