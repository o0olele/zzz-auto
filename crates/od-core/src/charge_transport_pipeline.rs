//! 名称、选中分类、前往和入口分别读取；所有节点均只读。
use serde_json::{Value, json};
pub fn templates() -> Vec<(&'static str, &'static [u8])> {
    crate::compendium_pipeline::templates()
}
pub fn pipeline() -> Value {
    let base = crate::compendium_pipeline::pipeline();
    let mut p = serde_json::Map::new();
    for (name, value) in base.as_object().expect("固定手册节点") {
        if name.starts_with("Compendium.") {
            p.insert(
                name.replace("Compendium.", "TrainingTransport."),
                serde_json::from_str::<Value>(
                    &serde_json::to_string(value)
                        .expect("固定节点编码")
                        .replace("Compendium.", "TrainingTransport.")
                        .replace("compendium/", "charge-transport/"),
                )
                .expect("固定节点重命名"),
            );
        }
    }
    let mut p = Value::Object(p);
    p["TrainingTransport.CategoryColor"] = json!({"recognition":"ColorMatch","enabled":false,"action":"DoNothing","next":[],"on_error":[],"roi":[164,287,265,583],"method":4,"lower":[0,70,180],"upper":[100,220,255]});
    for (name, roi) in [
        ("DetailLevel", [1454, 26, 132, 54]),
        ("DetailNext", [1590, 1000, 260, 55]),
        ("PatrolCharge", [1525, 1000, 125, 55]),
        ("ExpertCurrent", [1475, 1000, 53, 55]),
        ("ExpertRequired", [1535, 1000, 45, 55]),
        ("HuntRewards", [192, 992, 176, 33]),
        ("HuntNames", [240, 216, 1512, 250]),
        ("HuntStreet", [238, 24, 150, 58]),
        ("HuntTitle", [750, 168, 726, 97]),
    ] {
        p[format!("TrainingTransport.{name}")] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":roi,"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    }
    p
}
