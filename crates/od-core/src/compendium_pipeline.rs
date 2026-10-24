//! 固定快捷手册区域，沿用已有黄色 TAB 过滤和大世界模板。
use serde_json::{Value, json};
pub fn templates() -> Vec<(&'static str, &'static [u8])> {
    crate::basic_daily::templates()
        .into_iter()
        .filter(|(n, _)| ["world_function", "world_preset", "compendium"].contains(n))
        .collect()
}
pub fn pipeline() -> Value {
    let mut p = crate::simulation_pipeline::pipeline();
    for (name, roi) in [
        ("Tabs", [708, 140, 928, 80]),
        ("Categories", [164, 287, 265, 583]),
        ("Types", [498, 308, 350, 576]),
        ("Go", [1312, 334, 334, 556]),
        ("Modal", [570, 290, 740, 520]),
        ("Confirm", [1030, 602, 200, 48]),
    ] {
        p[format!("Compendium.Area.{name}")] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":roi,
            "action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    }
    p["Compendium.Selected"] = p["Compendium.Area.Tabs"].clone();
    p["Compendium.Selected"]["color_filter"] = json!("Compendium.TabColor");
    p["Compendium.TabColor"] = json!({"recognition":"ColorMatch","enabled":false,"action":"DoNothing","next":[],"on_error":[],
        "roi":[708,140,928,80],"method":4,"lower":[200,160,0],"upper":[255,255,60]});
    for (name, roi, template, mask) in [
        ("WorldFunction", [1776, 28, 93, 89], "world_function", true),
        ("WorldPreset", [1225, 44, 280, 58], "world_preset", true),
        ("Icon", [1440, 28, 249, 90], "compendium", false),
    ] {
        p[format!("Compendium.{name}")] = json!({"recognition":"TemplateMatch","roi":roi,"template":[format!("compendium/{template}.png")],
            "threshold":0.7,"method":5,"green_mask":mask,"action":"DoNothing","next":[],"on_error":[]});
    }
    p
}
