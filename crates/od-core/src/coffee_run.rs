//! 澄辉坪对话点单的受控正常流程；从已打开的咖啡选项开始。
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CoffeeKind {
    #[default]
    Tinman,
    Espresso,
}
impl CoffeeKind {
    pub fn options() -> Value {
        json!([
            {"value": Self::Tinman, "label": Self::Tinman.name()},
            {"value": Self::Espresso, "label": Self::Espresso.name()}
        ])
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Tinman => "汀曼特调",
            Self::Espresso => "浓缩咖啡",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CoffeeConfig {
    pub kind: CoffeeKind,
    pub timeout_ms: u32,
    pub start_delay_ms: u32,
}
impl Default for CoffeeConfig {
    fn default() -> Self {
        Self {
            kind: CoffeeKind::Tinman,
            timeout_ms: 60000,
            start_delay_ms: 3000,
        }
    }
}
impl CoffeeConfig {
    pub const MAX_TIMEOUT_MS: u32 = 60_000;
    pub const MAX_START_DELAY_MS: u32 = 10_000;

    pub fn fields() -> Value {
        json!([
            {"key": "kind", "kind": "select", "label": "默认咖啡", "options": CoffeeKind::options()},
            {"key": "timeout_ms", "kind": "integer", "label": "默认咖啡超时（ms）", "min": 1, "max": Self::MAX_TIMEOUT_MS},
            {"key": "start_delay_ms", "kind": "integer", "label": "默认咖啡启动 / 继续延时（ms）", "min": 0, "max": Self::MAX_START_DELAY_MS}
        ])
    }

    pub fn validate(self) -> Result<(), &'static str> {
        if !(1..=Self::MAX_TIMEOUT_MS).contains(&self.timeout_ms)
            || self.start_delay_ms > Self::MAX_START_DELAY_MS
        {
            return Err("咖啡超时须为 1..60000ms，启动 / 继续延时须为 0..10000ms");
        }
        Ok(())
    }
    pub fn recovery_inputs() -> [crate::InputProbeConfig; 1] {
        crate::map_transport::MapConfig::recovery_inputs()
    }
    pub fn overrides(self) -> Value {
        let roi = match self.kind {
            CoffeeKind::Tinman => [1440, 560, 300, 52],
            CoffeeKind::Espresso => [1440, 640, 300, 52],
        };
        json!({"Coffee.Option":{"expected":[self.kind.name()],"roi":roi},"Coffee.BuffTitle":{"expected":[format!("^.*{}.*已生效$",self.kind.name())]}})
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CoffeeState {
    Running,
    Paused,
    Succeeded,
    Failed,
    Stopped,
    TimedOut,
}
impl CoffeeState {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running | Self::Paused)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CoffeePhase {
    Inspect,
    Order,
    WaitEnergy,
    ConfirmEnergy,
    AfterEnergy,
    ConfirmBuff,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CoffeeProgress {
    pub config: CoffeeConfig,
    pub state: CoffeeState,
    pub phase: CoffeePhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub approach: Option<crate::coffee_approach::CoffeeApproachProgress>,
    pub order_attempted: bool,
    pub energy_seen: bool,
    pub inputs_released: bool,
    #[ts(type = "number")]
    pub active_ms: u64,
}
impl CoffeeProgress {
    pub fn new(config: CoffeeConfig) -> Self {
        Self {
            config,
            state: CoffeeState::Running,
            phase: CoffeePhase::Inspect,
            approach: None,
            order_attempted: false,
            energy_seen: false,
            inputs_released: true,
            active_ms: 0,
        }
    }
}

pub fn pipeline() -> Value {
    let mut nodes = json!({
        "Coffee.Title":{"recognition":"OCR","roi":[880,800,160,40],"expected":["^汀曼大师$"]},
        "Coffee.Option":{"recognition":"OCR","roi":[1440,560,300,52],"only_rec":true,"expected":["汀曼特调"]},
        "Coffee.Options":{"recognition":"And","all_of":["Coffee.Title","Coffee.Option"],"box_index":1},
        "Coffee.EnergyPrompt":{"recognition":"OCR","roi":[870,380,180,80],"expected":["^获得$"]},
        "Coffee.EnergyLabel":{"recognition":"OCR","roi":[880,608,180,50],"expected":["^电量$"]},
        "Coffee.EnergyConfirm":{"recognition":"OCR","roi":[875,690,325,90],"expected":["^确认$"]},
        "Coffee.Energy":{"recognition":"And","all_of":["Coffee.EnergyPrompt","Coffee.EnergyLabel","Coffee.EnergyConfirm"],"box_index":2},
        "Coffee.BuffTitle":{"recognition":"OCR","roi":[710,450,540,68],"expected":["^.*汀曼特调.*已生效$"]},
        "Coffee.BuffConfirm":{"recognition":"OCR","roi":[1010,620,240,80],"expected":["^确认$"]},
        "Coffee.Buff":{"recognition":"And","all_of":["Coffee.BuffTitle","Coffee.BuffConfirm"],"box_index":1},
        "Coffee.WorldFunction":{"recognition":"TemplateMatch","roi":[1776,28,93,89],"template":["coffee/world_function.png"],"method":5,"threshold":0.8},
        "Coffee.WorldText":{"recognition":"OCR","roi":[106,1010,154,50],"expected":["^快捷轮盘$"]},
        "Coffee.World":{"recognition":"And","all_of":["Coffee.WorldFunction","Coffee.WorldText"]},
        "Coffee.Inspect":{"next":["Coffee.Energy","Coffee.Buff","Coffee.World","Coffee.Options"]},
        "Coffee.Order":{"next":["Coffee.ClickOrder"]},
        "Coffee.ClickOrder":{"recognition":"And","all_of":["Coffee.Options"],"action":"LongPress","duration":100,"next":["Coffee.Energy"]},
        "Coffee.WaitEnergy":{"next":["Coffee.Energy"]},
        "Coffee.ConfirmEnergy":{"next":["Coffee.ClickEnergy"]},
        "Coffee.ClickEnergy":{"recognition":"And","all_of":["Coffee.Energy"],"action":"LongPress","duration":100},
        "Coffee.AfterEnergy":{"next":["Coffee.Buff","Coffee.World","Coffee.Energy"]},
        "Coffee.ConfirmBuff":{"next":["Coffee.ClickBuff"]},
        "Coffee.ClickBuff":{"recognition":"And","all_of":["Coffee.Buff"],"action":"LongPress","duration":100}
    });
    for node in nodes.as_object_mut().unwrap().values_mut() {
        let object = node.as_object_mut().unwrap();
        if object.get("recognition").is_some_and(|kind| kind == "OCR") {
            object.insert("model".into(), json!("ppocrv5"));
            object.insert("threshold".into(), json!(0.6));
        }
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
    }
    nodes
}
