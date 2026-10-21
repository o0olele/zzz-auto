//! schema 2 在冻结的 schema 1 托管定义上追加只读游历区域。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = crate::suibian_temple_auto::pipeline();
    p["Adventure.Area.Text"] =
        json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[0,0,1920,1080]});
    p["Adventure.Area.Count"] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[1550,27,181,52],"only_rec":true});
    p["Adventure.Area.ListOpen"] = json!({"recognition":"TemplateMatch","roi":[1723,34,32,30],"template":"suibian-temple/squads-open.png","threshold":0.85});
    p["Adventure.Area.ListBody"] = json!({"recognition":"ColorMatch","method":4,"roi":[1530,96,200,15],"lower":[30,30,30],"upper":[100,100,100],"count":1800,"connected":false});
    p["Adventure.Area.Marker"] = json!({"recognition":"ColorMatch","method":4,"roi":[0,0,1,1],"lower":[150,170,0],"upper":[255,255,90],"count":35,"connected":false});
    for (i, x) in [1380, 1484, 1584, 1681].into_iter().enumerate() {
        p[format!("Adventure.Area.Stage{}", i + 1)] = json!({"recognition":"ColorMatch","method":4,"roi":[x,114,86,81],"lower":[210,160,0],"upper":[255,255,100],"count":1800,"connected":false});
    }
    p["Adventure.Area.DurationBorder"] = json!({"recognition":"ColorMatch","method":4,"roi":[0,0,1,1],"lower":[200,200,0],"upper":[255,255,100],"count":400,"connected":false});
    for (name, node) in p.as_object_mut().unwrap() {
        if !name.starts_with("Adventure.") {
            continue;
        }
        for (key, value) in [
            ("action", json!("DoNothing")),
            ("next", json!([])),
            ("on_error", json!([])),
            ("pre_delay", json!(0)),
            ("post_delay", json!(0)),
        ] {
            node.as_object_mut().unwrap().entry(key).or_insert(value);
        }
    }
    p
}
pub fn templates() -> [(&'static str, &'static [u8]); 1] {
    [(
        "image/suibian-temple/squads-open.png",
        include_bytes!("../../../resources/suibian-temple/squads-open.png"),
    )]
}
