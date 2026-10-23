//! 固定实战模拟室识别区域；独立资源 schema 1 不投递 Pipeline 输入。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = json!({});
    for (name, roi) in [
        ("Modal", [570, 290, 740, 520]),
        ("Level", [1454, 26, 132, 54]),
        ("Next", [1590, 1000, 260, 55]),
        ("Current", [1440, 1000, 56, 55]),
        ("Required", [1508, 1000, 44, 55]),
        ("Save", [1600, 1008, 234, 40]),
        ("Types", [406, 592, 1122, 236]),
        ("Deploy", [1634, 1000, 198, 54]),
        ("Team", [368, 1002, 195, 55]),
        ("Finish", [1566, 1004, 266, 52]),
        ("Retry", [1232, 1006, 276, 46]),
    ] {
        p[format!("Simulation.Area.{name}")] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,
            "roi":roi,"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    }
    p
}
