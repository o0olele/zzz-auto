//! 固定 1080p 的咖啡只读识别定义；没有输入动作、后继或完成判定。
use serde_json::{Value, json};

pub struct CoffeeArea {
    pub alias: &'static str,
    pub legacy_name: &'static str,
    pub rect: [i32; 4],
    pub text: &'static str,
}

pub const AREAS: &[CoffeeArea] = &[
    CoffeeArea {
        alias: "Order",
        legacy_name: "点单",
        rect: [1702, 880, 1914, 1020],
        text: "点单",
    },
    CoffeeArea {
        alias: "Menu",
        legacy_name: "咖啡列表",
        rect: [132, 682, 1122, 1054],
        text: "",
    },
    CoffeeArea {
        alias: "Go",
        legacy_name: "对话框前往",
        rect: [728, 600, 928, 692],
        text: "前往",
    },
    CoffeeArea {
        alias: "Confirm",
        legacy_name: "对话框确认",
        rect: [1040, 600, 1228, 688],
        text: "确认",
    },
    CoffeeArea {
        alias: "Skip",
        legacy_name: "点单后跳过",
        rect: [1684, 64, 1850, 200],
        text: "跳过",
    },
    CoffeeArea {
        alias: "ChargeConfirm",
        legacy_name: "电量确认",
        rect: [875, 640, 1200, 754],
        text: "确认",
    },
    CoffeeArea {
        alias: "LimitConfirm",
        legacy_name: "不可贪杯确认",
        rect: [882, 606, 1084, 690],
        text: "确认",
    },
    CoffeeArea {
        alias: "Options",
        legacy_name: "右侧选项区域",
        rect: [1438, 536, 1914, 830],
        text: "",
    },
    CoffeeArea {
        alias: "Title",
        legacy_name: "对话框标题-汀曼大师",
        rect: [880, 800, 1040, 840],
        text: "汀曼大师",
    },
];

pub fn pipeline() -> Value {
    let mut nodes = serde_json::Map::new();
    for area in AREAS {
        let [x, y, x2, y2] = area.rect;
        let mut node = json!({"recognition":"OCR","action":"DoNothing","roi":[x,y,x2-x,y2-y],"model":"ppocrv5"});
        if !area.text.is_empty() {
            node["expected"] = json!([format!("^{}$", area.text)]);
        }
        nodes.insert(format!("Coffee.ReadOnly.{}", area.alias), node);
    }
    for (alias, text) in [("TinmanOption", "汀曼特调"), ("LeaveOption", "不喝了")] {
        let mut node = nodes["Coffee.ReadOnly.Options"].clone();
        node["expected"] = json!([text]);
        nodes.insert(format!("Coffee.ReadOnly.{alias}"), node);
    }
    Value::Object(nodes)
}
