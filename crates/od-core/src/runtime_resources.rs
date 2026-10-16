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
    Refresh,
    Activate { id: String, revision: String },
    Rollback { revision: String },
}

impl RuntimeResourceRequest {
    pub fn changes_selection(&self) -> bool {
        !matches!(self, Self::Refresh)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let hash = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        };
        let valid = match self {
            Self::Refresh => true,
            Self::Activate { id, revision } => hash(id) && hash(revision),
            Self::Rollback { revision } => hash(revision),
        };
        if valid {
            Ok(())
        } else {
            Err("资源版本编号或选择修订无效，请刷新后重试")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeResourceJobState {
    Running,
    Succeeded,
    Failed,
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct DesktopRuntimeResources {
    pub catalog: Option<RuntimeResourceCatalog>,
    pub job: Option<RuntimeResourceJob>,
}
