use serde_json::{Value, json};

pub fn pipeline() -> Value {
    let mut p = crate::dismantle::pipeline();
    for name in [
        "OpenMenu",
        "HoldAlt",
        "ClickMenu",
        "ReleaseAlt",
        "Navigated",
    ] {
        p[format!("Redeem.{name}")] = serde_json::from_str(
            &p[format!("Fund.{name}")]
                .to_string()
                .replace("Fund.", "Redeem."),
        )
        .unwrap();
    }
    for (name, roi, text) in [
        ("MoreEntry", [200, 924, 1528, 134], "^更多$"),
        ("MoreTitle", [510, 195, 300, 90], "^更多功能$"),
        ("CodeEntry", [474, 280, 966, 624], "^兑换码$"),
        ("InputTitle", [630, 330, 300, 90], "^兑换码$"),
        ("SubmitText", [878, 690, 196, 48], "^兑换$"),
        ("InputValue", [718, 536, 400, 74], "^ZZZ_AUTO_UNSET_CODE$"),
        ("Redeemed", [420, 240, 1080, 650], "^兑换成功.*"),
        (
            "AlreadyRedeemed",
            [420, 240, 1080, 650],
            ".*(兑换码|礼包码).*(已使用|已兑换).*|.*(礼包|奖励).*已领取.*",
        ),
        (
            "Invalid",
            [420, 240, 1080, 650],
            ".*(兑换码|礼包码).*(无效|不存在).*|^无效的兑换码.*",
        ),
        (
            "Expired",
            [420, 240, 1080, 650],
            ".*(兑换码|礼包码).*(过期|失效).*",
        ),
        ("ResultConfirm", [600, 500, 720, 400], "^确认$|^确定$"),
    ] {
        p[format!("Redeem.{name}")] = ocr(roi, text);
    }
    p["Redeem.World"] = and(&["Base.World"]);
    p["Redeem.Menu"] = and(&["Base.Menu"]);
    p["Redeem.More"] = and(&["Redeem.MoreTitle", "Redeem.CodeEntry"]);
    p["Redeem.Input"] = and(&["Redeem.InputTitle", "Redeem.SubmitText"]);
    p["Redeem.Result"] = json!({"recognition":"Or","any_of":["Redeem.Expired","Redeem.Invalid","Redeem.AlreadyRedeemed","Redeem.Redeemed"]});
    p["Redeem.Inspect"] = next(&[
        "Redeem.Result",
        "Redeem.Input",
        "Redeem.More",
        "Redeem.Menu",
        "Redeem.World",
    ]);
    p["Redeem.OpenMore"] = click(&["Redeem.Menu", "Redeem.MoreEntry"], "Redeem.Navigated");
    p["Redeem.OpenCode"] = click(&["Redeem.More", "Redeem.CodeEntry"], "Redeem.Navigated");
    // 先清空旧输入；Maa 管理 Ctrl、A、Backspace 的持有与释放，输入正文严格来自冻结码表。
    p["Redeem.FillCode"] = next(&["Redeem.Blocked", "Redeem.FocusInput"]);
    p["Redeem.Blocked"] = and(&["Redeem.Result"]);
    p["Redeem.FocusInput"] = click(&["Redeem.Input"], "Redeem.ControlDown");
    p["Redeem.FocusInput"]["target"] = json!([740, 545, 320, 55]);
    p["Redeem.ControlDown"] = key("KeyDown", 17, "Redeem.SelectAllDown");
    p["Redeem.SelectAllDown"] = key("KeyDown", 65, "Redeem.SelectAllUp");
    p["Redeem.SelectAllUp"] = key("KeyUp", 65, "Redeem.ControlUp");
    p["Redeem.ControlUp"] = key("KeyUp", 17, "Redeem.EraseDown");
    p["Redeem.EraseDown"] = key("KeyDown", 8, "Redeem.EraseUp");
    p["Redeem.EraseUp"] = key("KeyUp", 8, "Redeem.Type");
    p["Redeem.Type"] = and(&["Redeem.Input"]);
    p["Redeem.Type"]["action"] = json!("InputText");
    p["Redeem.Type"]["input_text"] = json!("ZZZ_AUTO_UNSET_CODE");
    p["Redeem.Type"]["next"] = json!(["Redeem.Filled"]);
    p["Redeem.Filled"] = and(&["Redeem.Input"]);
    p["Redeem.VerifyInput"] = next(&["Redeem.Blocked", "Redeem.Verified"]);
    p["Redeem.Verified"] = and(&["Redeem.Input", "Redeem.InputValue"]);
    p["Redeem.SubmitCode"] = next(&["Redeem.Blocked", "Redeem.ClickSubmit"]);
    p["Redeem.ClickSubmit"] = click(
        &["Redeem.Verified", "Redeem.SubmitText"],
        "Redeem.Submitted",
    );
    p["Redeem.Submitted"] = json!({});
    p["Redeem.AwaitResult"] = next(&[
        "Redeem.Expired",
        "Redeem.Invalid",
        "Redeem.AlreadyRedeemed",
        "Redeem.Redeemed",
    ]);
    for stage in ["DismissResult", "DismissOrphan"] {
        p[format!("Redeem.{stage}")] = next(&[
            "Redeem.ConfirmResult",
            "Redeem.ResultPending",
            "Redeem.Dismissed",
        ]);
    }
    p["Redeem.ConfirmResult"] = click(
        &["Redeem.Result", "Redeem.ResultConfirm"],
        "Redeem.WaitResultClose",
    );
    p["Redeem.ResultPending"] = and(&["Redeem.Result"]);
    p["Redeem.ResultPending"]["next"] = json!(["Redeem.WaitResultClose"]);
    p["Redeem.WaitResultClose"] = next(&["Redeem.ResultPending", "Redeem.Dismissed"]);
    p["Redeem.Dismissed"] = and(&["Redeem.Input"]);
    p["Redeem.Return"] = next(&["Redeem.Blocked", "Redeem.World", "Redeem.CloseKnown"]);
    p["Redeem.Known"] =
        json!({"recognition":"Or","any_of":["Redeem.Input","Redeem.More","Redeem.Menu"]});
    p["Redeem.CloseKnown"] = and(&["Redeem.Known"]);
    p["Redeem.CloseKnown"]["action"] = json!("KeyDown");
    p["Redeem.CloseKnown"]["key"] = json!(27);
    p["Redeem.CloseKnown"]["auto_up"] = json!(true);
    p["Redeem.CloseKnown"]["next"] = json!(["Redeem.EscapeUp"]);
    p["Redeem.EscapeUp"] = key("KeyUp", 27, "Redeem.Return");
    for (name, node) in p.as_object_mut().unwrap() {
        if !name.starts_with("Redeem.") {
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

/// 聚焦后标题会被 OCR 输出为「兌換码」；旧包的固定定义保留，
/// 仅在本次任务兼容「换／換」，输入正文仍按冻结码完整核对。
pub fn ocr_overrides(pipeline: &Value) -> Value {
    let mut overrides = json!({});
    for (name, node) in pipeline.as_object().expect("固定 Pipeline 是对象") {
        if !name.starts_with("Redeem.") || node["recognition"] != "OCR" {
            continue;
        }
        let Some(expected) = node["expected"].as_array() else {
            continue;
        };
        if expected
            .iter()
            .any(|pattern| pattern.as_str().is_some_and(|text| text.contains('换')))
        {
            let expected: Vec<_> = expected
                .iter()
                .map(|pattern| {
                    pattern
                        .as_str()
                        .expect("固定 OCR 规则是文字")
                        .replace('换', "[换換]")
                })
                .collect();
            overrides[name] = json!({"expected":expected});
        }
    }
    overrides
}

fn and(nodes: &[&str]) -> Value {
    json!({"recognition":"And","all_of":nodes})
}
fn next(nodes: &[&str]) -> Value {
    json!({"next":nodes})
}
fn ocr(roi: [i32; 4], text: &str) -> Value {
    // 同字形常被 OCR 输出为繁体「兌」；仍要求整条语义匹配。
    json!({"recognition":"OCR","model":"ppocrv5","roi":roi,"expected":[text.replace('兑', "[兑兌]")],"threshold":0.6})
}
fn key(action: &str, key: i32, next: &str) -> Value {
    json!({"action":action,"key":key,"auto_up":true,"next":[next]})
}
fn click(nodes: &[&str], next: &str) -> Value {
    let mut value = and(nodes);
    value["box_index"] = json!(nodes.len() - 1);
    value["action"] = json!("LongPress");
    value["duration"] = json!(100);
    value["next"] = json!([next]);
    value
}
