//! 后续领取应用；每次动作之后重新识别，不以点击返回值代替业务后态。
use crate::basic_daily::DailyProgress;
use serde_json::{Value, json};

pub fn observe(p: &mut DailyProgress, node: &str) -> Result<(), &'static str> {
    let stage = p.stage.as_str();
    let app = p.app_id.as_str();
    let route_app = matches!(app, "trigrams_collection" | "hou_hou_bakery");
    let next = match (stage, node) {
        ("Return", "World") => {
            p.returned_world = true;
            "Done"
        }
        ("Inspect", "World") if route_app && p.reward_confirmed && app == "trigrams_collection" => {
            p.completion_proven = true;
            p.returned_world = true;
            "Done"
        }
        ("Inspect", "World") if route_app && !p.route_confirmed && !p.attempted => "OpenMap",
        ("Inspect", "World") if route_app && !p.attempted => {
            if p.approach_attempted {
                "WaitInteraction"
            } else {
                "Approach"
            }
        }
        ("Inspect" | "OpenMap", "MapOpened") if route_app => "SelectArea",
        ("Inspect" | "SelectArea", "AreaSelected") if route_app => "SelectPoint",
        ("Inspect" | "SelectPoint", "PointSelected") if route_app && !p.transport_attempted => {
            "ConfirmTransport"
        }
        ("ConfirmTransport", "Transported") if route_app => "WaitArrival",
        ("WaitArrival", "Arrived") if route_app => {
            p.route_confirmed = true;
            "Approach"
        }
        ("Approach", "Interacted") if route_app => "WaitInteraction",
        ("Inspect" | "WaitInteraction" | "ConfirmReward", "UsedDialog") if route_app => {
            p.completion_proven = true;
            "ConfirmUsed"
        }
        ("ConfirmUsed", "Dismissed") if route_app => "Return",
        ("Inspect" | "WaitInteraction", "Used") if app == "hou_hou_bakery" => {
            p.completion_proven = true;
            "Return"
        }
        ("Inspect" | "WaitInteraction", "Main")
            if app == "trigrams_collection" && p.reward_confirmed =>
        {
            p.completion_proven = true;
            "Return"
        }
        ("Inspect" | "WaitInteraction", "Main") if app == "trigrams_collection" => "SelectGift",
        ("Inspect" | "WaitInteraction", "Machine") if app == "hou_hou_bakery" => "SelectGift",
        ("SelectGift", "Selected") if route_app => "Inspect",
        ("Inspect" | "WaitInteraction", "SlideVisible") if app == "trigrams_collection" => {
            p.attempted = true;
            p.reveal_observed = true;
            "Slide"
        }
        ("Slide", "Slid") if app == "trigrams_collection" => "Inspect",
        ("Inspect" | "WaitInteraction", "Box") if app == "hou_hou_bakery" => {
            p.attempted = true;
            p.reveal_observed = true;
            "OpenBox"
        }
        ("OpenBox", "Opened") if app == "hou_hou_bakery" => "Inspect",
        ("Inspect" | "WaitInteraction" | "Slide", "Reward")
            if route_app && p.attempted && p.reveal_observed =>
        {
            "ConfirmReward"
        }
        ("ConfirmReward", "Dismissed") if route_app => {
            p.reward_confirmed = true;
            "Inspect"
        }
        ("Inspect", "World") if app == "ridu_weekly" => "OpenCompendium",
        ("Inspect", "Menu") if app == "ridu_weekly" => "CloseMenu",
        ("Inspect", "Compendium") if app == "ridu_weekly" => "SelectDaily",
        ("Inspect", "Daily") if app == "ridu_weekly" => "OpenWeekly",
        ("OpenCompendium" | "CloseMenu" | "SelectDaily" | "OpenWeekly", "Navigated")
            if app == "ridu_weekly" =>
        {
            "Inspect"
        }
        ("Inspect" | "ClaimReward", "Score") if app == "ridu_weekly" => "ClaimScore",
        ("ClaimScore", "Scored") if app == "ridu_weekly" => "Inspect",
        ("Inspect", "Ready") if app == "ridu_weekly" && !p.reward_confirmed => "ClaimReward",
        ("ClaimReward", "RewardClicked") if app == "ridu_weekly" => {
            p.reward_confirmed = true;
            "Inspect"
        }
        ("Inspect", "Dialog") if app == "ridu_weekly" && (p.attempted || p.reward_checked) => {
            "ConfirmDialog"
        }
        ("ConfirmDialog", "Dismissed") if app == "ridu_weekly" => "Inspect",
        ("Inspect", "Ready") if app == "ridu_weekly" && p.reward_confirmed => {
            p.completion_proven = true;
            "Return"
        }
        _ => return Err("日常流程未到达当前检查点允许的业务节点"),
    };
    p.stage = next.into();
    Ok(())
}

pub fn pipeline() -> Value {
    let mut p = crate::basic_daily::legacy_pipeline();
    // 复用已固定的日常导航，但保留首批包原定义以兼容旧资源。
    clone_nodes(&mut p, "Engagement.", "Weekly.");
    p["Weekly.Entry"] = ocr([414, 840, 182, 60], &["^丽都周纪$"]);
    p["Weekly.Title"] = ocr([1450, 130, 400, 100], &["^积分特赏$"]);
    p["Weekly.Bingo"] = ocr([160, 140, 420, 90], &[".*三格连线额外奖励.*"]);
    p["Weekly.Page"] = and(&["Weekly.Title", "Weekly.Bingo"]);
    p["Weekly.White"] =
        json!({"recognition":"ColorMatch","method":4,"lower":[250,250,250],"upper":[255,255,255]});
    for (index, roi) in [
        (1, [120, 410, 930, 60]),
        (2, [120, 640, 930, 50]),
        (3, [120, 870, 935, 55]),
    ] {
        let text = format!("Weekly.ScoreText{index}");
        p[&text] = ocr(roi, &["^100$"]);
        p[&text]["color_filter"] = json!("Weekly.White");
        p[format!("Weekly.ScoreRow{index}")] = click(&["Weekly.Page", &text], None);
        p[format!("Weekly.ScoreRow{index}")]["action"] = json!("DoNothing");
    }
    p["Weekly.Score"] = json!({"recognition":"Or","any_of":["Weekly.ScoreRow1","Weekly.ScoreRow2","Weekly.ScoreRow3"]});
    // 优先级分支检查无积分；And 内联对象不是节点引用，不能用伪引用取反。
    p["Weekly.Ready"] = and(&["Weekly.Page"]);
    p["Weekly.Inspect"] = next(&[
        "Weekly.Dialog",
        "Weekly.Score",
        "Weekly.Ready",
        "Weekly.Daily",
        "Weekly.Compendium",
        "Weekly.Menu",
        "Weekly.World",
    ]);
    p["Weekly.OpenWeekly"] = click(&["Weekly.Daily", "Weekly.Entry"], Some("Weekly.Navigated"));
    p["Weekly.ClaimScore"] = click(&["Weekly.Score"], Some("Weekly.Scored"));
    p["Weekly.Scored"] = json!({});
    p["Weekly.ClaimReward"] = next(&["Weekly.Score", "Weekly.ClickReward"]);
    p["Weekly.ClickReward"] = click(&["Weekly.Ready"], Some("Weekly.RewardClicked"));
    // 原 screen_info 此处为一个点；Maa 目标允许点，不伪造矩形。
    p["Weekly.ClickReward"]["target"] = json!([1558, 314]);
    p["Weekly.RewardClicked"] = json!({});
    p["Weekly.ConfirmDialog"] = click(&["Weekly.Dialog"], Some("Weekly.Dismissed"));
    add_return(
        &mut p,
        "Weekly",
        &["Weekly.Page", "Weekly.Compendium", "Base.Menu"],
    );

    for (prefix, area, point) in [
        ("Trigram", "澄辉坪", "阿朔"),
        ("Bakery", "布亚斯特城区", "吼吼饼铺"),
    ] {
        clone_nodes(&mut p, "Scratch.", &format!("{prefix}."));
        p[format!("{prefix}.AreaText")] = ocr([0, 90, 1920, 840], &[&format!("^{area}$")]);
        p[format!("{prefix}.PointText")] = ocr([70, 1000, 1778, 58], &[&format!("^{point}$")]);
        // 传送后无需前移；保留有界等待后交互。
        p[format!("{prefix}.Approach")] = and(&["Base.World"]);
        p[format!("{prefix}.Approach")]["next"] = json!([format!("{prefix}.InteractWait")]);
        p[format!("{prefix}.UsedText")] = ocr(
            [400, 300, 1200, 400],
            &[".*今日.*已经.*相同类型.*奖励领取.*"],
        );
        p[format!("{prefix}.ConfirmText")] = ocr([700, 550, 520, 450], &["^确认$", "^确定$"]);
        p[format!("{prefix}.UsedDialog")] = click(
            &[
                &format!("{prefix}.UsedText"),
                &format!("{prefix}.ConfirmText"),
            ],
            None,
        );
        p[format!("{prefix}.UsedDialog")]["action"] = json!("DoNothing");
        p[format!("{prefix}.ConfirmUsed")] = click(
            &[&format!("{prefix}.UsedDialog")],
            Some(&format!("{prefix}.Dismissed")),
        );
        p[format!("{prefix}.ClickReward")] = click(
            &[&format!("{prefix}.Reward")],
            Some(&format!("{prefix}.Dismissed")),
        );
        p[format!("{prefix}.ConfirmReward")] = next(&[
            &format!("{prefix}.UsedDialog"),
            &format!("{prefix}.PointSelected"),
            &format!("{prefix}.ClickReward"),
        ]);
        p[format!("{prefix}.Selected")] = json!({});
    }
    p["Trigram.Title"] = ocr([1250, 780, 480, 230], &["^卦象集录$", "^卦象集錄$"]);
    p["Trigram.Main"] = and(&["Base.MenuBack", "Trigram.Title"]);
    p["Trigram.SlideText"] = ocr([0, 540, 1920, 540], &[".*滑动屏幕以获取卦象.*"]);
    p["Trigram.Sliding"] = and(&["Trigram.SlideText"]);
    p["Trigram.SelectGift"] = click(&["Trigram.Main"], Some("Trigram.Selected"));
    p["Trigram.SelectGift"]["target"] = json!([1318, 298, 332, 342]);
    p["Trigram.Reward"] = and(&["Trigram.ConfirmText"]);
    // 普通确认只能由 Rust 在本次滑动已开始的上下文内投递。
    p["Trigram.Slide"] = and(&["Trigram.Sliding"]);
    p["Trigram.Slide"]["action"] = json!("Swipe");
    p["Trigram.Slide"]["begin"] = json!([1820, 100]);
    p["Trigram.Slide"]["end"] = json!([100, 980]);
    p["Trigram.Slide"]["duration"] = json!(1000);
    p["Trigram.Slide"]["next"] = json!(["Trigram.Reward", "Trigram.SlideBack"]);
    p["Trigram.SlideBack"] = and(&["Trigram.Sliding"]);
    for (key, value) in [
        ("action", json!("Swipe")),
        ("begin", json!([100, 980])),
        ("end", json!([1820, 100])),
        ("duration", json!(1000)),
        ("next", json!(["Trigram.Slid"])),
    ] {
        p["Trigram.SlideBack"][key] = value;
    }
    p["Trigram.Slid"] = json!({});
    // 识别节点与输入节点必须分名，Inspect 不能直接进入 Swipe。
    p["Trigram.SlideVisible"] = and(&["Trigram.Sliding"]);
    p["Trigram.Inspect"] = next(&[
        "Trigram.UsedDialog",
        "Trigram.SlideVisible",
        "Trigram.PointSelected",
        "Trigram.Reward",
        "Trigram.Main",
        "Trigram.AreaSelected",
        "Trigram.MapOpened",
        "Trigram.World",
    ]);
    p["Trigram.WaitInteraction"] = next(&[
        "Trigram.UsedDialog",
        "Trigram.SlideVisible",
        "Trigram.Reward",
        "Trigram.Main",
    ]);
    add_return(&mut p, "Trigram", &["Trigram.Main"]);

    p["Bakery.Used"] = ocr([0, 540, 1920, 540], &[".*今日已领取同类型奖励.*"]);
    p["Bakery.MachineText"] = ocr([0, 540, 1920, 540], &[".*每日可领取一次.*"]);
    p["Bakery.Machine"] = and(&["Bakery.MachineText"]);
    p["Bakery.Box"] = ocr([0, 540, 1920, 540], &[".*查看今天.*"]);
    p["Bakery.SelectGift"] = click(&["Bakery.Machine"], Some("Bakery.Selected"));
    p["Bakery.SelectGift"]["target"] = json!([1100, 430, 160, 140]);
    p["Bakery.OpenBox"] = click(&["Bakery.Box"], Some("Bakery.Opened"));
    p["Bakery.OpenBox"]["target"] = json!([960, 540]);
    p["Bakery.Opened"] = json!({});
    p["Bakery.Reward"] = and(&["Bakery.ConfirmText"]);
    p["Bakery.Inspect"] = next(&[
        "Bakery.UsedDialog",
        "Bakery.Used",
        "Bakery.Box",
        "Bakery.PointSelected",
        "Bakery.Reward",
        "Bakery.Machine",
        "Bakery.AreaSelected",
        "Bakery.MapOpened",
        "Bakery.World",
    ]);
    p["Bakery.WaitInteraction"] = next(&[
        "Bakery.UsedDialog",
        "Bakery.Used",
        "Bakery.Box",
        "Bakery.Reward",
        "Bakery.Machine",
    ]);
    add_return(&mut p, "Bakery", &["Bakery.Machine", "Bakery.Used"]);
    normalize(&mut p);
    p
}

fn clone_nodes(p: &mut Value, from: &str, to: &str) {
    let nodes: Vec<_> = p
        .as_object()
        .unwrap()
        .iter()
        .filter(|(key, _)| key.starts_with(from))
        .map(|(key, node)| {
            (
                key.replacen(from, to, 1),
                serde_json::from_str::<Value>(&node.to_string().replace(from, to)).unwrap(),
            )
        })
        .collect();
    for (key, node) in nodes {
        p[key] = node;
    }
}
fn and(nodes: &[&str]) -> Value {
    json!({"recognition":"And","all_of":nodes})
}
fn next(nodes: &[&str]) -> Value {
    json!({"next":nodes})
}
fn ocr(roi: [i32; 4], text: &[&str]) -> Value {
    json!({"recognition":"OCR","model":"ppocrv5","roi":roi,"expected":text,"threshold":0.6})
}
fn click(nodes: &[&str], next: Option<&str>) -> Value {
    let mut node = and(nodes);
    node["box_index"] = json!(nodes.len() - 1);
    node["action"] = json!("LongPress");
    node["duration"] = json!(100);
    if let Some(next) = next {
        node["next"] = json!([next]);
    }
    node
}
fn add_return(p: &mut Value, prefix: &str, known: &[&str]) {
    p[format!("{prefix}.Known")] = json!({"recognition":"Or","any_of":known});
    p[format!("{prefix}.Return")] =
        next(&[&format!("{prefix}.World"), &format!("{prefix}.CloseKnown")]);
    p[format!("{prefix}.CloseKnown")] = and(&[&format!("{prefix}.Known")]);
    p[format!("{prefix}.CloseKnown")]["action"] = json!("KeyDown");
    p[format!("{prefix}.CloseKnown")]["key"] = json!(27);
    p[format!("{prefix}.CloseKnown")]["auto_up"] = json!(true);
    p[format!("{prefix}.CloseKnown")]["next"] = json!([format!("{prefix}.EscapeUp")]);
    p[format!("{prefix}.EscapeUp")] = json!({"action":"KeyUp","key":27,"next":[format!("{prefix}.World"),format!("{prefix}.CloseKnown")]});
}
fn normalize(p: &mut Value) {
    for node in p.as_object_mut().unwrap().values_mut() {
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
}
