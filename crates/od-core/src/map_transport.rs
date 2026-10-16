//! 地图可见目标的首条固定路线；滚动搜索和打开地图尚未迁移。
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const AREA_NAME: &str = "澄辉坪";
pub const POINT_NAME: &str = "汀曼咖啡";
pub const AREA_RECT: [i32; 4] = [0, 90, 1920, 930];
pub const POINT_RECT: [i32; 4] = [70, 1000, 1848, 1058];
pub const CONFIRM_RECT: [i32; 4] = [1024, 600, 1232, 708];
pub const BACK_RECT: [i32; 4] = [82, 13, 150, 90];

#[derive(Debug, Clone, Copy)]
pub enum MapStep {
    Inspect,
    SelectArea,
    SelectPoint,
    WaitDialog,
    Confirm,
    WaitArrival,
}
impl MapStep {
    pub fn entry(self) -> &'static str {
        match self {
            Self::Inspect => "Map.Inspect",
            Self::SelectArea => "Map.SelectArea",
            Self::SelectPoint => "Map.SelectPoint",
            Self::WaitDialog => "Map.WaitDialog",
            Self::Confirm => "Map.Confirm",
            Self::WaitArrival => "Map.WaitArrival",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapPhase {
    Inspect,
    SelectArea,
    SelectPoint,
    InspectDialog,
    Confirm,
    WaitArrival,
    Done,
}

pub fn pipeline() -> Value {
    let mut nodes = json!({
        "Map.AreaText": {"recognition":"OCR","roi":[0,90,1920,840],"expected":["^澄辉坪$"]},
        "Map.PointText": {"recognition":"OCR","roi":[70,1000,1778,58],"expected":["^汀曼咖啡$"]},
        "Map.Prompt": {"recognition":"OCR","roi":[700,480,520,90],"expected":["^是否传送至该地点[？?]$"]},
        "Map.ConfirmText": {"recognition":"OCR","roi":[1024,600,208,108],"expected":["^确认$"]},
        "Map.Back": {"recognition":"TemplateMatch","roi":[82,13,68,77],"template":["map/back.png"],"method":5,"green_mask":true,"threshold":0.7},
        "Map.WorldFunction": {"recognition":"TemplateMatch","roi":[1776,28,93,89],"template":["map/function_menu.png"],"method":5,"green_mask":true,"threshold":0.7},
        "Map.WorldPreset": {"recognition":"TemplateMatch","roi":[1225,44,280,58],"template":["map/preset_config.png"],"method":5,"green_mask":true,"threshold":0.7},
        "Map.World": {"recognition":"And","all_of":["Map.WorldFunction","Map.WorldPreset"]},
        "Map.Dialog": {"recognition":"And","all_of":["Map.Prompt","Map.ConfirmText"],"box_index":1},
        "Map.Area": {"recognition":"And","all_of":["Map.Back","Map.AreaText"],"box_index":1},
        "Map.Point": {"recognition":"And","all_of":["Map.Back","Map.PointText"],"box_index":1},
        "Map.Inspect": {"next":["Map.Dialog","Map.World","Map.Point","Map.Area"]},
        "Map.SelectArea": {"next":["Map.ClickArea"]},
        "Map.ClickArea": {"recognition":"And","all_of":["Map.Area"],"action":"LongPress","duration":100,"next":["Map.Point"]},
        "Map.SelectPoint": {"next":["Map.ClickPoint"]},
        "Map.ClickPoint": {"recognition":"And","all_of":["Map.Point"],"action":"LongPress","duration":100,"next":["Map.Dialog"]},
        "Map.WaitDialog": {"next":["Map.Dialog"]},
        "Map.Confirm": {"next":["Map.ClickConfirm"]},
        "Map.ClickConfirm": {"recognition":"And","all_of":["Map.Dialog"],"action":"LongPress","duration":100},
        "Map.WaitArrival": {"next":["Map.World"]}
    });
    for node in nodes.as_object_mut().expect("固定节点对象").values_mut() {
        let object = node.as_object_mut().expect("固定节点对象");
        if object.get("recognition").is_some_and(|kind| kind == "OCR") {
            object.insert("model".into(), json!("ppocrv5"));
            object.insert("threshold".into(), json!(0.6));
        }
        for (key, value) in [
            ("recognition", json!("DirectHit")),
            ("action", json!("DoNothing")),
            ("pre_delay", json!(0)),
            ("post_delay", json!(0)),
            ("rate_limit", json!(50)),
            ("timeout", json!(60_000)),
            ("next", json!([])),
            ("on_error", json!([])),
            ("inverse", json!(false)),
            ("enabled", json!(true)),
        ] {
            object.entry(key).or_insert(value);
        }
    }
    nodes
}
