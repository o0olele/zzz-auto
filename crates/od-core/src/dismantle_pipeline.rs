use serde_json::{Value, json};

/// 固定画面来源见 dismantle-source-lock.json；所有点击先确认当前页面。
pub fn pipeline() -> Value {
    let mut p = crate::remaining_daily::pipeline();
    for name in [
        "OpenMenu",
        "HoldAlt",
        "ClickMenu",
        "ReleaseAlt",
        "Navigated",
    ] {
        p[format!("Disc.{name}")] = serde_json::from_str(
            &p[format!("Fund.{name}")]
                .to_string()
                .replace("Fund.", "Disc."),
        )
        .unwrap();
    }
    for (name, roi, text) in [
        ("StorageText", [65, 110, 440, 65], "^驱动仓库.*"),
        ("WengineText", [65, 110, 440, 65], "^音擎仓库.*"),
        ("RecycleText", [250, 900, 188, 60], "^回收$"),
        ("OpenText", [250, 900, 188, 60], "^拆解$"),
        ("StorageEntry", [200, 924, 1528, 134], "^仓库$"),
        ("Title", [168, 17, 390, 75], "^驱动盘拆解.*"),
        ("QuickText", [134, 968, 273, 100], "^快速选择$"),
        ("FilterTitle", [580, 280, 300, 80], "^快速选择$"),
        ("ResetText", [650, 750, 300, 65], "^重置$"),
        ("AbandonedText", [642, 466, 296, 36], "^全选已弃置$"),
        ("GradeBText", [642, 566, 288, 44], "^B$"),
        ("GradeAText", [984, 568, 286, 42], "^A及以下$"),
        ("GradeSText", [642, 628, 292, 42], "^S及以下$"),
        ("ApplyText", [994, 754, 240, 48], "^确认$"),
        ("DismantleText", [1626, 876, 182, 50], "^拆解$"),
        ("ConfirmTitle", [550, 300, 820, 280], ".*(拆解|分解).*"),
        ("ConfirmText", [1030, 622, 190, 50], "^确认$"),
        ("CancelText", [650, 622, 280, 60], "^取消$"),
        ("ObtainedText", [860, 386, 200, 74], "^获得$"),
        ("ObtainedConfirm", [820, 712, 340, 78], "^确认$"),
        (
            "ZeroText",
            [168, 17, 390, 75],
            r"^驱动盘拆解\s*[\[【(（]?\s*0\s*/\s*300.*",
        ),
        (
            "CountText",
            [168, 17, 390, 75],
            r"^驱动盘拆解\s*[\[【(（]?\s*[1-9][0-9]{0,2}\s*/\s*300.*",
        ),
    ] {
        p[format!("Disc.{name}")] = ocr(roi, text);
    }
    p["Disc.World"] = and(&["Base.World"]);
    p["Disc.Menu"] = and(&["Base.Menu"]);
    p["Disc.Storage"] = and(&["Disc.StorageText", "Disc.OpenText"]);
    p["Disc.Wengine"] = and(&["Disc.WengineText", "Disc.RecycleText"]);
    p["Disc.Filter"] = and(&["Disc.FilterTitle", "Disc.ResetText", "Disc.ApplyText"]);
    p["Disc.Confirm"] = and(&["Disc.ConfirmTitle", "Disc.CancelText", "Disc.ConfirmText"]);
    p["Disc.Obtained"] = and(&["Disc.ObtainedText", "Disc.ObtainedConfirm"]);
    // 模态窗口后面的标题仍可能可见；输入前有序检查模态，命中则停止本步骤。
    // SDK 的复合识别不应用内联 inverse，不能用它表达“窗口已关闭”。
    p["Disc.Page"] = and(&["Disc.Title", "Disc.QuickText"]);
    p["Disc.Blocked"] =
        json!({"recognition":"Or","any_of":["Disc.Confirm","Disc.Filter","Disc.Obtained"]});
    p["Disc.Empty"] = and(&["Disc.Page", "Disc.ZeroText"]);
    p["Disc.Selected"] = and(&["Disc.Page", "Disc.CountText"]);
    p["Disc.Inspect"] = next(&[
        "Disc.Confirm",
        "Disc.Filter",
        "Disc.Page",
        "Disc.Storage",
        "Disc.Wengine",
        "Disc.Menu",
        "Disc.World",
    ]);
    for (name, guard, target) in [
        ("OpenStorage", "Disc.Menu", "Disc.StorageEntry"),
        ("OpenDismantle", "Disc.Storage", "Disc.OpenText"),
    ] {
        p[format!("Disc.{name}")] = click(&[guard, target], "Disc.Navigated");
    }
    p["Disc.SelectDiscs"] = click(&["Disc.Wengine"], "Disc.Navigated");
    p["Disc.SelectDiscs"]["target"] = json!([1510, 126, 78, 84]);
    p["Disc.OpenFilter"] = next(&["Disc.Blocked", "Disc.OpenFilterClick"]);
    p["Disc.OpenFilterClick"] = click(&["Disc.Page", "Disc.QuickText"], "Disc.Filter");
    p["Disc.ResetFilter"] = click(&["Disc.Filter", "Disc.ResetText"], "Disc.FilterReset");
    p["Disc.FilterReset"] = and(&["Disc.Filter"]);
    p["Disc.SelectGrade"] = next(&["Disc.GradeA"]);
    for (grade, target) in [
        ("B", "GradeBText"),
        ("A", "GradeAText"),
        ("S", "GradeSText"),
    ] {
        p[format!("Disc.Grade{grade}")] = click(
            &["Disc.Filter", &format!("Disc.{target}")],
            "Disc.GradeSelected",
        );
    }
    p["Disc.GradeSelected"] = and(&["Disc.Filter"]);
    p["Disc.SelectAbandoned"] = click(
        &["Disc.Filter", "Disc.AbandonedText"],
        "Disc.AbandonedSelected",
    );
    p["Disc.AbandonedSelected"] = and(&["Disc.Filter"]);
    p["Disc.ApplyFilter"] = click(&["Disc.Filter", "Disc.ApplyText"], "Disc.WaitFilterClose");
    p["Disc.WaitFilterClose"] = next(&["Disc.FilterPending", "Disc.FilterApplied"]);
    p["Disc.FilterPending"] = and(&["Disc.Filter"]);
    p["Disc.FilterPending"]["next"] = json!(["Disc.WaitFilterClose"]);
    p["Disc.FilterApplied"] = and(&["Disc.Page"]);
    p["Disc.CheckSelection"] = next(&["Disc.Blocked", "Disc.Empty", "Disc.Selected"]);
    p["Disc.Dismantle"] = next(&["Disc.Blocked", "Disc.Empty", "Disc.ClickDismantle"]);
    p["Disc.ClickDismantle"] = click(
        &["Disc.Selected", "Disc.DismantleText"],
        "Disc.OpenedConfirm",
    );
    p["Disc.OpenedConfirm"] = and(&["Disc.Confirm"]);
    p["Disc.CheckConfirm"] = next(&["Disc.Confirm"]);
    p["Disc.ConfirmDismantle"] = click(&["Disc.Confirm", "Disc.ConfirmText"], "Disc.Submitted");
    p["Disc.Submitted"] = json!({});
    p["Disc.CancelConfirm"] = click(&["Disc.Confirm", "Disc.CancelText"], "Disc.Cancelled");
    p["Disc.Cancelled"] = and(&["Disc.Page"]);
    p["Disc.AfterConfirm"] = next(&["Disc.Obtained"]);
    p["Disc.DismissObtained"] = click(&["Disc.Obtained", "Disc.ObtainedConfirm"], "Disc.Dismissed");
    p["Disc.Dismissed"] = and(&["Disc.Page", "Base.MenuBack"]);
    p["Disc.ReturnStorage"] = next(&[
        "Disc.Obtained",
        "Disc.Blocked",
        "Disc.Storage",
        "Disc.ClosePage",
    ]);
    p["Disc.ClosePage"] = click(&["Disc.Page", "Base.MenuBack"], "Disc.Storage");
    p["Disc.Return"] = next(&["Disc.World", "Disc.CloseStorage", "Disc.CloseMenu"]);
    p["Disc.CloseStorage"] = click(&["Disc.Storage", "Base.MenuBack"], "Disc.Return");
    p["Disc.CloseMenu"] = click(&["Disc.Menu", "Base.MenuBack"], "Disc.Return");
    for (name, node) in p.as_object_mut().unwrap() {
        if !name.starts_with("Disc.") {
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
fn ocr(roi: [i32; 4], text: &str) -> Value {
    json!({"recognition":"OCR","model":"ppocrv5","roi":roi,"expected":[text],"threshold":0.6})
}
fn and(nodes: &[&str]) -> Value {
    json!({"recognition":"And","all_of":nodes})
}
fn next(nodes: &[&str]) -> Value {
    json!({"next":nodes})
}
fn click(nodes: &[&str], end: &str) -> Value {
    let mut value = and(nodes);
    value["box_index"] = json!(nodes.len() - 1);
    value["action"] = json!("LongPress");
    value["duration"] = json!(100);
    value["next"] = json!([end]);
    value
}
