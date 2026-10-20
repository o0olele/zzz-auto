use serde_json::{Value, json};

pub fn pipeline() -> Value {
    let mut p = crate::team_check::pipeline();
    for name in [
        "World",
        "OpenMap",
        "MapUp",
        "MapOpened",
        "SelectArea",
        "AreaSelected",
        "SelectPoint",
        "PointSelected",
        "ConfirmTransport",
        "MapPrompt",
        "MapConfirm",
        "Transported",
        "WaitArrival",
        "Arrived",
        "AreaText",
        "PointText",
    ] {
        p[format!("Intel.{name}")] = serde_json::from_str(
            &p[format!("Shop.{name}")]
                .to_string()
                .replace("Shop.", "Intel."),
        )
        .unwrap();
    }
    p["Intel.PointText"]["expected"] = json!(["^房间$"]);
    p["Intel.WaitArrival"]["next"] = json!(["Intel.PointSelected", "Intel.Arrived"]);
    for (name, roi, text) in [
        ("GuideEntry", [0, 0, 1920, 1000], "^情报板$"),
        ("Cycle", [0, 880, 1000, 200], ".*周期内可获取.*"),
        ("Exchange", [954, 991, 288, 73], "^点数[兑兌][换換]$"),
        ("ResetText", [1422, 998, 282, 60], "^重置$"),
        ("HuntFilter", [1200, 100, 720, 860], "^恶名狩猎$"),
        ("ExpertFilter", [1200, 100, 720, 860], "^专业挑战室$"),
        ("Kind", [0, 80, 1920, 840], "^ZZZ_AUTO_UNSET_COMMISSION$"),
        ("AcceptText", [0, 100, 1920, 980], "^接取委托$"),
        ("GoText", [0, 100, 1920, 980], "^前往$"),
        ("ActiveText", [0, 0, 1920, 1080], "^委托代行中$"),
        ("NextText", [0, 500, 1920, 580], "^下一步$"),
        ("FreeMode", [0, 100, 1920, 980], "^无报酬模式$"),
        ("FailureText", [300, 200, 1320, 600], ".*接取失败.*"),
        ("ConfirmText", [600, 500, 720, 400], "^确认$|^确定$"),
        (
            "NoAgentsText",
            [300, 200, 1320, 600],
            ".*至少选择1位代理人出战.*",
        ),
        ("TeamText", [368, 1002, 195, 55], "^预备编队$"),
        ("DeployText", [1638, 1002, 194, 50], "^出战$"),
        ("CompleteText", [1566, 1004, 266, 52], "^完成$"),
        ("RetreatText", [526, 848, 220, 64], "^撤退$"),
        (
            "SettlementPrompt",
            [300, 200, 1320, 700],
            ".*(委托完成|挑战完成|委托代行中|获得奖励|奖励).*",
        ),
    ] {
        p[format!("Intel.{name}")] = ocr(roi, text);
    }
    // SDK 检测可能把“下一步”拆成“下一”和“步”；固定右下按钮补一次整行识别。
    p["Intel.NextTextAny"] = p["Intel.NextText"].clone();
    p["Intel.NextTextLine"] = ocr([1500, 980, 370, 85], "^下一步$");
    p["Intel.NextTextLine"]["only_rec"] = json!(true);
    p["Intel.NextText"] =
        json!({"recognition":"Or","any_of":["Intel.NextTextAny","Intel.NextTextLine"]});
    p["Intel.FunctionIcon"] = json!({"recognition":"TemplateMatch","roi":[1776,28,93,89],"template":"intel-board/function-menu.png","threshold":0.7,"green_mask":true});
    p["Intel.RefreshIcon"] = json!({"recognition":"TemplateMatch","roi":[801,999,59,59],"template":"intel-board/refresh.png","threshold":0.7,"green_mask":true});
    p["Intel.Guide"] = and(&["Intel.FunctionIcon", "Intel.GuideEntry"]);
    p["Intel.Board"] = and(&["Intel.Cycle", "Intel.Exchange"]);
    p["Intel.Filter"] = and(&["Intel.ResetText", "Intel.HuntFilter", "Intel.ExpertFilter"]);
    p["Intel.Inspect"] = next(&[
        "Intel.Filter",
        "Intel.Board",
        "Intel.Guide",
        "Intel.PointSelected",
        "Intel.AreaSelected",
        "Intel.MapOpened",
        "Intel.World",
    ]);
    p["Intel.OpenGuide"] = and(&["Intel.World"]);
    p["Intel.OpenGuide"]["action"] = json!("KeyDown");
    p["Intel.OpenGuide"]["key"] = json!(18);
    p["Intel.OpenGuide"]["auto_up"] = json!(true);
    p["Intel.OpenGuide"]["next"] = json!(["Intel.ClickGuide"]);
    p["Intel.ClickGuide"] = click(&["Intel.World", "Intel.FunctionIcon"], "Intel.GuideUp");
    p["Intel.GuideUp"] = json!({"action":"KeyUp","key":18,"next":["Intel.Guide"]});
    p["Intel.OpenBoard"] = click(&["Intel.Guide", "Intel.GuideEntry"], "Intel.Board");
    p["Intel.OpenFilter"] = click(&["Intel.Board"], "Intel.Filter");
    p["Intel.OpenFilter"]["target"] = json!([878, 999, 58, 59]);
    for (stage, text, end) in [
        ("ResetFilter", "ResetText", "FilterReset"),
        ("SelectNotorious", "HuntFilter", "NotoriousSelected"),
        ("SelectExpert", "ExpertFilter", "ExpertSelected"),
    ] {
        p[format!("Intel.{stage}")] = click(
            &["Intel.Filter", &format!("Intel.{text}")],
            &format!("Intel.{end}"),
        );
        p[format!("Intel.{end}")] = and(&["Intel.Filter"]);
    }
    p["Intel.CloseFilter"] = click(&["Intel.Filter"], "Intel.FilterClosed");
    p["Intel.CloseFilter"]["target"] = json!([1800, 40]);
    p["Intel.FilterClosed"] = and(&["Intel.Board"]);
    p["Intel.Refresh"] = click(&["Intel.Board", "Intel.RefreshIcon"], "Intel.Refreshed");
    p["Intel.Refresh"]["post_delay"] = json!(1000);
    p["Intel.Refreshed"] = and(&["Intel.Board"]);
    p["Intel.Scroll"] = and(&["Intel.Board"]);
    p["Intel.Scroll"]["action"] = json!("Swipe");
    p["Intel.Scroll"]["begin"] = json!([1560, 800]);
    p["Intel.Scroll"]["end"] = json!([1560, 300]);
    p["Intel.Scroll"]["duration"] = json!(500);
    p["Intel.Scroll"]["post_delay"] = json!(1000);
    p["Intel.Scroll"]["next"] = json!(["Intel.Scrolled"]);
    p["Intel.Scrolled"] = and(&["Intel.Board"]);
    p["Intel.PreDeployment"] = and(&["Intel.TeamText", "Intel.DeployText"]);
    p["Intel.CommissionOpened"] = and(&["Intel.Kind", "Intel.ActiveText"]);
    p["Intel.NextPage"] = and(&["Intel.Kind", "Intel.NextText"]);
    p["Intel.AcceptFailed"] = and(&["Intel.FailureText", "Intel.ConfirmText"]);
    p["Intel.NoAgents"] = and(&["Intel.NoAgentsText", "Intel.ConfirmText"]);
    p["Intel.DetailButtons"] = json!({"recognition":"Or","any_of":["Intel.AcceptText","Intel.GoText","Intel.ActiveText","Intel.NextText","Intel.FreeMode"]});
    p["Intel.CommissionSelected"] = and(&["Intel.Kind", "Intel.DetailButtons"]);
    p["Intel.OpenCommission"] = next(&[
        "Intel.PreDeployment",
        "Intel.AcceptFailed",
        "Intel.CommissionSelected",
    ]);
    p["Intel.Accept"] = next(&[
        "Intel.AcceptFailed",
        "Intel.PreDeployment",
        "Intel.NextPage",
        "Intel.CommissionOpened",
        "Intel.ClickAccept",
        "Intel.ClickGo",
    ]);
    p["Intel.ClickAccept"] = click(&["Intel.Kind", "Intel.AcceptText"], "Intel.AcceptAdvanced");
    p["Intel.ClickGo"] = click(&["Intel.Kind", "Intel.GoText"], "Intel.AcceptAdvanced");
    p["Intel.Next"] = next(&[
        "Intel.AcceptFailed",
        "Intel.PreDeployment",
        "Intel.ClickNext",
        "Intel.ClickFreeMode",
    ]);
    p["Intel.ClickNext"] = click(&["Intel.Kind", "Intel.NextText"], "Intel.NextAdvanced");
    p["Intel.ClickFreeMode"] = click(&["Intel.Kind", "Intel.FreeMode"], "Intel.NextAdvanced");
    p["Intel.ConfirmFailed"] = click(
        &["Intel.AcceptFailed", "Intel.ConfirmText"],
        "Intel.FailureDismissed",
    );
    p["Intel.FailureDismissed"] = and(&["Intel.Board"]);
    p["Intel.OpenTeams"] = next(&["Intel.SelectionOpened", "Intel.ClickTeams"]);
    p["Intel.SelectionOpened"] = and(&["Teams.Selection"]);
    p["Intel.ClickTeams"] = click(
        &["Intel.PreDeployment", "Intel.TeamText"],
        "Intel.SelectionOpened",
    );
    p["Intel.Deploy"] = next(&["Intel.NoAgents", "Intel.ClickDeploy"]);
    p["Intel.ClickDeploy"] = click(
        &["Intel.PreDeployment", "Intel.DeployText"],
        "Intel.DeployClicked",
    );
    p["Intel.WaitDeploy"] = next(&[
        "Intel.NoAgents",
        "Intel.ConfirmAgent",
        "Intel.PreDeployment",
    ]);
    p["Intel.ConfirmAgent"] = click(
        &["Intel.ActiveText", "Intel.ConfirmText"],
        "Intel.AgentConfirmed",
    );
    p["Intel.ConfirmNoAgents"] = click(
        &["Intel.NoAgents", "Intel.ConfirmText"],
        "Intel.NoAgentsDismissed",
    );
    p["Intel.NoAgentsDismissed"] = and(&["Intel.PreDeployment"]);
    // 结算入口仅由已保存的本次战斗结果授权；泛用确认还必须有奖励/委托提示。
    p["Intel.Settlement"] = next(&[
        "Intel.Board",
        "Intel.ClickComplete",
        "Intel.ClickRetreat",
        "Intel.SettlementNext",
        "Intel.SettlementConfirm",
    ]);
    p["Intel.ClickComplete"] = click(&["Intel.CompleteText"], "Intel.SettlementAdvanced");
    p["Intel.ClickRetreat"] = click(&["Intel.RetreatText"], "Intel.SettlementAdvanced");
    p["Intel.SettlementNext"] = click(
        &["Intel.SettlementPrompt", "Intel.NextText"],
        "Intel.SettlementAdvanced",
    );
    p["Intel.SettlementConfirm"] = click(
        &["Intel.SettlementPrompt", "Intel.ConfirmText"],
        "Intel.SettlementAdvanced",
    );
    for name in [
        "AcceptAdvanced",
        "NextAdvanced",
        "DeployClicked",
        "AgentConfirmed",
        "SettlementAdvanced",
    ] {
        p[format!("Intel.{name}")] = json!({});
    }
    // 功能导览展开时大世界 HUD 仍可见，先关闭已知覆盖页再确认返回。
    p["Intel.Return"] = next(&["Intel.CloseKnown", "Intel.World"]);
    p["Intel.Known"] =
        json!({"recognition":"Or","any_of":["Intel.Board","Intel.Guide","Intel.Filter"]});
    p["Intel.CloseKnown"] = and(&["Intel.Known"]);
    p["Intel.CloseKnown"]["action"] = json!("KeyDown");
    p["Intel.CloseKnown"]["key"] = json!(27);
    p["Intel.CloseKnown"]["auto_up"] = json!(true);
    p["Intel.CloseKnown"]["next"] = json!(["Intel.EscapeUp"]);
    p["Intel.EscapeUp"] = json!({"action":"KeyUp","key":27,"next":["Intel.Return"]});
    for (name, node) in p.as_object_mut().unwrap() {
        if !name.starts_with("Intel.") {
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
