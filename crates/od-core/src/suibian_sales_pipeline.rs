//! schema 5 仅添加售卖只读区域；历史四版 Pipeline 保持原样。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = crate::suibian_yum_pipeline::pipeline();
    p["Sales.Area.Text"] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[0,0,1920,1080],"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    p["Sales.Area.Selected"] = p["Craft.Area.Selected"].clone();
    p
}
