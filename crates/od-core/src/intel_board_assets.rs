pub type SourceImage = (&'static str, &'static [u8], &'static [u8]);
pub fn source_images() -> [SourceImage; 3] {
    [
        (
            "star",
            include_bytes!("../../../resources/source/intel-board/star-raw.png"),
            include_bytes!("../../../resources/source/intel-board/star-mask.png"),
        ),
        (
            "refresh",
            include_bytes!("../../../resources/source/intel-board/refresh-raw.png"),
            include_bytes!("../../../resources/source/intel-board/refresh-mask.png"),
        ),
        (
            "function-menu",
            include_bytes!("../../../resources/source/intel-board/function-menu-raw.png"),
            include_bytes!("../../../resources/source/intel-board/function-menu-mask.png"),
        ),
    ]
}
pub fn templates() -> &'static std::collections::BTreeMap<String, Vec<u8>> {
    static CACHE: std::sync::OnceLock<std::collections::BTreeMap<String, Vec<u8>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        source_images()
            .into_iter()
            .map(|(name, raw, mask)| {
                let mut image = image::load_from_memory(raw)
                    .expect("固定情报板模板无效")
                    .to_rgb8();
                let mask = image::load_from_memory(mask)
                    .expect("固定情报板掩码无效")
                    .to_luma8();
                assert_eq!(image.dimensions(), mask.dimensions());
                for (pixel, mask) in image.pixels_mut().zip(mask.pixels()) {
                    assert!(matches!(mask[0], 0 | 255));
                    if mask[0] == 0 {
                        *pixel = image::Rgb([0, 255, 0]);
                    } else {
                        assert_ne!(pixel.0, [0, 255, 0]);
                    }
                }
                let mut png = std::io::Cursor::new(Vec::new());
                image
                    .write_to(&mut png, image::ImageFormat::Png)
                    .expect("固定情报板模板编码失败");
                (format!("image/intel-board/{name}.png"), png.into_inner())
            })
            .collect()
    })
}
