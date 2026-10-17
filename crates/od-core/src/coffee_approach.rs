//! 固定咖啡路线的开图、传送与对话检查点。
use crate::{InputMode, InputProbeAction, InputProbeConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CoffeeApproachPhase {
    #[default]
    InspectWorld,
    OpenMap,
    WaitMap,
    SelectArea,
    SelectPoint,
    WaitDialog,
    Confirm,
    WaitArrival,
    Interact,
    WaitDialogue,
    AdvanceDialogue,
    Done,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct CoffeeApproachProgress {
    pub phase: CoffeeApproachPhase,
    pub dialogue_advances: u8,
}

pub fn recovery_inputs() -> [InputProbeConfig; 3] {
    let mouse = crate::coffee_run::CoffeeConfig::recovery_inputs()[0];
    [
        mouse,
        InputProbeConfig {
            mode: InputMode::Foreground,
            action: InputProbeAction::Key { key: 78 },
            hold_ms: 1,
            start_delay_ms: 0,
        },
        InputProbeConfig {
            mode: InputMode::Foreground,
            action: InputProbeAction::Key { key: 70 },
            hold_ms: 1,
            start_delay_ms: 0,
        },
    ]
}

pub fn pipeline() -> Value {
    let mut nodes = crate::coffee_run::pipeline();
    let extra = json!({
        "CoffeeApproach.InspectWorld":{"next":["Map.Dialog","Coffee.Energy","Coffee.Buff","Coffee.Options","Map.Area","Map.Point","Map.World"]},
        "CoffeeApproach.OpenMap":{"next":["CoffeeApproach.MapDown"]},
        "CoffeeApproach.MapDown":{"recognition":"And","all_of":["Map.World"],"action":"KeyDown","key":78,"auto_up":true,"post_delay":100,"next":["CoffeeApproach.MapUp"]},
        "CoffeeApproach.MapUp":{"action":"KeyUp","key":78,"next":["Map.Area","Map.Point"]},
        "CoffeeApproach.WaitMap":{"next":["Map.Area","Map.Point"]},
        "CoffeeApproach.Interact":{"next":["CoffeeApproach.InteractDown"]},
        "CoffeeApproach.InteractDown":{"recognition":"And","all_of":["Map.World"],"pre_delay":1000,"action":"KeyDown","key":70,"auto_up":true,"post_delay":200,"next":["CoffeeApproach.InteractUp"]},
        "CoffeeApproach.InteractUp":{"action":"KeyUp","key":70,"next":["Coffee.Options","Coffee.Title"]},
        "CoffeeApproach.WaitDialogue":{"next":["Coffee.Options","Coffee.Title"]},
        "CoffeeApproach.AdvanceDialogue":{"next":["Coffee.Options","CoffeeApproach.Advance","Map.World"]},
        "CoffeeApproach.Advance":{"recognition":"And","all_of":["Coffee.Title"],"action":"LongPress","duration":100,"post_delay":750},
        "CoffeeApproach.WaitOptions":{"next":["Coffee.Options","Map.World"]}
    });
    for (name, mut node) in extra.as_object().expect("固定前置节点").clone() {
        let object = node.as_object_mut().expect("固定前置节点对象");
        for (key, value) in [
            ("recognition", json!("DirectHit")),
            ("action", json!("DoNothing")),
            ("pre_delay", json!(0)),
            ("post_delay", json!(150)),
            ("rate_limit", json!(100)),
            ("timeout", json!(60000)),
            ("next", json!([])),
            ("on_error", json!([])),
            ("inverse", json!(false)),
            ("enabled", json!(true)),
        ] {
            object.entry(key).or_insert(value);
        }
        nodes[name] = node;
    }
    nodes
}
