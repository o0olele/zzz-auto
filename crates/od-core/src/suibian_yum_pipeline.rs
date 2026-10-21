//! schema 4 添加饮茶仙只读区域；历史子流程定义保持原样。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = crate::suibian_craft_pipeline::pipeline();
    p["Yum.Area.Text"] =
        json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[0,0,1920,1080]});
    p["Yum.Area.Red"] = json!({"recognition":"ColorMatch","method":4,"roi":[0,0,1,1],"lower":[220,70,30],"upper":[230,140,110],"count":8,"connected":false});
    p["Yum.Area.Regular"] = json!({"recognition":"ColorMatch","method":4,"roi":[1560,110,270,70],"lower":[210,70,0],"upper":[255,170,90],"count":5000,"connected":false});
    for (name, node) in p.as_object_mut().unwrap() {
        if name.starts_with("Yum.") {
            for (k, v) in [
                ("action", json!("DoNothing")),
                ("next", json!([])),
                ("on_error", json!([])),
                ("pre_delay", json!(0)),
                ("post_delay", json!(0)),
            ] {
                node.as_object_mut().unwrap().entry(k).or_insert(v);
            }
        }
    }
    p
}
