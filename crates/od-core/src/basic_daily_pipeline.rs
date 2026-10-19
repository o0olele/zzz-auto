use serde_json::{Value, json};

/// 固定 1080p 节点，领取步骤不含自动回到领取入口的边。
pub fn pipeline() -> Value {
    let mut p = json!({});
    for (name, roi, template, mask) in [
        (
            "Base.WorldFunction",
            [1776, 28, 93, 89],
            "world_function",
            true,
        ),
        (
            "Base.WorldPreset",
            [1225, 44, 280, 58],
            "world_preset",
            true,
        ),
        ("Base.MenuBack", [82, 13, 68, 77], "back", true),
        (
            "Base.MenuWallpaper",
            [80, 776, 102, 65],
            "menu_wallpaper",
            true,
        ),
        (
            "Base.CompendiumIcon",
            [1440, 28, 249, 90],
            "compendium",
            false,
        ),
        ("Base.Close", [120, 80, 1680, 920], "close", false),
        ("Engagement.Marker", [1532, 250, 70, 100], "completed", true),
    ] {
        p[name] = json!({"recognition":"TemplateMatch","roi":roi,"template":[format!("daily/{template}.png")],
            "threshold":0.7,"method":5,"green_mask":mask});
    }
    p["Base.World"] = and(&["Base.WorldFunction", "Base.WorldPreset"]);
    p["Base.Menu"] = and(&["Base.MenuBack", "Base.MenuWallpaper"]);
    p["Base.RewardTitle"] = ocr([700, 300, 520, 180], &["^获得$", "^获得奖励$"]);
    p["Base.RewardConfirm"] = ocr([700, 600, 520, 260], &["^确认$", "^确定$"]);
    p["Base.Dialog"] = and(&["Base.RewardTitle", "Base.RewardConfirm"]);
    p["Base.Dialog"]["box_index"] = json!(1);

    // 日常页签必须有黄色选中态，泛用确认文本不能独立代表领取结果。
    let mut selected = crate::engagement_reward::pipeline();
    for (name, node) in selected.as_object_mut().unwrap().iter_mut() {
        let name = name.replace("EngagementReward.ReadOnly.", "Engagement.");
        let name = if name == "Engagement.Confirm" {
            "Engagement.ConfirmText".to_owned()
        } else {
            name
        };
        if node.get("template").is_some() {
            continue;
        }
        if node.get("color_filter").is_some() {
            node["color_filter"] = json!("Engagement.DailyTabColor");
        }
        p[&name] = node.clone();
    }
    p["Engagement.Daily"] = and(&["Engagement.DailyTab", "Engagement.DailyTitle"]);
    p["Engagement.Completed"] = and(&["Engagement.Daily", "Engagement.Marker"]);
    p["Engagement.Dialog"] = and(&["Base.RewardTitle", "Engagement.ConfirmText"]);
    p["Engagement.Dialog"]["box_index"] = json!(1);
    p["Engagement.Compendium"] = ocr(
        [708, 140, 928, 80],
        &["^目标$", "^日常$", "^训练$", "^作战$", "^战术$"],
    );
    p["Engagement.World"] = and(&["Base.World"]);
    p["Engagement.Menu"] = and(&["Base.Menu"]);
    p["Engagement.Inspect"] = next(&[
        "Engagement.Dialog",
        "Engagement.Preview",
        "Engagement.Completed",
        "Engagement.Daily",
        "Engagement.Compendium",
        "Engagement.Menu",
        "Engagement.World",
    ]);
    p["Engagement.OpenCompendium"] = next(&["Engagement.HoldAlt"]);
    p["Engagement.HoldAlt"] = key("Base.World", "KeyDown", 18, "Engagement.ClickCompendium");
    p["Engagement.ClickCompendium"] = click(
        &["Base.World", "Base.CompendiumIcon"],
        Some("Engagement.ReleaseAlt"),
    );
    p["Engagement.ReleaseAlt"] = json!({"action":"KeyUp","key":18,"next":["Engagement.Navigated"]});
    p["Engagement.Navigated"] = json!({});
    p["Engagement.SelectDaily"] = ocr([708, 140, 928, 80], &["^日常$"]);
    p["Engagement.SelectDaily"]["action"] = json!("LongPress");
    p["Engagement.SelectDaily"]["duration"] = json!(100);
    p["Engagement.SelectDaily"]["next"] = json!(["Engagement.Navigated"]);
    p["Engagement.CloseMenu"] = click(
        &["Base.Menu", "Base.MenuBack"],
        Some("Engagement.Navigated"),
    );
    p["Engagement.Claim"] = next(&["Engagement.ClickClaim"]);
    p["Engagement.ClickClaim"] = click(
        &["Engagement.Daily", "Engagement.DailyTitle"],
        Some("Engagement.Claimed"),
    );
    p["Engagement.Claimed"] = json!({});
    p["Engagement.AfterClaim"] = next(&[
        "Engagement.Dialog",
        "Engagement.Preview",
        "Engagement.Completed",
        "Engagement.Daily",
    ]);
    p["Engagement.AfterClaim"]["pre_delay"] = json!(750);
    p["Engagement.Confirm"] = click(&["Engagement.Dialog"], Some("Engagement.Dismissed"));
    p["Engagement.ClosePreview"] = click(
        &["Engagement.Preview", "Base.Close"],
        Some("Engagement.Dismissed"),
    );
    p["Engagement.Dismissed"] = json!({});
    p["Engagement.Verify"] = next(&["Engagement.Completed", "Engagement.Daily"]);
    p["Engagement.Verify"]["pre_delay"] = json!(750);

    // 城募的双按钮缺失必须同时有页面和等级选中态作上下文。
    for (name, roi, text) in [
        ("Fund.Tasks", [1436, 26, 152, 50], "^成长任务$"),
        ("Fund.Levels", [1218, 20, 144, 68], "^等级回馈$"),
        ("Fund.TaskClaimText", [1592, 982, 256, 74], "^全部领取$"),
        ("Fund.LevelClaimText", [1240, 982, 256, 94], "^全部领取$"),
        (
            "Fund.ActivationText",
            [1402, 936, 446, 120],
            "^开启丽都城募$",
        ),
        ("Fund.ActivationConfirmText", [874, 600, 200, 260], "^确认$"),
        ("Fund.MenuText", [200, 924, 1528, 134], "^丽都城募$"),
    ] {
        p[name] = ocr(roi, &[text]);
    }
    p["Fund.Page"] = and(&["Fund.Tasks", "Fund.Levels"]);
    p["Fund.Activate"] = and(&["Fund.ActivationText"]);
    // 开通确认只在本次明确点击开启后接受，由 Rust 检查点限制。
    p["Fund.ActivationConfirm"] = and(&["Fund.ActivationConfirmText"]);
    p["Fund.Dialog"] = and(&["Base.Dialog"]);
    p["Fund.World"] = and(&["Base.World"]);
    p["Fund.Menu"] = and(&["Base.Menu"]);
    p["Fund.Inspect"] = next(&[
        "Fund.Dialog",
        "Fund.Activate",
        "Fund.Page",
        "Fund.ActivationConfirm",
        "Fund.Menu",
        "Fund.World",
    ]);
    p["Fund.OpenMenu"] = next(&["Fund.HoldAlt"]);
    p["Fund.HoldAlt"] = key("Base.World", "KeyDown", 18, "Fund.ClickMenu");
    p["Fund.ClickMenu"] = json!({"action":"LongPress","target":[96,34,68,74],"duration":100,"next":["Fund.ReleaseAlt"]});
    p["Fund.ReleaseAlt"] = json!({"action":"KeyUp","key":18,"next":["Fund.Navigated"]});
    p["Fund.OpenFund"] = click(&["Base.Menu", "Fund.MenuText"], Some("Fund.Navigated"));
    p["Fund.Navigated"] = json!({});
    p["Fund.Activate"] = click(&["Fund.ActivationText"], Some("Fund.Activated"));
    // Inspect 只识别，不在检查阶段投递动作。
    p["Fund.ActivationAvailable"] = and(&["Fund.ActivationText"]);
    p["Fund.Inspect"]["next"] = json!([
        "Fund.Dialog",
        "Fund.ActivationAvailable",
        "Fund.Page",
        "Fund.ActivationConfirm",
        "Fund.Menu",
        "Fund.World"
    ]);
    p["Fund.ConfirmActivation"] = click(&["Fund.ActivationConfirmText"], Some("Fund.Activated"));
    p["Fund.Activated"] = json!({});
    p["Fund.SelectTasks"] = click(&["Fund.Page", "Fund.Tasks"], Some("Fund.TaskSelected"));
    p["Fund.TaskSelected"] = and(&["Fund.TaskView"]);
    p["Fund.TaskView"] = and(&["Fund.Page"]);
    p["Fund.TaskView"]["all_of"] = json!(["Fund.Page", {"recognition":"OCR","roi":[1180,110,690,90],"model":"ppocrv5","expected":["^(今日|本周|本期)任务$"],"threshold":0.6}]);
    p["Fund.TaskClaim"] = and(&["Fund.TaskView", "Fund.TaskClaimText"]);
    p["Fund.TaskClaim"]["box_index"] = json!(1);
    p["Fund.TasksDone"] = and(&["Fund.TaskView"]);
    p["Fund.InspectTasks"] = next(&["Fund.TaskClaim", "Fund.TasksDone"]);
    p["Fund.ClaimTasks"] = click(&["Fund.TaskClaim"], Some("Fund.Claimed"));
    p["Fund.Claimed"] = json!({});
    p["Fund.AfterTasks"] = next(&["Fund.Dialog", "Fund.TasksDone"]);
    p["Fund.ConfirmTasks"] = click(&["Fund.Dialog"], Some("Fund.Dismissed"));
    p["Fund.SelectLevels"] = click(&["Fund.Page", "Fund.Levels"], Some("Fund.LevelSelected"));
    p["Fund.LevelSelected"] = and(&["Fund.Page", "Fund.SelectedLevel"]);
    // 先检查领取与弹窗，再接受本次已切换的等级页上下文。
    p["Fund.LevelYellow"] = json!({"recognition":"ColorMatch","roi":[1218,20,144,68],"method":4,"lower":[120,160,0],"upper":[255,255,100],"count":500,"connected":false});
    p["Fund.SelectedLevel"] = and(&["Fund.Levels", "Fund.LevelYellow"]);
    p["Fund.LevelClaim"] = and(&["Fund.Page", "Fund.LevelClaimText"]);
    p["Fund.LevelClaim"]["box_index"] = json!(1);
    p["Fund.LevelsDone"] = and(&["Fund.Page", "Fund.SelectedLevel"]);
    p["Fund.InspectLevels"] = next(&["Fund.Dialog", "Fund.LevelClaim", "Fund.LevelsDone"]);
    p["Fund.ClaimLevels"] = click(&["Fund.LevelClaim"], Some("Fund.Claimed"));
    p["Fund.ConfirmLevels"] = click(&["Fund.Dialog"], Some("Fund.Dismissed"));
    p["Fund.Dismissed"] = json!({});

    // 报刊亭路线：复用已转换地图的固定区域，仅替换明确登记的目标文字。
    for (name, roi, texts) in [
        ("Scratch.CardText", [94, 956, 78, 94], vec!["^刮刮卡$"]),
        (
            "Scratch.Card",
            [704, 656, 428, 36],
            vec!["^每日可刮取一次$"],
        ),
        (
            "Scratch.Wake",
            [1434, 638, 186, 56],
            vec!["^叫醒他$", "^叫醒嗷呜$"],
        ),
        (
            "Scratch.Awake",
            [780, 518, 358, 68],
            vec!["^嗷呜被你叫醒了$"],
        ),
        (
            "Scratch.Dialogue",
            [1438, 560, 424, 216],
            vec!["^只是来看刮刮卡和报纸$"],
        ),
        ("Scratch.Npc", [890, 792, 152, 36], vec!["^嗷呜$"]),
        (
            "Scratch.SharedPrompt",
            [450, 300, 1020, 300],
            vec![
                ".*(已|已经).*(刮刮卡|签到|领取).*",
                ".*(卦象集录|吼吼饼铺).*",
            ],
        ),
        ("Scratch.UsedConfirm", [882, 602, 200, 52], vec!["^确认$"]),
        ("Scratch.AreaText", [0, 90, 1920, 840], vec!["^六分街$"]),
        ("Scratch.PointText", [70, 1000, 1778, 58], vec!["^报刊亭$"]),
        (
            "Scratch.MapPrompt",
            [700, 480, 520, 90],
            vec!["^是否传送至该地点[？?]$"],
        ),
        ("Scratch.MapConfirm", [1024, 600, 208, 108], vec!["^确认$"]),
    ] {
        p[name] = ocr(roi, &texts);
    }
    p["Scratch.World"] = and(&["Base.World"]);
    p["Scratch.Stand"] = and(&["Base.MenuBack", "Scratch.CardText"]);
    p["Scratch.CardButton"] = and(&["Base.MenuBack", "Scratch.CardText"]);
    p["Scratch.CardButton"]["box_index"] = json!(1);
    p["Scratch.Used"] = and(&["Scratch.SharedPrompt", "Scratch.UsedConfirm"]);
    p["Scratch.Used"]["box_index"] = json!(1);
    p["Scratch.Result"] = and(&["Base.Dialog"]);
    p["Scratch.Inspect"] = next(&[
        "Scratch.Used",
        "Scratch.Card",
        "Scratch.CardButton",
        "Scratch.Wake",
        "Scratch.Dialogue",
        "Scratch.Awake",
        "Scratch.Result",
        "Scratch.Npc",
        "Scratch.PointSelected",
        "Scratch.AreaSelected",
        "Scratch.MapOpened",
        "Scratch.World",
    ]);
    p["Scratch.WaitInteraction"] = next(&[
        "Scratch.Used",
        "Scratch.Card",
        "Scratch.CardButton",
        "Scratch.Wake",
        "Scratch.Dialogue",
        "Scratch.Awake",
        "Scratch.Npc",
    ]);
    p["Scratch.OpenMap"] = key("Base.World", "KeyDown", 78, "Scratch.MapUp");
    p["Scratch.MapUp"] = json!({"action":"KeyUp","key":78,"next":["Scratch.MapOpened"]});
    p["Scratch.MapOpened"] = and(&["Base.MenuBack", "Scratch.AreaText"]);
    p["Scratch.SelectArea"] = click(
        &["Base.MenuBack", "Scratch.AreaText"],
        Some("Scratch.AreaSelected"),
    );
    p["Scratch.AreaSelected"] = and(&["Base.MenuBack", "Scratch.PointText"]);
    p["Scratch.SelectPoint"] = click(
        &["Base.MenuBack", "Scratch.PointText"],
        Some("Scratch.PointSelected"),
    );
    p["Scratch.PointSelected"] = and(&["Scratch.MapPrompt", "Scratch.MapConfirm"]);
    p["Scratch.ConfirmTransport"] = click(
        &["Scratch.MapPrompt", "Scratch.MapConfirm"],
        Some("Scratch.Transported"),
    );
    p["Scratch.Transported"] = json!({});
    p["Scratch.WaitArrival"] = next(&["Scratch.Arrived"]);
    p["Scratch.Arrived"] = and(&["Base.World"]);
    p["Scratch.Approach"] = key("Base.World", "KeyDown", 87, "Scratch.WalkWait");
    p["Scratch.WalkWait"] = json!({"repeat":40,"repeat_delay":25,"next":["Scratch.WalkUp"]});
    p["Scratch.WalkUp"] = json!({"action":"KeyUp","key":87,"next":["Scratch.InteractWait"]});
    p["Scratch.InteractWait"] = json!({"repeat":40,"repeat_delay":25,"next":["Scratch.Interact"]});
    p["Scratch.Interact"] =
        json!({"action":"KeyDown","key":70,"auto_up":true,"next":["Scratch.InteractUp"]});
    p["Scratch.InteractUp"] = json!({"action":"KeyUp","key":70,"next":["Scratch.Interacted"]});
    p["Scratch.Interacted"] = json!({});
    p["Scratch.DialogueStep"] = next(&[
        "Scratch.ClickWake",
        "Scratch.ClickOption",
        "Scratch.ClickAwake",
        "Scratch.ClickNpc",
    ]);
    p["Scratch.ClickWake"] = click(&["Scratch.Wake"], Some("Scratch.Advanced"));
    p["Scratch.ClickOption"] = click(&["Scratch.Dialogue"], Some("Scratch.Advanced"));
    p["Scratch.ClickAwake"] = click(&["Scratch.Awake"], Some("Scratch.Advanced"));
    p["Scratch.ClickNpc"] = click(&["Scratch.Npc"], Some("Scratch.Advanced"));
    p["Scratch.Advanced"] = json!({});
    p["Scratch.SelectCard"] = click(&["Scratch.CardButton"], Some("Scratch.Selected"));
    p["Scratch.Selected"] = json!({});
    // 每次只刮一层，当前层坐标由冻结的 0..2 索引覆写。
    p["Scratch.ScratchRow"] = and(&["Scratch.Card"]);
    p["Scratch.ScratchRow"]["action"] = json!("Swipe");
    p["Scratch.ScratchRow"]["begin"] = json!([780, 522]);
    p["Scratch.ScratchRow"]["end"] = json!([1134, 556]);
    p["Scratch.ScratchRow"]["duration"] = json!(1500);
    p["Scratch.ScratchRow"]["next"] = json!(["Scratch.Scratched"]);
    p["Scratch.Scratched"] = json!({});
    p["Scratch.AfterScratch"] = next(&["Scratch.Used", "Scratch.Result"]);
    p["Scratch.ConfirmUsed"] = click(&["Scratch.Used"], Some("Scratch.Dismissed"));
    p["Scratch.Dismissed"] = json!({});

    for prefix in ["Engagement", "Fund", "Scratch"] {
        p[format!("{prefix}.Return")] =
            next(&[&format!("{prefix}.World"), &format!("{prefix}.CloseKnown")]);
        // Escape 只在本应用已识别页面上投递，未知画面等待超时。
        let known: &[&str] = match prefix {
            "Engagement" => &["Engagement.Compendium", "Engagement.Preview"],
            "Fund" => &["Fund.Page", "Base.Menu"],
            _ => &[
                "Scratch.Stand",
                "Scratch.Npc",
                "Scratch.Card",
                "Scratch.Result",
            ],
        };
        p[format!("{prefix}.Known")] = json!({"recognition":"Or","any_of":known});
        p[format!("{prefix}.CloseKnown")] = key(
            &format!("{prefix}.Known"),
            "KeyDown",
            27,
            &format!("{prefix}.EscapeUp"),
        );
        p[format!("{prefix}.EscapeUp")] = json!({"action":"KeyUp","key":27,"next":[format!("{prefix}.World"),format!("{prefix}.CloseKnown")]});
    }
    normalize(&mut p);
    p
}
fn and(names: &[&str]) -> Value {
    json!({"recognition":"And","all_of":names})
}
fn next(names: &[&str]) -> Value {
    json!({"next":names})
}
fn ocr(roi: [i32; 4], expected: &[&str]) -> Value {
    json!({"recognition":"OCR","model":"ppocrv5","roi":roi,"expected":expected,"threshold":0.6})
}
fn click(names: &[&str], next: Option<&str>) -> Value {
    let mut node = and(names);
    node["box_index"] = json!(names.len() - 1);
    node["action"] = json!("LongPress");
    node["duration"] = json!(100);
    if let Some(next) = next {
        node["next"] = json!([next]);
    }
    node
}
fn key(context: &str, action: &str, key: i32, next: &str) -> Value {
    let mut node = and(&[context]);
    node["action"] = json!(action);
    node["key"] = json!(key);
    node["auto_up"] = json!(true);
    node["next"] = json!([next]);
    node
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
        if node["action"] == "KeyDown" {
            node["post_delay"] = json!(100);
        }
    }
}
