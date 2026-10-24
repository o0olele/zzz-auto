use serde_json::{Value, json};
pub fn templates() -> Vec<(&'static str, &'static [u8])> {
    crate::basic_daily::templates()
        .into_iter()
        .filter(|(n, _)| *n == "back")
        .collect()
}
pub fn pipeline() -> Value {
    let mut p = crate::simulation_pipeline::pipeline();
    p["Mission.Names"] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[128,724,1668,158],"action":"DoNothing","next":[],"on_error":[]});
    p["Mission.Back"] = json!({"recognition":"TemplateMatch","template":["simulation_mission/back.png"],"roi":[82,13,68,77],"threshold":0.7,"method":5,"green_mask":true,"action":"DoNothing","next":[],"on_error":[]});
    p
}
