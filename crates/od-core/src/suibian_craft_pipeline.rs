//! schema 3 在冻结的托管、游历定义上增加制造只读区域。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = crate::suibian_adventure_pipeline::pipeline();
    p["Craft.Area.Text"] =
        json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[0,0,1920,1080]});
    p["Craft.Area.WhitePixels"] = json!({"recognition":"ColorMatch","enabled":false,"method":4,"lower":[230,230,230],"upper":[255,255,255]});
    p["Craft.Area.White"] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[306,326,830,604],"color_filter":"Craft.Area.WhitePixels"});
    p["Craft.Area.Selected"] = json!({"recognition":"ColorMatch","method":4,"roi":[0,0,1,1],"lower":[150,170,0],"upper":[255,255,100],"count":1800,"connected":false});
    for (name, node) in p.as_object_mut().unwrap() {
        if name.starts_with("Craft.") {
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
