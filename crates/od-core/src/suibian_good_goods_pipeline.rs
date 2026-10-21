//! schema 6 在冻结的五版随便观区域上添加好物铺只读识别。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = crate::suibian_sales_pipeline::pipeline();
    p["GoodGoods.Area.Text"] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[0,0,1920,1080],"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    p["GoodGoods.Area.Business"] = json!({"recognition":"ColorMatch","method":4,"roi":[65,685,250,75],"lower":[220,200,0],"upper":[255,255,80],"count":7000,"connected":false,"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    p["GoodGoods.Area.Payment"] = p["GoodGoods.Area.Text"].clone();
    p["GoodGoods.Area.Payment"]["roi"] = json!([750, 735, 260, 65]);
    p
}
