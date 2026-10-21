//! schema 7 保留旧六版定义，仅增加邦巢同帧只读区域。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = crate::suibian_good_goods_pipeline::pipeline();
    p["BooBox.Area.Text"] = p["GoodGoods.Area.Text"].clone();
    p["BooBox.Area.Selected"] = json!({"recognition":"ColorMatch","method":4,"roi":[1148,250,8,180],"lower":[220,200,0],"upper":[255,255,80],"count":350,"connected":false,"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    p["BooBox.Area.Quantity"] = p["BooBox.Area.Text"].clone();
    p["BooBox.Area.Quantity"]["only_rec"] = json!(true);
    p["BooBox.Area.Quantity"]["roi"] = json!([936, 580, 48, 36]);
    p
}
