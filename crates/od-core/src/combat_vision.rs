//! 参考角色数据转换后的显式识别定义；无 Python 动态加载。
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStateDef {
    pub state_name: String,
    pub check_way: String,
    pub template_id: String,
    pub lower_color: Option<serde_json::Value>,
    pub upper_color: Option<serde_json::Value>,
    pub hsv_color: Option<[f64; 3]>,
    pub hsv_color_diff: Option<[f64; 3]>,
    pub connect_cnt: Option<u32>,
    pub split_color_range: Option<[f64; 2]>,
    #[serde(default = "hundred")]
    pub max_length: u32,
    #[serde(default)]
    pub min_value_trigger_state: i64,
    pub template_threshold: Option<f64>,
    #[serde(default)]
    pub clear_on_zero: bool,
}
fn hundred() -> u32 { 100 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub templates: Vec<String>,
    pub states: Vec<AgentStateDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub image: String,
    pub mask: Option<String>,
    pub rect: Option<[i32; 4]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisionSpec {
    pub schema: u32,
    pub agents: Vec<Agent>,
    pub common: BTreeMap<String, AgentStateDef>,
    pub areas: BTreeMap<String, [i32; 4]>,
    pub templates: BTreeMap<String, Template>,
}
