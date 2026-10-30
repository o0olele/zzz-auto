use serde_json::{Value, json};
pub fn templates() -> Vec<(&'static str, &'static [u8])> {
    crate::basic_daily::templates()
        .into_iter()
        .filter(|(name, _)| ["back", "close"].contains(name))
        .collect()
}
pub fn pipeline() -> Value {
    json!({
        "ChargeNav.Modal":{"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[570,290,740,520],"action":"DoNothing","next":[],"on_error":[]},
        "ChargeNav.Title":{"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[178,18,152,68],"action":"DoNothing","next":[],"on_error":[]},
        "ChargeNav.Back":{"recognition":"TemplateMatch","template":["charge-navigation/back.png"],"roi":[82,13,68,77],"threshold":0.7,"method":5,"green_mask":true,"action":"DoNothing","next":[],"on_error":[]},
        "ChargeNav.Close":{"recognition":"TemplateMatch","template":["charge-navigation/close.png"],"roi":[120,80,1680,920],"threshold":0.7,"method":5,"green_mask":false,"action":"DoNothing","next":[],"on_error":[]},
        "ChargeNav.Tabs":{"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[708,140,928,80],"action":"DoNothing","next":[],"on_error":[]},
        "ChargeNav.Categories":{"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[164,287,265,583],"action":"DoNothing","next":[],"on_error":[]}
    })
}
