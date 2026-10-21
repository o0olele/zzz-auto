//! 只收录已迁移的托管子流程，独立版本不改变既有日常包的定义。
use serde_json::{Value, json};

pub fn pipeline() -> Value {
    let mut p = json!({});
    for (name, roi, text) in [
        ("Neighbor", [1332, 922, 256, 65], "^邻里街坊$"),
        ("Resident", [90, 730, 180, 75], "^派驻邦布$"),
        ("ManagedText", [1200, 665, 530, 100], "^自动托管中$"),
        ("ManualText", [1246, 578, 180, 65], "^经营$"),
        ("ManagedLog", [80, 950, 200, 85], "^托管中$"),
        ("ManualLog", [80, 950, 200, 85], "^经营方针$"),
        ("LogTitle", [1210, 105, 245, 80], "^经营日志$"),
        ("AutoTitle", [1460, 105, 200, 80], "^自动托管$"),
        ("LogLimit", [330, 190, 320, 75], "^当前周期获取上限$"),
        ("StopText", [1060, 865, 320, 70], "^停止托管$"),
        ("StartText", [1060, 865, 320, 70], "^开始托管$"),
        ("ClaimText", [1060, 865, 320, 70], "^领取收益$"),
        ("StopTitle", [720, 450, 490, 90], ".*确认暂停自动托管.*"),
        ("StopCancel", [680, 600, 290, 100], "^取消$"),
        ("StopConfirm", [1000, 600, 290, 100], "^确认$"),
        (
            "StartTitle",
            [450, 380, 1040, 90],
            ".*已暂停正在运作的制造台和游历小队.*",
        ),
        ("StartConfirm", [820, 680, 320, 95], "^确认$"),
        ("RewardTitle", [820, 380, 300, 90], "^获得奖励$"),
        ("RewardConfirm", [810, 665, 310, 95], "^确认$"),
    ] {
        p[format!("Hosting.Area.{name}")] = json!({"recognition":"OCR","model":"ppocrv5","roi":roi,"expected":[text],"threshold":0.6});
    }
    p["Hosting.Area.Back"] = json!({"recognition":"TemplateMatch","roi":[60,10,115,85],"template":"suibian-temple/back.png","threshold":0.7,"green_mask":true});
    for (name, areas) in [
        (
            "EntryManaged",
            vec!["Neighbor", "Resident", "Back", "ManagedText"],
        ),
        (
            "EntryManual",
            vec!["Neighbor", "Resident", "Back", "ManualText"],
        ),
        ("Log", vec!["LogTitle", "AutoTitle", "LogLimit", "Back"]),
        ("StopPrompt", vec!["StopTitle", "StopCancel", "StopConfirm"]),
        ("StartPrompt", vec!["StartTitle", "StartConfirm"]),
        ("Reward", vec!["RewardTitle", "RewardConfirm"]),
    ] {
        let areas: Vec<_> = areas.iter().map(|s| format!("Hosting.Area.{s}")).collect();
        p[format!("Hosting.{name}")] = and(&areas);
    }
    for (name, button) in [
        ("LogRunning", "StopText"),
        ("LogStopped", "StartText"),
        ("Claimable", "ClaimText"),
    ] {
        p[format!("Hosting.{name}")] =
            and(&["Hosting.Log".into(), format!("Hosting.Area.{button}")]);
    }
    let contexts = [
        "Hosting.StopPrompt",
        "Hosting.StartPrompt",
        "Hosting.Reward",
        "Hosting.Claimable",
        "Hosting.LogRunning",
        "Hosting.LogStopped",
        "Hosting.EntryManaged",
        "Hosting.EntryManual",
    ];
    p["Hosting.Inspect"] = json!({"next":contexts});
    for stage in ["WaitStop", "WaitClaim", "WaitReward", "WaitStart"] {
        // 这些入口只读。提交的 Tasker 即使被暂停取消，也不能据背景旧按钮再次提交。
        p[format!("Hosting.{stage}")] = json!({"next":contexts});
    }
    p["Hosting.OpenLog"] = json!({"next":["Hosting.StopPrompt","Hosting.StartPrompt","Hosting.Reward","Hosting.Claimable","Hosting.LogRunning","Hosting.LogStopped","Hosting.OpenManagedLog","Hosting.OpenManualLog"]});
    p["Hosting.OpenManagedLog"] = click(&["Hosting.EntryManaged", "Hosting.Area.ManagedLog"], None);
    p["Hosting.OpenManualLog"] = click(&["Hosting.EntryManual", "Hosting.Area.ManualLog"], None);
    for name in ["OpenManagedLog", "OpenManualLog"] {
        p[format!("Hosting.{name}")]["next"] = json!(contexts);
    }
    // 先识别模态窗口，再允许点击背景。不能以全屏“确认”猜测弹窗归属。
    for (stage, context, button, end) in [
        ("Stop", "LogRunning", "StopText", "StopClicked"),
        ("Claim", "Claimable", "ClaimText", "ClaimClicked"),
        ("Start", "LogStopped", "StartText", "StartClicked"),
    ] {
        let action = format!("Hosting.Action.{stage}");
        p[format!("Hosting.{stage}")] =
            json!({"next":["Hosting.StopPrompt","Hosting.StartPrompt","Hosting.Reward",action]});
        p[&action] = click(
            &[
                &format!("Hosting.{context}"),
                &format!("Hosting.Area.{button}"),
            ],
            Some(end),
        );
    }
    for (stage, context, button, end) in [
        ("ConfirmStop", "StopPrompt", "StopConfirm", "StopConfirmed"),
        (
            "ConfirmStart",
            "StartPrompt",
            "StartConfirm",
            "StartConfirmed",
        ),
        (
            "ConfirmReward",
            "Reward",
            "RewardConfirm",
            "RewardConfirmed",
        ),
    ] {
        p[format!("Hosting.{stage}")] = click(
            &[
                &format!("Hosting.{context}"),
                &format!("Hosting.Area.{button}"),
            ],
            Some(end),
        );
    }
    for end in [
        "StopClicked",
        "StopConfirmed",
        "ClaimClicked",
        "RewardConfirmed",
        "StartClicked",
        "StartConfirmed",
    ] {
        p[format!("Hosting.{end}")] = json!({});
    }
    p["Hosting.ReturnEntry"] = json!({"next":["Hosting.StopPrompt","Hosting.StartPrompt","Hosting.Reward","Hosting.EntryManaged","Hosting.CloseLog"]});
    p["Hosting.CloseLog"] = click(&["Hosting.LogRunning", "Hosting.Area.Back"], None);
    p["Hosting.CloseLog"]["next"] = json!(["Hosting.EntryManaged"]);
    for node in p.as_object_mut().expect("固定托管定义有效").values_mut() {
        for (key, value) in [
            ("recognition", json!("DirectHit")),
            ("action", json!("DoNothing")),
            ("pre_delay", json!(0)),
            ("post_delay", json!(100)),
            ("rate_limit", json!(50)),
            ("timeout", json!(10_000)),
            ("next", json!([])),
            ("on_error", json!([])),
            ("focus", json!({})),
        ] {
            node.as_object_mut().unwrap().entry(key).or_insert(value);
        }
    }
    p
}
fn and(names: &[String]) -> Value {
    json!({"recognition":"And","all_of":names})
}
fn click(names: &[&str], end: Option<&str>) -> Value {
    let mut node = json!({"recognition":"And","all_of":names,"box_index":names.len()-1,"action":"LongPress","duration":100});
    if let Some(end) = end {
        node["next"] = json!([format!("Hosting.{end}")]);
    }
    node
}
