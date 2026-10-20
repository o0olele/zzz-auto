use serde_json::{Value, json};

pub fn pipeline() -> Value {
    let mut p = crate::random_play::pipeline();
    for name in [
        "OpenMenu",
        "HoldAlt",
        "ClickMenu",
        "ReleaseAlt",
        "Navigated",
        "MoreEntry",
        "MoreTitle",
    ] {
        p[format!("Teams.{name}")] = serde_json::from_str(
            &p[format!("Redeem.{name}")]
                .to_string()
                .replace("Redeem.", "Teams."),
        )
        .unwrap();
    }
    p["Teams.Entry"] = ocr([478, 282, 964, 626], "^预备编队$");
    p["Teams.World"] = and(&["Base.World"]);
    p["Teams.Menu"] = and(&["Base.Menu"]);
    p["Teams.More"] = and(&["Teams.MoreTitle", "Teams.Entry"]);
    p["Teams.Back"] = json!({"recognition":"TemplateMatch","template":"daily/back.png","roi":[54,12,115,80],"green_mask":true,"threshold":0.7});
    p["Teams.LeftCount"] = ocr([765, 100, 122, 760], "^[0-3][/／1][0-3]$");
    p["Teams.RightCount"] = ocr([1580, 100, 122, 760], "^[0-3][/／1][0-3]$");
    p["Teams.Grid"] = and(&["Teams.Back", "Teams.LeftCount", "Teams.RightCount"]);
    p["Teams.SelectLeft"] = ocr([795, 215, 145, 745], ".*SELECT.*|^TEAM.*");
    p["Teams.SelectRight"] = ocr([1610, 215, 145, 745], ".*SELECT.*|^TEAM.*");
    p["Teams.Deploy"] = ocr([1570, 985, 310, 85], "^预备出战$");
    p["Teams.Selection"] = json!({"recognition":"Or","any_of":["Teams.SelectLeft","Teams.SelectRight","Teams.Deploy"]});
    p["Teams.Inspect"] = next(&[
        "Teams.Selection",
        "Teams.More",
        "Teams.Grid",
        "Teams.Menu",
        "Teams.World",
    ]);
    p["Teams.OpenMore"] = click(&["Teams.Menu", "Teams.MoreEntry"], "Teams.Navigated");
    p["Teams.OpenTeams"] = click(&["Teams.More", "Teams.Entry"], "Teams.WaitOpened");
    p["Teams.WaitOpened"] = next(&["Teams.Selection", "Teams.Opened"]);
    p["Teams.Opened"] = and(&["Teams.Grid"]);
    p["Teams.ResetTop"] = next(&["Teams.Selection", "Teams.DragTop"]);
    p["Teams.DragTop"] = and(&["Teams.Grid"]);
    p["Teams.DragTop"]["action"] = json!("Swipe");
    p["Teams.DragTop"]["begin"] = json!([960, 300]);
    p["Teams.DragTop"]["end"] = json!([960, 900]);
    p["Teams.DragTop"]["duration"] = json!(500);
    p["Teams.DragTop"]["post_delay"] = json!(1000);
    p["Teams.DragTop"]["next"] = json!(["Teams.WaitTop"]);
    p["Teams.WaitTop"] = next(&["Teams.Selection", "Teams.TopReset"]);
    p["Teams.TopReset"] = and(&["Teams.Grid"]);
    p["Teams.Scroll"] = next(&["Teams.Selection", "Teams.Drag"]);
    p["Teams.Drag"] = and(&["Teams.Grid"]);
    p["Teams.Drag"]["action"] = json!("Swipe");
    p["Teams.Drag"]["begin"] = json!([960, 540]);
    p["Teams.Drag"]["end"] = json!([960, 40]);
    p["Teams.Drag"]["duration"] = json!(500);
    p["Teams.Drag"]["post_delay"] = json!(1000);
    p["Teams.Drag"]["next"] = json!(["Teams.WaitScrolled"]);
    p["Teams.WaitScrolled"] = next(&["Teams.Selection", "Teams.Scrolled"]);
    p["Teams.Scrolled"] = and(&["Teams.Grid"]);
    for stage in ["Return", "ResetReturn"] {
        p[format!("Teams.{stage}")] = next(&[
            "Teams.Selection",
            "Teams.MoreReturn",
            "Teams.GridReturn",
            "Teams.World",
            "Teams.MenuReturn",
        ]);
    }
    for (name, guard) in [
        ("MoreReturn", "Teams.More"),
        ("GridReturn", "Teams.Grid"),
        ("MenuReturn", "Teams.Menu"),
    ] {
        p[format!("Teams.{name}")] = and(&[guard]);
        p[format!("Teams.{name}")]["action"] = json!("KeyDown");
        p[format!("Teams.{name}")]["key"] = json!(27);
        p[format!("Teams.{name}")]["auto_up"] = json!(true);
        p[format!("Teams.{name}")]["next"] = json!(["Teams.EscapeUp"]);
    }
    p["Teams.EscapeUp"] = json!({"action":"KeyUp","key":27,"next":["Teams.Return"]});
    for (name, node) in p.as_object_mut().unwrap() {
        if !name.starts_with("Teams.") {
            continue;
        }
        for (key, value) in [
            ("recognition", json!("DirectHit")),
            ("action", json!("DoNothing")),
            ("pre_delay", json!(0)),
            ("post_delay", json!(250)),
            ("rate_limit", json!(50)),
            ("timeout", json!(10_000)),
            ("next", json!([])),
            ("on_error", json!([])),
        ] {
            node.as_object_mut().unwrap().entry(key).or_insert(value);
        }
    }
    p
}
fn and(nodes: &[&str]) -> Value {
    json!({"recognition":"And","all_of":nodes})
}
fn next(nodes: &[&str]) -> Value {
    json!({"next":nodes})
}
fn ocr(roi: [i32; 4], text: &str) -> Value {
    json!({"recognition":"OCR","model":"ppocrv5","roi":roi,"expected":[text],"threshold":0.6})
}
fn click(nodes: &[&str], next: &str) -> Value {
    let mut p = and(nodes);
    p["box_index"] = json!(nodes.len() - 1);
    p["action"] = json!("LongPress");
    p["duration"] = json!(100);
    p["next"] = json!([next]);
    p
}
