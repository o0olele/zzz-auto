//! 所有节点只读；合成动作由 Rust 在同帧复核后投递。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut nodes = serde_json::Map::new();
    for (name, roi) in [
        ("Title", [178, 18, 152, 68]),
        ("Item", [1400, 196, 320, 58]),
        ("Quantity", [1400, 518, 320, 40]),
        ("Charge", [1550, 812, 150, 40]),
        ("Shortage", [1455, 890, 180, 60]),
        ("Synthesize", [1540, 1000, 300, 62]),
        ("ConfirmTitle", [670, 380, 610, 74]),
        ("Costs", [750, 580, 430, 65]),
        ("Cancel", [674, 700, 286, 90]),
        ("Confirm", [980, 700, 356, 90]),
        ("RewardTitle", [860, 386, 200, 74]),
        ("RewardItem", [860, 613, 200, 40]),
        ("RewardCount", [900, 578, 120, 35]),
        ("Collect", [820, 700, 340, 90]),
        ("Modal", [650, 370, 700, 430]),
    ] {
        nodes.insert(
            format!("Synthesis.{name}"),
            json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":roi,
            "action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0}),
        );
    }
    Value::Object(nodes)
}
