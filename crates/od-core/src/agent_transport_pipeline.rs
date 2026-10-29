//! 代理人手册导航独立命名空间，普通传送包定义保持不变。
use serde_json::Value;
pub fn templates() -> Vec<(&'static str, &'static [u8])> {
    crate::charge_transport_pipeline::templates()
}
pub fn pipeline() -> Value {
    let source = crate::charge_transport_pipeline::pipeline();
    let renamed = serde_json::to_string(&source)
        .expect("固定代理人节点")
        .replace("TrainingTransport.", "AgentTransport.")
        .replace("charge-transport/", "agent-transport/");
    let mut p: Value = serde_json::from_str(&renamed).expect("代理人节点重命名");
    for (name, node) in crate::simulation_pipeline::pipeline()
        .as_object()
        .expect("固定模拟室节点")
    {
        p[name] = node.clone();
    }
    p
}
