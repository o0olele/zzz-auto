use super::*;

#[test]
fn reused_tensor_preserves_every_byte_value_and_rewrites_all_planes() -> Result<()> {
    let mut tensor = Vec::new();
    for pass in 0..3 {
        let pixels: Vec<_> = (0..256 * 3)
            .map(|v| ((v + pass * 43) % 256) as u8)
            .collect();
        let previous = tensor.as_ptr();
        bgr_to_rgb_tensor_into(&pixels, [256, 1], None, &mut tensor)?;
        if pass > 0 {
            assert_eq!(tensor.as_ptr(), previous);
        }
        for (i, pixel) in pixels.chunks_exact(3).enumerate() {
            for (channel, source) in [2, 1, 0].into_iter().enumerate() {
                assert_eq!(
                    tensor[channel * 256 + i].to_bits(),
                    ((f64::from(pixel[source]) / 255.0) as f32).to_bits()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn cancelled_conversion_is_not_reused_as_a_completed_input() -> Result<()> {
    let mut tensor = vec![f32::NAN; 640 * 640 * 3];
    let pixels = vec![23u8; tensor.len()];
    let control = RawRecognitionControl::default();
    control.cancel()?;
    let error = control
        .execute(|| bgr_to_rgb_tensor_into(&pixels, [640, 640], Some(&control), &mut tensor))
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::RawRecognitionCancelled>()
            .is_some()
    );
    let control = RawRecognitionControl::default();
    control.execute(|| bgr_to_rgb_tensor_into(&pixels, [640, 640], Some(&control), &mut tensor))?;
    assert!(
        tensor
            .iter()
            .all(|v| v.to_bits() == ((23f64 / 255.0) as f32).to_bits())
    );
    Ok(())
}

fn reference(image: &RecognitionImage, [x, y, w, h]: [i32; 4]) -> Result<(Vec<u8>, Vec<f32>)> {
    let source = image.buffer.raw_data().unwrap();
    let mut crop = Vec::new();
    for row in y..y + h {
        let offset = ((row * 1920 + x) * 3) as usize;
        crop.extend_from_slice(&source[offset..offset + w as usize * 3]);
    }
    let bgr = flash::prepare_flash_bgr(&crop, w, h)?.bgr;
    let mut tensor = vec![0.0; bgr.len()];
    for (i, pixel) in bgr.chunks_exact(3).enumerate() {
        for (channel, source) in [2, 1, 0].into_iter().enumerate() {
            tensor[channel * 640 * 640 + i] = (f64::from(pixel[source]) / 255.0) as f32;
        }
    }
    Ok((bgr, tensor))
}

#[test]
#[ignore = "需要固定 Maa SDK；生成像素，不连接游戏"]
fn borrowed_roi_and_reused_buffers_match_cropped_reference_and_model_scores() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    crate::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/combat-preprocess-native"),
    )?;
    let pixels: Vec<_> = (0..1920 * 1080 * 3)
        .map(|i| ((i * 37 + i / 1920) % 256) as u8)
        .collect();
    let mut buffer = maa_framework::buffer::MaaImageBuffer::new()?;
    buffer.set_raw_data(&pixels, 1920, 1080, 16)?;
    let image = RecognitionImage {
        buffer,
        prepare_us: 0,
    };
    let mut bgr = Vec::new();
    let mut tensor = Vec::new();
    for roi in [
        [0, 0, 1920, 1080],
        [101, 203, 319, 181],
        [700, 50, 99, 900],
        [1913, 1071, 7, 9],
        [0, 0, 1920, 1080],
    ] {
        let previous = (bgr.as_ptr(), tensor.as_ptr());
        let warm = !bgr.is_empty();
        flash::prepare_flash_image_into(&image, roi, &mut bgr)?;
        bgr_to_rgb_tensor_into(&bgr, [640, 640], None, &mut tensor)?;
        let (expected_bgr, expected_tensor) = reference(&image, roi)?;
        assert_eq!(bgr, expected_bgr, "{roi:?}: ROI 跨度 / 填充不一致");
        assert_eq!(tensor, expected_tensor, "{roi:?}: 模型输入不一致");
        if warm {
            assert_eq!((bgr.as_ptr(), tensor.as_ptr()), previous);
        }
    }
    assert!(flash::prepare_flash_image_into(&image, [1919, 0, 2, 1], &mut bgr).is_err());
    // 相同真实模型，对照独立裁图 + f64 除法的原始输入，不能只比较预处理图片。
    let native = NativeSession::for_fixture(
        &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
        &root.join("resources/generated/combat"),
        InferenceDevice::Cpu,
    )?;
    let mut previous = None;
    for roi in [[0, 0, 1920, 1080], [101, 203, 319, 181], [700, 50, 99, 900]] {
        let result = native.recognize_prepared(
            &image,
            "LegacyFlashRaw",
            &json!({"model":RAW_FLASH_MODEL,"roi":roi}),
        )?;
        let (_, expected_tensor) = reference(&image, roi)?;
        let mut cache = native.raw_flash.lock().unwrap();
        let classifier = cache.as_mut().unwrap();
        let pointers = (classifier.bgr.as_ptr(), classifier.tensor.as_ptr());
        if let Some(previous) = previous {
            assert_eq!(pointers, previous);
        }
        previous = Some(pointers);
        let reference = ort::value::Tensor::from_array(([1, 3, 640, 640], expected_tensor))?;
        let output = classifier.session.run(ort::inputs!["images"=>reference])?;
        let (_, scores) = output["output0"].try_extract_tensor::<f32>()?;
        for (i, score) in scores.iter().enumerate() {
            assert_eq!(
                (result.detail["adaptation"]["raw_scores"][i]
                    .as_f64()
                    .unwrap() as f32)
                    .to_bits(),
                score.to_bits()
            );
        }
    }
    let roi = [0, 0, 1920, 1080];
    let start = Instant::now();
    for _ in 0..30 {
        std::hint::black_box(reference(&image, roi)?);
    }
    let reference_us = start.elapsed().as_micros();
    let start = Instant::now();
    for _ in 0..30 {
        flash::prepare_flash_image_into(&image, roi, &mut bgr)?;
        bgr_to_rgb_tensor_into(&bgr, [640, 640], None, &mut tensor)?;
        std::hint::black_box(&tensor);
    }
    eprintln!(
        "flash_preprocess_comparison {}",
        json!({"iterations":30,"reference_us":reference_us,"reused_us":start.elapsed().as_micros(),"bgr_capacity":bgr.capacity(),"tensor_capacity":tensor.capacity()})
    );
    Ok(())
}
