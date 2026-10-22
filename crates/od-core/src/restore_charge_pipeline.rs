//! 独立恢复资源 schema 1；只读节点，输入由核对检查点的原生适配器发送。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = json!({});
    for (name, roi) in [
        ("Titles", [570, 290, 530, 175]),
        ("Sources", [775, 575, 370, 65]),
        ("QuickSource", [780, 425, 420, 55]),
        ("Quantity", [968, 602, 65, 45]),
        ("Reward", [890, 575, 140, 75]),
        ("Buttons", [715, 700, 485, 105]),
    ] {
        p[format!("Restore.Area.{name}")] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,
            "roi":roi,"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    }
    for (name, roi) in [
        ("BackupSelected", [756, 450, 10, 120]),
        ("EtherSelected", [900, 450, 10, 120]),
    ] {
        p[format!("Restore.Area.{name}")] = json!({"recognition":"ColorMatch","method":4,"roi":roi,
            "lower":[200,180,0],"upper":[255,255,100],"count":50,"connected":false,
            "action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    }
    p
}
