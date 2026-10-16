use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    Missing,
    Invalid,
    Valid,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ModelCacheStatus {
    pub id: String,
    pub archive: CacheState,
    pub files: CacheState,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PreparedModel {
    pub id: String,
    pub reused: bool,
    pub downloaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ModelProgress {
    pub id: String,
    pub phase: String,
    #[ts(type = "number")]
    pub completed_bytes: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResourceJobKind {
    Check,
    Prepare,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResourceJobState {
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

impl ResourceJobState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ResourceJob {
    pub id: Uuid,
    pub kind: ResourceJobKind,
    pub state: ResourceJobState,
    pub models: Vec<String>,
    pub offline: bool,
    pub progress: Option<ModelProgress>,
    pub prepared: Vec<PreparedModel>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ResourceModel {
    pub id: String,
    pub name: String,
    #[ts(type = "number")]
    pub archive_bytes: u64,
    pub archive: Option<CacheState>,
    pub files: Option<CacheState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ModelResources {
    pub cache_directory: String,
    pub models: Vec<ResourceModel>,
    pub job: Option<ResourceJob>,
}
