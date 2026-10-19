//! 每日活跃度奖励的固定只读识别；可见标记不表示本次领取成功。
use serde_json::{Value, json};

pub struct EngagementArea {
    pub alias: &'static str,
    pub legacy_name: &'static str,
    pub rect: [i32; 4],
    pub text: &'static str,
}

pub const AREAS: &[EngagementArea] = &[
    EngagementArea {
        alias: "DailyTitle",
        legacy_name: "今日最大活跃度",
        rect: [417, 250, 646, 320],
        text: "今日最大活跃度",
    },
    EngagementArea {
        alias: "Confirm",
        legacy_name: "活跃度奖励-确认",
        rect: [882, 704, 1086, 754],
        text: "确认",
    },
    EngagementArea {
        alias: "Preview",
        legacy_name: "活跃度奖励-奖励预览",
        rect: [429, 195, 655, 303],
        text: "奖励预览",
    },
    EngagementArea {
        alias: "Completed",
        legacy_name: "活跃度奖励-4",
        rect: [1532, 250, 1602, 350],
        text: "",
    },
];

pub const RAW_SHA256: &str = "1e8ea03d840539959caa967e7a21aee846829c6c870a2f845010581f1ee7cb1d";
pub const MASK_SHA256: &str = "30c5f7f5613aeff79ac9c6314e3953c0bcbd02aefe046e6ee02e74f6ba0062ac";
pub const TEMPLATE_SHA256: &str =
    "a84cf53ae6848ad01c22a6fe4bff8f491fc8ef0d0a54e47c5560d3a1e16c2691";

pub fn pipeline() -> Value {
    let mut nodes = serde_json::Map::new();
    for area in AREAS {
        let [x, y, x2, y2] = area.rect;
        let mut node = json!({"action":"DoNothing","roi":[x,y,x2-x,y2-y]});
        if area.text.is_empty() {
            node["recognition"] = json!("TemplateMatch");
            node["template"] = json!(["engagement-reward/completed.png"]);
            node["threshold"] = json!(0.7);
            node["method"] = json!(5);
            node["green_mask"] = json!(true);
        } else {
            node["recognition"] = json!("OCR");
            node["model"] = json!("ppocrv5");
            node["expected"] = json!([format!("^{}$", area.text)]);
            node["threshold"] = json!(0.6);
        }
        nodes.insert(format!("EngagementReward.ReadOnly.{}", area.alias), node);
    }
    Value::Object(nodes)
}

/// 使用可辨认的标题确定画面；确认和完成图标都不能独立提供上下文。
pub fn visible_view(daily_title: bool, preview: bool) -> &'static str {
    match (daily_title, preview) {
        (true, true) => "ambiguous",
        (true, false) => "daily_visible",
        (false, true) => "reward_preview_visible",
        (false, false) => "unknown",
    }
}
