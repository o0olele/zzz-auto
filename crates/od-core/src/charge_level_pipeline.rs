use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = json!({});
    for (name, roi) in [
        ("Modal", [570, 290, 740, 520]),
        ("Street", [238, 24, 150, 58]),
        ("Title", [750, 168, 726, 97]),
        ("Header", [1566, 24, 246, 48]),
        ("Choices", [1570, 100, 258, 324]),
    ] {
        p[format!("ChargeLevel.{name}")] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":roi,"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    }
    p
}
