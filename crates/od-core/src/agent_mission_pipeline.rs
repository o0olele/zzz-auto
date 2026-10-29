//! 培养详情选关复用原模拟室锚点，新增节点使用独立名称。
use serde_json::{Value, json};
pub fn templates() -> Vec<(&'static str, &'static [u8])> {
    crate::simulation_mission_pipeline::templates()
}
pub fn pipeline() -> Value {
    let mut p = crate::simulation_pipeline::pipeline();
    p["AgentMission.Names"] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[128,724,1668,158],"action":"DoNothing","next":[],"on_error":[]});
    p["AgentMission.Back"] = json!({"recognition":"TemplateMatch","template":["agent-mission/back.png"],"roi":[82,13,68,77],"threshold":0.7,"method":5,"green_mask":true,"action":"DoNothing","next":[],"on_error":[]});
    p
}
