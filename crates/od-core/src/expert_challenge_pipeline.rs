//! 专业挑战室使用独立电量区域，共用动作只在 Rust 复核之后投递。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = json!({});
    for (name, roi) in [
        ("Modal", [570, 290, 740, 520]),
        ("Level", [1454, 26, 132, 54]),
        ("Next", [1590, 1000, 260, 55]),
        ("Current", [1475, 1000, 53, 55]),
        ("Required", [1535, 1000, 45, 55]),
        ("Mode", [1658, 672, 96, 50]),
        ("ModeConfirm", [1030, 602, 196, 50]),
        ("Deploy", [1634, 1000, 198, 54]),
        ("Team", [368, 1002, 195, 55]),
        ("Finish", [1566, 1004, 266, 52]),
        ("Retry", [1232, 1006, 276, 46]),
        ("Retreat", [526, 848, 220, 64]),
        ("Rewind", [874, 850, 214, 64]),
        ("Exit", [1566, 1004, 266, 52]),
    ] {
        p[format!("Expert.{name}")] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":roi,"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    }
    p["Expert.ModeOn"] = p["Expert.Mode"].clone();
    p["Expert.ModeOn"]["color_filter"] = json!("Expert.ModeColor");
    p["Expert.ModeColor"] = json!({"recognition":"ColorMatch","enabled":false,"action":"DoNothing","next":[],"on_error":[],
        "roi":[1658,672,96,50],"method":4,"lower":[70,80,0],"upper":[255,255,255]});
    p
}
