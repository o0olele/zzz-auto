//! 深度追猎使用独立电量区域，共用动作只在 Rust 复核之后投递。
use serde_json::{Value, json};
pub fn pipeline() -> Value {
    let mut p = json!({});
    for (name, roi) in [
        ("Modal", [570, 290, 740, 520]),
        ("Street", [238, 24, 150, 58]),
        ("Title", [750, 168, 726, 97]),
        ("Names", [240, 216, 1512, 250]),
        ("ListRewards", [192, 992, 176, 33]),
        ("RewardsLabel", [1380, 1004, 126, 46]),
        ("RewardsCount", [1516, 1018, 22, 25]),
        ("NoReward", [1602, 1000, 228, 55]),
        ("Next", [1590, 1000, 260, 55]),
        ("Current", [1465, 1000, 63, 55]),
        ("Required", [1533, 1000, 53, 55]),
        ("Mode", [1658, 672, 96, 50]),
        ("ModeConfirm", [1030, 602, 196, 50]),
        ("Deploy", [1634, 1000, 198, 54]),
        ("Team", [368, 1002, 195, 55]),
        ("Finish", [1566, 1004, 266, 52]),
        ("Retry", [1232, 1006, 276, 46]),
        ("Retreat", [526, 848, 220, 64]),
        ("Rewind", [874, 850, 214, 64]),
        ("Exit", [1566, 1004, 266, 52]),
    ] {
        p[format!("Hunt.{name}")] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":roi,"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    }
    p["Hunt.RewardsCount"]["only_rec"] = json!(true);
    p["Hunt.ModeOn"] = p["Hunt.Mode"].clone();
    p["Hunt.ModeOn"]["color_filter"] = json!("Hunt.ModeColor");
    p["Hunt.ModeColor"] = json!({"recognition":"ColorMatch","enabled":false,"action":"DoNothing","next":[],"on_error":[],
        "roi":[1658,672,96,50],"method":4,"lower":[70,80,0],"upper":[255,255,255]});
    p["Hunt.Back"] = json!({"recognition":"TemplateMatch","template":["deep-hunt/back.png"],"roi":[82,13,68,77],"threshold":0.7,"method":5,"green_mask":true,"action":"DoNothing","next":[],"on_error":[]});
    p["Hunt.Info"] = json!({"recognition":"TemplateMatch","roi":[1480,675,45,45],"template":["deep-hunt/info.png"],"threshold":0.7,"method":5,"green_mask":true,"action":"DoNothing","next":[],"on_error":[]});
    p
}

pub fn info_template() -> &'static [u8] {
    static IMAGE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    IMAGE.get_or_init(|| {
        let mut raw = image::load_from_memory(include_bytes!(
            "../../../resources/source/deep-hunt/info-raw.png"
        ))
        .expect("固定深度信息模板有效")
        .to_rgb8();
        let mask = image::load_from_memory(include_bytes!(
            "../../../resources/source/deep-hunt/info-mask.png"
        ))
        .expect("固定深度信息掩码有效")
        .to_luma8();
        assert_eq!(raw.dimensions(), mask.dimensions());
        for (pixel, mask) in raw.pixels_mut().zip(mask.pixels()) {
            assert!(matches!(mask[0], 0 | 255));
            if mask[0] == 0 {
                *pixel = image::Rgb([0, 255, 0]);
            } else {
                assert_ne!(pixel.0, [0, 255, 0]);
            }
        }
        let mut png = std::io::Cursor::new(Vec::new());
        raw.write_to(&mut png, image::ImageFormat::Png)
            .expect("固定深度信息模板编码成功");
        png.into_inner()
    })
}

pub fn back_template() -> &'static [u8] {
    crate::basic_daily::templates()
        .into_iter()
        .find(|(n, _)| *n == "back")
        .expect("固定返回模板")
        .1
}
