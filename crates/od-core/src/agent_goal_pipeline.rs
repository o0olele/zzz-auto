//! 达成文字及结算上下文独立只读包；不修改原挑战资源定义。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = json!({});
    for (name, roi) in [
        ("Modal", [570, 290, 740, 520]),
        ("Finish", [1566, 1004, 266, 52]),
        ("Goal", [610, 1000, 80, 55]),
    ] {
        p[format!("AgentGoal.{name}")] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":roi,"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    }
    p["AgentGoal.Goal"]["only_rec"] = json!(true);
    p
}
