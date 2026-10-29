use serde_json::{Value, json};
pub fn pipeline() -> Value {
    json!({
        "SynthesisEntry.Full":{"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":[0,0,1920,1080],"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0},
        "SynthesisEntry.Battery":{"recognition":"TemplateMatch","roi":[1220,35,550,75],"template":["ether-synthesis-entry/battery.png"],"threshold":0.7,"method":5,"green_mask":true,"action":"DoNothing","next":[],"on_error":[]}
    })
}
pub fn battery_template() -> &'static [u8] {
    static IMAGE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    IMAGE.get_or_init(|| {
        let mut raw = image::load_from_memory(include_bytes!(
            "../../../resources/source/ether-synthesis-entry/battery-raw.png"
        ))
        .expect("固定电池模板有效")
        .to_rgb8();
        let mask = image::load_from_memory(include_bytes!(
            "../../../resources/source/ether-synthesis-entry/battery-mask.png"
        ))
        .expect("固定电池掩码有效")
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
            .expect("固定电池模板编码成功");
        png.into_inner()
    })
}
pub fn source_images() -> [(&'static str, &'static [u8]); 2] {
    [
        (
            "assets/template/compendium/battery/raw.png",
            include_bytes!("../../../resources/source/ether-synthesis-entry/battery-raw.png"),
        ),
        (
            "assets/template/compendium/battery/mask.png",
            include_bytes!("../../../resources/source/ether-synthesis-entry/battery-mask.png"),
        ),
    ]
}
