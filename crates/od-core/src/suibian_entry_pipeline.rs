//! 完整随便观的固定传送与返回；旧子流程资源保持冻结。
use serde_json::{Value, json};

pub fn pipeline() -> Value {
    let mut p = crate::suibian_boo_box_pipeline::pipeline();
    let daily = crate::basic_daily::legacy_pipeline();
    for name in [
        "Base.WorldFunction",
        "Base.WorldPreset",
        "Base.World",
        "Base.MenuBack",
    ] {
        p[name] = daily[name].clone();
        if let Some(templates) = p[name]["template"].as_array_mut() {
            for template in templates {
                *template = json!(
                    template
                        .as_str()
                        .unwrap()
                        .replace("daily/", "suibian-temple/")
                );
            }
        }
    }
    for name in [
        "OpenMap",
        "MapUp",
        "MapOpened",
        "SelectArea",
        "AreaSelected",
        "SelectPoint",
        "PointSelected",
        "ConfirmTransport",
        "Transported",
        "MapPrompt",
        "MapConfirm",
    ] {
        let node = daily[format!("Scratch.{name}")].clone();
        // 固定定义仅含节点名和数值；整份 JSON 替换前缀保持所有引用一致。
        p[format!("Temple.{name}")] =
            serde_json::from_str(&node.to_string().replace("Scratch.", "Temple.")).unwrap();
    }
    p["Temple.AreaText"] = json!({"recognition":"OCR","model":"ppocrv5","roi":[0,90,1920,840],"expected":["^澄辉坪$"],"threshold":0.6});
    p["Temple.PointText"] = json!({"recognition":"OCR","model":"ppocrv5","roi":[70,1000,1778,58],"expected":["^随便观$"],"threshold":0.6});
    p["Temple.World"] = json!({"recognition":"And","all_of":["Base.World"]});
    p["Temple.Entry"] =
        json!({"recognition":"Or","any_of":["Hosting.EntryManaged","Hosting.EntryManual"]});
    p["Temple.Inspect"] = json!({"next":["Temple.Entry","Temple.PointSelected","Temple.AreaSelected","Temple.MapOpened","Temple.World"]});
    p["Temple.WaitArrival"] = json!({"next":["Temple.Entry"]});
    p["Temple.Return"] = json!({"next":["Temple.World","Temple.CloseEntry"]});
    p["Temple.CloseEntry"] = json!({"recognition":"And","all_of":["Temple.Entry"],"action":"KeyDown","key":27,"next":["Temple.EscapeUp"]});
    p["Temple.EscapeUp"] =
        json!({"action":"KeyUp","key":27,"next":["Temple.World","Temple.CloseEntry"]});
    p
}

pub fn templates() -> Vec<(&'static str, &'static [u8])> {
    crate::basic_daily::templates()
        .into_iter()
        .filter(|(name, _)| matches!(*name, "world_function" | "world_preset"))
        .collect()
}
