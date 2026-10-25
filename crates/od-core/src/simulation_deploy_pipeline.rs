//! 固定出战区域，原生输入由有检查点的 Rust 适配器执行。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = json!({});
    for (name, roi) in [
        ("Modal", [570, 290, 740, 520]),
        ("Capacity", [720, 500, 490, 55]),
        ("Confirm", [1030, 602, 192, 54]),
        ("Deploy", [1570, 982, 250, 62]),
        ("Team", [368, 1002, 195, 55]),
    ] {
        p[format!("Deploy.Area.{name}")] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":roi,"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    }
    p
}
