//! 固定结算和奖励区域；复用原卡片包的只读节点，原包定义保持冻结。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = crate::simulation_pipeline::pipeline();
    for (name, roi) in [
        ("Retreat", [526, 848, 220, 64]),
        ("Rewind", [874, 850, 214, 64]),
        ("Exit", [1566, 1004, 266, 52]),
    ] {
        p[format!("Settlement.{name}")] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,
            "roi":roi,"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    }
    p
}
