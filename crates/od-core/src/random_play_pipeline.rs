use super::{RandomPlayProgress, THEMES};
use serde_json::{Value, json};

pub fn pipeline() -> Value {
    pipeline_for_schema(7)
}
pub fn pipeline_for_schema(schema: u64) -> Value {
    let mut p = crate::redemption::pipeline();
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
        p[format!("Shop.{name}")] = serde_json::from_str(
            &p[format!("Scratch.{name}")]
                .to_string()
                .replace("Scratch.", "Shop."),
        )
        .unwrap();
    }
    p["Shop.AreaText"]["expected"] = json!(["^录像店$"]);
    p["Shop.PointText"]["expected"] = json!(["^柜台$"]);
    for (name, roi, text) in [
        ("MainText", [1272, 14, 280, 70], "^经营状况$"),
        ("BookText", [528, 224, 151, 49], "^昨日账本$"),
        ("PromoterTitle", [104, 1006, 316, 44], "^选择宣传员$"),
        ("PromoterConfirm", [1590, 1004, 226, 50], "^确认$"),
        ("RemoveText", [1628, 1004, 188, 48], "^换下$"),
        ("VideoTitle", [1588, 28, 228, 50], "^上架筛选$"),
        ("RecommendedText", [1320, 1002, 196, 50], "^推荐上架$"),
        ("ShelfText", [1628, 1004, 188, 48], "^上架$"),
        ("UnshelfText", [1628, 1004, 188, 48], "^下架$"),
        ("StartText", [1536, 950, 256, 48], "^开始营业$"),
        ("RunningText", [1536, 950, 256, 48], "^正在营业$"),
        ("StartPrompt", [600, 440, 720, 130], ".*是否准备营业.*"),
        ("StartConfirmText", [1032, 600, 200, 56], "^确认$"),
        ("CancelText", [650, 600, 290, 56], "^取消$"),
        (
            "StartedText",
            [600, 440, 720, 130],
            ".*(丁尼广进|生意兴隆).*",
        ),
        ("StartedConfirm", [800, 600, 250, 56], "^确认$"),
        ("ViewText", [1438, 536, 476, 294], "^查看经营状况$"),
        ("DeclineText", [1438, 536, 476, 294], "^暂时不用$"),
        (
            "NarrationText",
            [380, 780, 1160, 250],
            ".*(熟悉的一切|层层安检|阿玖偷偷送上来).*",
        ),
        ("PromoterName", [530, 140, 1240, 832], "^随机$"),
        ("ThemeText", [1124, 104, 700, 324], "^纪实$"),
        ("VideoGenre", [0, 200, 480, 90], ".*纪实.*"),
        ("VideoName1", [1030, 550, 250, 60], ".{2,}"),
        ("VideoName2", [1290, 550, 250, 60], ".{2,}"),
        ("VideoName3", [1540, 550, 250, 60], ".{2,}"),
    ] {
        p[format!("Shop.{name}")] = ocr(roi, text);
    }
    p["Shop.White"] =
        json!({"recognition":"ColorMatch","method":4,"lower":[230,230,230],"upper":[255,255,255]});
    p["Shop.PromoterName"]["color_filter"] = json!("Shop.White");
    p["Shop.Back"] = json!({"recognition":"TemplateMatch","template":"daily/back.png","roi":[54,12,108,72],"threshold":0.7,"green_mask":true});
    p["Shop.BookClose"] = json!({"recognition":"TemplateMatch","template":"daily/close.png","roi":[1250,100,250,250],"threshold":0.7,"green_mask":true});
    p["Shop.Main"] = and(&["Shop.MainText", "Shop.Back"]);
    p["Shop.Book"] = and(&["Shop.BookText", "Shop.BookClose"]);
    p["Shop.Promoters"] = and(&["Shop.PromoterTitle", "Shop.Back"]);
    p["Shop.Videos"] = and(&["Shop.VideoTitle", "Shop.Back"]);
    p["Shop.Running"] = and(&["Shop.Main", "Shop.RunningText"]);
    p["Shop.StartConfirm"] = and(&[
        "Shop.StartPrompt",
        "Shop.StartConfirmText",
        "Shop.CancelText",
    ]);
    p["Shop.Started"] = and(&["Shop.StartedText", "Shop.StartedConfirm"]);
    p["Shop.Options"] = and(&["Shop.ViewText", "Shop.DeclineText"]);
    p["Shop.Narration"] = and(&["Shop.NarrationText"]);
    p["Shop.Modal"] =
        json!({"recognition":"Or","any_of":["Shop.StartConfirm","Shop.Started","Shop.Book"]});
    p["Shop.Blocked"] = and(&["Shop.Modal"]);
    let theme_nodes: Vec<_> = THEMES
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let name = format!("Shop.FilterTheme{i}");
            p[&name] = ocr([1124, 104, 700, 324], &format!("^{t}$"));
            name
        })
        .collect();
    p["Shop.FilterOptions"] = json!({"recognition":"Or","any_of":theme_nodes});
    p["Shop.Filter"] = and(&["Shop.Videos", "Shop.FilterOptions"]);
    p["Shop.Inspect"] = next(&[
        "Shop.StartConfirm",
        "Shop.Started",
        "Shop.Book",
        "Shop.Running",
        "Shop.Main",
        "Shop.Filter",
        "Shop.Promoters",
        "Shop.Videos",
        "Shop.Options",
        "Shop.Narration",
        "Shop.PointSelected",
        "Shop.AreaSelected",
        "Shop.MapOpened",
        "Shop.World",
    ]);
    p["Shop.RetryInspect"] = next(&[
        "Shop.StartConfirm",
        "Shop.Started",
        "Shop.Book",
        "Shop.Running",
        "Shop.Main",
        "Shop.Options",
        "Shop.Narration",
        "Shop.World",
    ]);
    p["Shop.WaitInteraction"] = next(&[
        "Shop.StartConfirm",
        "Shop.Started",
        "Shop.Book",
        "Shop.Running",
        "Shop.Main",
        "Shop.Options",
        "Shop.Narration",
    ]);
    p["Shop.CloseBook"] = click(&["Shop.Book", "Shop.BookClose"], "Shop.WaitBookClosed");
    p["Shop.WaitBookClosed"] = next(&["Shop.BookPending", "Shop.BookClosed"]);
    p["Shop.BookPending"] = and(&["Shop.Book"]);
    p["Shop.BookPending"]["next"] = json!(["Shop.WaitBookClosed"]);
    p["Shop.BookClosed"] = and(&["Shop.Main"]);
    p["Shop.ViewBusiness"] = click(&["Shop.Options", "Shop.ViewText"], "Shop.Viewed");
    p["Shop.DeclineInteraction"] = click(&["Shop.Options", "Shop.DeclineText"], "Shop.Declined");
    p["Shop.AdvanceNarration"] = click(&["Shop.Narration"], "Shop.Advanced");
    p["Shop.AdvanceNarration"]["target"] = json!([880, 800, 160, 40]);
    for name in [
        "Viewed",
        "Declined",
        "Advanced",
        "Moved",
        "Interacted",
        "PromoterSelected",
        "PromotersScrolled",
        "Recommended",
        "ThemeChosen",
        "ThemesScrolled",
        "StartSubmitted",
    ] {
        p[format!("Shop.{name}")] = json!({});
    }
    p["Shop.Approach"] = key("Shop.World", 87, 1000, "Shop.MoveUp");
    p["Shop.MoveUp"] = json!({"action":"KeyUp","key":87,"next":["Shop.Moved"]});
    p["Shop.Interact"] = key("Shop.World", 70, 200, "Shop.InteractUp");
    p["Shop.Interact"]["pre_delay"] = json!(1000);
    p["Shop.InteractUp"] = json!({"action":"KeyUp","key":70,"next":["Shop.Interacted"]});
    p["Shop.Interacted"]["post_delay"] = json!(1000);
    for (name, target, guard, end) in [
        (
            "OpenPromoters",
            [800, 620, 200, 272],
            "Shop.Main",
            "Shop.Promoters",
        ),
        (
            "OpenVideos",
            [1106, 618, 164, 278],
            "Shop.Main",
            "Shop.Videos",
        ),
        (
            "SelectRandom",
            [532, 144, 188, 278],
            "Shop.Promoters",
            "Shop.PromoterSelected",
        ),
        (
            "SelectFallback",
            [532, 144, 188, 278],
            "Shop.Promoters",
            "Shop.PromoterSelected",
        ),
        (
            "SelectVideo",
            [532, 144, 188, 278],
            "Shop.Videos",
            "Shop.VideoChosen",
        ),
    ] {
        p[format!("Shop.{name}")] = next(&["Shop.Blocked", &format!("Shop.{name}Click")]);
        p[format!("Shop.{name}Click")] = click(&[guard], end);
        p[format!("Shop.{name}Click")]["target"] = json!(target);
    }
    p["Shop.PromoterAlready"] = and(&["Shop.Promoters", "Shop.RemoveText"]);
    p["Shop.ChoosePromoter"] = next(&["Shop.Blocked", "Shop.PromoterAlready", "Shop.SelectRandom"]);
    p["Shop.PromoterMissing"] = and(&["Shop.Promoters"]);
    p["Shop.SelectByName"] = click(
        &["Shop.Promoters", "Shop.PromoterName"],
        "Shop.PromoterSelected",
    );
    for i in 0..2 {
        p[format!("Shop.PromoterName{i}")] = p["Shop.PromoterName"].clone();
        p[format!("Shop.SelectByName{i}")] = click(
            &["Shop.Promoters", &format!("Shop.PromoterName{i}")],
            "Shop.PromoterSelected",
        );
    }
    for agent in super::agents_for_schema(schema) {
        for id in agent.templates {
            let name = format!("Shop.Avatar_{id}");
            p[&name] = json!({"recognition":"FeatureMatch","template":format!("random-play/avatar_{id}.png"),
                "roi":[530,140,1240,832],"detector":"SIFT","ratio":0.7,"count":4,"green_mask":true,"order_by":"Score"});
            p[format!("Shop.SelectAvatar_{id}")] =
                click(&["Shop.Promoters", &name], "Shop.PromoterSelected");
        }
    }
    p["Shop.ScrollPromoters"] = swipe(
        "Shop.Promoters",
        [1150, 800],
        [1150, 400],
        "Shop.PromotersScrolled",
    );
    p["Shop.ConfirmPromoter"] = next(&["Shop.Blocked", "Shop.ConfirmPromoterClick"]);
    p["Shop.ConfirmPromoterClick"] = click(
        &["Shop.Promoters", "Shop.PromoterConfirm"],
        "Shop.PromoterConfirmed",
    );
    p["Shop.PromoterConfirmed"] =
        json!({"recognition":"Or","any_of":["Shop.Main","Shop.PromoterAlready"]});
    p["Shop.TryRecommended"] = next(&[
        "Shop.Blocked",
        "Shop.ClickRecommended",
        "Shop.NoRecommendation",
    ]);
    p["Shop.ClickRecommended"] =
        click(&["Shop.Videos", "Shop.RecommendedText"], "Shop.Recommended");
    p["Shop.NoRecommendation"] = and(&["Shop.Videos"]);
    p["Shop.OpenFilter"] = next(&["Shop.Blocked", "Shop.Filter", "Shop.OpenFilterClick"]);
    p["Shop.OpenFilterClick"] = click(&["Shop.Videos", "Shop.VideoTitle"], "Shop.Filter");
    p["Shop.SelectTheme"] = next(&["Shop.ClickTheme", "Shop.ThemeMissing"]);
    p["Shop.ClickTheme"] = click(&["Shop.Filter", "Shop.ThemeText"], "Shop.ThemeChosen");
    p["Shop.ThemeMissing"] = and(&["Shop.Filter"]);
    p["Shop.ScrollThemes"] = swipe(
        "Shop.Filter",
        [1474, 300],
        [1474, 200],
        "Shop.ThemesScrolled",
    );
    p["Shop.CloseFilter"] = next(&["Shop.CloseFilterClick", "Shop.FilterClosed"]);
    p["Shop.CloseFilterClick"] =
        click(&["Shop.Filter", "Shop.VideoTitle"], "Shop.WaitFilterClosed");
    p["Shop.WaitFilterClosed"] = next(&["Shop.FilterPending", "Shop.FilterClosed"]);
    p["Shop.FilterPending"] = and(&["Shop.Filter"]);
    p["Shop.FilterPending"]["next"] = json!(["Shop.WaitFilterClosed"]);
    p["Shop.FilterClosed"] = and(&["Shop.Videos"]);
    p["Shop.VideoChosen"] = and(&["Shop.Videos", "Shop.VideoGenre"]);
    p["Shop.CanShelf"] = and(&["Shop.VideoChosen", "Shop.ShelfText"]);
    p["Shop.OnShelf"] = and(&["Shop.VideoChosen", "Shop.UnshelfText"]);
    p["Shop.CheckShelf"] = next(&["Shop.Blocked", "Shop.OnShelf", "Shop.CanShelf"]);
    p["Shop.ShelfVideo"] = next(&["Shop.Blocked", "Shop.ShelfVideoClick"]);
    p["Shop.ShelfVideoClick"] = click(&["Shop.CanShelf", "Shop.ShelfText"], "Shop.ShelfApplied");
    p["Shop.ShelfApplied"] = and(&["Shop.OnShelf"]);
    p["Shop.ReturnMain"] = next(&["Shop.Blocked", "Shop.Main", "Shop.CloseSubpage"]);
    p["Shop.Subpage"] = json!({"recognition":"Or","any_of":["Shop.Promoters","Shop.Videos"]});
    p["Shop.CloseSubpage"] = click(&["Shop.Subpage", "Shop.Back"], "Shop.ReturnMain");
    p["Shop.SetupReady"] = and(&[
        "Shop.Main",
        "Shop.VideoName1",
        "Shop.VideoName2",
        "Shop.VideoName3",
    ]);
    p["Shop.SetupMissing"] = and(&["Shop.Main"]);
    p["Shop.CheckSetup"] = next(&[
        "Shop.Blocked",
        "Shop.Running",
        "Shop.SetupReady",
        "Shop.SetupMissing",
    ]);
    p["Shop.StartBusiness"] = next(&["Shop.Blocked", "Shop.StartBusinessClick"]);
    p["Shop.StartBusinessClick"] =
        click(&["Shop.SetupReady", "Shop.StartText"], "Shop.StartConfirm");
    p["Shop.ConfirmBusiness"] = click(
        &["Shop.StartConfirm", "Shop.StartConfirmText"],
        "Shop.StartSubmitted",
    );
    p["Shop.CancelStart"] = click(
        &["Shop.StartConfirm", "Shop.CancelText"],
        "Shop.WaitStartCancelled",
    );
    p["Shop.WaitStartCancelled"] = next(&["Shop.StartCancelPending", "Shop.StartCancelled"]);
    p["Shop.StartCancelPending"] = and(&["Shop.StartConfirm"]);
    p["Shop.StartCancelPending"]["next"] = json!(["Shop.WaitStartCancelled"]);
    p["Shop.StartCancelled"] = and(&["Shop.Main"]);
    p["Shop.AfterStart"] = next(&["Shop.Started", "Shop.Running"]);
    p["Shop.DismissStarted"] = click(&["Shop.Started", "Shop.StartedConfirm"], "Shop.Dismissed");
    p["Shop.Dismissed"] = json!({});
    p["Shop.Return"] = next(&[
        "Shop.StartConfirm",
        "Shop.Book",
        "Shop.World",
        "Shop.ReturnDismiss",
        "Shop.ReturnClose",
        "Shop.ReturnSubpage",
        "Shop.ReturnOptions",
    ]);
    p["Shop.ReturnDismiss"] = click(&["Shop.Started", "Shop.StartedConfirm"], "Shop.Return");
    p["Shop.ReturnClose"] = click(&["Shop.Main", "Shop.Back"], "Shop.Return");
    p["Shop.ReturnSubpage"] = click(&["Shop.Subpage", "Shop.Back"], "Shop.Return");
    p["Shop.ReturnOptions"] = click(&["Shop.Options", "Shop.DeclineText"], "Shop.Return");
    for (name, node) in p.as_object_mut().unwrap() {
        if !name.starts_with("Shop.") {
            continue;
        }
        for (key, value) in [
            ("recognition", json!("DirectHit")),
            ("action", json!("DoNothing")),
            ("pre_delay", json!(0)),
            ("post_delay", json!(250)),
            ("rate_limit", json!(50)),
            ("timeout", json!(10000)),
            ("next", json!([])),
            ("on_error", json!([])),
        ] {
            node.as_object_mut().unwrap().entry(key).or_insert(value);
        }
    }
    p
}

pub fn overrides(progress: &RandomPlayProgress) -> Value {
    overrides_for_schema(progress, 7)
}
pub fn overrides_for_schema(progress: &RandomPlayProgress, schema: u64) -> Value {
    let mut value = json!({
        "Shop.AreaText":{"expected":[format!("^{}$",progress.config.transport_point.area())]},
        "Shop.PointText":{"expected":[format!("^{}$",progress.config.transport_point.point())]},
        "Shop.SelectRandomClick":{"target":if progress.random_slot()==1 {[532,144,188,278]} else {[744,144,180,276]}}
    });
    if !progress.config.random() {
        let mut candidates = vec![
            "Shop.Blocked".to_string(),
            "Shop.PromoterAlready".to_string(),
        ];
        for (i, name) in progress.candidates().into_iter().enumerate() {
            let text = format!("Shop.PromoterName{i}");
            value[&text] = json!({"expected":[format!("^{}$",escape(name))]});
            let select = format!("Shop.SelectByName{i}");
            candidates.push(select);
            if let Some(agent) = super::agents_for_schema(schema)
                .into_iter()
                .find(|a| a.name == name)
            {
                candidates.extend(
                    agent
                        .templates
                        .iter()
                        .map(|id| format!("Shop.SelectAvatar_{id}")),
                );
            }
        }
        candidates.push("Shop.PromoterMissing".into());
        value["Shop.ChoosePromoter"] = json!({"next":candidates});
    }
    if let Some(theme) = progress.current_theme() {
        value["Shop.ThemeText"] = json!({"expected":[format!("^{theme}$")]});
        value["Shop.VideoGenre"] = json!({"expected":[format!(".*{theme}.*")]});
    }
    value
}
fn escape(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if ".+*?()[]{}^$|\\".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}
fn ocr(roi: [i32; 4], text: &str) -> Value {
    json!({"recognition":"OCR","model":"ppocrv5","roi":roi,"expected":[text],"threshold":0.6})
}
fn and(nodes: &[&str]) -> Value {
    json!({"recognition":"And","all_of":nodes})
}
fn next(nodes: &[&str]) -> Value {
    json!({"next":nodes})
}
fn click(nodes: &[&str], next: &str) -> Value {
    json!({"recognition":"And","all_of":nodes,"box_index":nodes.len()-1,
        "action":"LongPress","duration":100,"next":[next]})
}
fn key(guard: &str, key: i32, hold: u32, next: &str) -> Value {
    json!({"recognition":"And","all_of":[guard],"action":"KeyDown","key":key,"auto_up":true,"post_delay":hold,"next":[next]})
}
fn swipe(guard: &str, begin: [i32; 2], end: [i32; 2], next: &str) -> Value {
    json!({"recognition":"And","all_of":[guard],"action":"Swipe","begin":begin,"end":end,"duration":400,"next":[next]})
}
