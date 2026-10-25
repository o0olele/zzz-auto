//! Maa 无视野扇形的极坐标梯度识别；复用截图，保留旧版 105° 扇形算法。
use anyhow::{Result, ensure};
use image::{GrayImage, Luma, RgbImage};
use imageproc::contours::{BorderType, find_contours};

pub const MINI_MAP_SIZE: i32 = 210;
pub const STATIC_MAP_RECT: [i32; 4] = [1534, 201, 210, 210];
pub const MAP_BUTTON_RECT: [i32; 4] = [1696, 319, 162, 96];
pub const MAP_BUTTON_DELTA: [i32; 2] = [169, 151];

#[derive(Debug, Clone, Copy)]
pub struct Heading {
    pub degrees: f64,
    pub confidence: f64,
}

/// 仅接受固定 1080p UI 的 210×210 小地图；空帧或无玩家箭头不提供朝向。
pub fn heading(rgb: &RgbImage) -> Result<Option<Heading>> {
    ensure!(
        rgb.dimensions() == (MINI_MAP_SIZE as u32, MINI_MAP_SIZE as u32),
        "小地图须为 210×210 RGB8"
    );
    if !player_found(rgb) {
        return Ok(None);
    }
    let mut mask = GrayImage::new(rgb.width(), rgb.height());
    for (x, y, pixel) in rgb.enumerate_pixels() {
        let [r, g, b] = pixel.0.map(i32::from);
        // OpenCV RGB2YUV 的 14 位定点系数，Y 先取整再用于色差。
        let luma = (r * 4899 + g * 9617 + b * 1868 + 8192) >> 14;
        let u = (((b - luma) * 8061 + 8192) >> 14) + 128;
        let v = (((r - luma) * 14369 + 8192) >> 14) + 128;
        mask.put_pixel(
            x,
            y,
            Luma([if (100..=110).contains(&u) || (140..=150).contains(&v) {
                255
            } else {
                0
            }]),
        );
    }
    // 3×3 方形核：close → open；边界只取图内像素，对应形态学默认边界。
    mask = morphology(&morphology(&mask, true), false);
    mask = morphology(&morphology(&mask, false), true);
    let mut blur = mask.clone();
    for y in 0..MINI_MAP_SIZE {
        for x in 0..MINI_MAP_SIZE {
            let mut sum = 0u32;
            for (dy, wy) in [(-1, 1), (0, 2), (1, 1)] {
                for (dx, wx) in [(-1, 1), (0, 2), (1, 1)] {
                    sum += u32::from(
                        mask.get_pixel(
                            reflect(x + dx, MINI_MAP_SIZE) as u32,
                            reflect(y + dy, MINI_MAP_SIZE) as u32,
                        )[0],
                    ) * wy
                        * wx;
                }
            }
            blur.put_pixel(x as u32, y as u32, Luma([((sum + 8) / 16) as u8]));
        }
    }
    Ok(sector_heading(&blur))
}

pub(crate) fn player_found(rgb: &RgbImage) -> bool {
    player_mask(rgb).pixels().any(|pixel| pixel[0] != 0)
}

pub(crate) fn player_mask(rgb: &RgbImage) -> GrayImage {
    let mut mask = GrayImage::new(rgb.width(), rgb.height());
    for (x, y, pixel) in rgb.enumerate_pixels() {
        let [h, s, v] = crate::combat_vision::hsv(pixel.0);
        if (14..=34).contains(&h) && s >= 90 && v >= 175 {
            mask.put_pixel(x, y, Luma([255]));
        }
    }
    let mut player = GrayImage::new(rgb.width(), rgb.height());
    for contour in find_contours::<i32>(&mask) {
        if contour.border_type != BorderType::Outer || contour.parent.is_some() {
            continue;
        }
        let mut perimeter = 0.0;
        let mut area = 0i32;
        let mut cx = 0.0;
        let mut cy = 0.0;
        for (a, b) in contour
            .points
            .iter()
            .zip(contour.points.iter().cycle().skip(1))
            .take(contour.points.len())
        {
            let cross = a.x * b.y - b.x * a.y;
            area += cross;
            cx += f64::from((a.x + b.x) * cross);
            cy += f64::from((a.y + b.y) * cross);
            perimeter += f64::from(a.x - b.x).hypot(f64::from(a.y - b.y));
        }
        // 旋转和抗锯齿会让边缘像素略有变化；仍同时约束周长与中心位置。
        if !(60.0..=70.0).contains(&perimeter) || !(460..=620).contains(&area.abs()) {
            continue;
        }
        // 防止静态兜底框偏移后把其他橙色图标当作位于中心的玩家。
        if (cx / (3.0 * f64::from(area)) - 105.0).abs() <= 8.0
            && (cy / (3.0 * f64::from(area)) - 105.0).abs() <= 8.0
        {
            imageproc::drawing::draw_polygon_mut(&mut player, &contour.points, Luma([255]));
        }
    }
    player
}

pub(crate) fn morphology(input: &GrayImage, dilate: bool) -> GrayImage {
    let mut output = input.clone();
    for y in 0..input.height() {
        for x in 0..input.width() {
            let mut value = if dilate { 0 } else { 255 };
            for yy in y.saturating_sub(1)..=(y + 1).min(input.height() - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(input.width() - 1) {
                    let pixel = input.get_pixel(xx, yy)[0];
                    value = if dilate {
                        value.max(pixel)
                    } else {
                        value.min(pixel)
                    };
                }
            }
            output.put_pixel(x, y, Luma([value]));
        }
    }
    output
}

fn reflect(at: i32, size: i32) -> i32 {
    if at < 0 {
        -at
    } else if at >= size {
        2 * size - at - 2
    } else {
        at
    }
}

// OpenCV remap(INTER_LINEAR) 的 1/32 子像素表与整数输出。
fn sample(image: &GrayImage, x: f64, y: f64) -> f64 {
    let x = ((x as f32 * 32.0).round_ties_even()) as i32;
    let y = ((y as f32 * 32.0).round_ties_even()) as i32;
    let (xx, yy) = (x >> 5, y >> 5);
    let (fx, fy) = (x & 31, y & 31);
    let mut sum = 0i32;
    for (dy, wy) in [(0, 32 - fy), (1, fy)] {
        for (dx, wx) in [(0, 32 - fx), (1, fx)] {
            if (0..image.width() as i32).contains(&(xx + dx))
                && (0..image.height() as i32).contains(&(yy + dy))
            {
                sum += i32::from(image.get_pixel((xx + dx) as u32, (yy + dy) as u32)[0]) * wx * wy;
            }
        }
    }
    f64::from((sum + 512) >> 10)
}

fn peaks(values: &[f64], min_height: f64) -> Vec<usize> {
    let mut peaks = Vec::new();
    let mut i = 1;
    while i + 1 < values.len() {
        if values[i] >= min_height && values[i] > values[i - 1] {
            let start = i;
            while i + 1 < values.len() && values[i + 1] == values[i] {
                i += 1;
            }
            if i + 1 < values.len() && values[i] > values[i + 1] {
                peaks.push((start + i) / 2);
            }
        }
        i += 1;
    }
    peaks
}

fn convolve(values: &[f64], width: i32) -> Vec<f64> {
    (0..values.len() as i32)
        .map(|i| {
            (1 - width..width)
                .filter_map(|offset| {
                    let j = i + offset;
                    (0..values.len() as i32).contains(&j).then(|| {
                        values[j as usize] * f64::from(width - offset.abs()) / f64::from(width)
                    })
                })
                .sum()
        })
        .collect()
}

fn confidence(values: &[f64]) -> f64 {
    let extended = values.repeat(3);
    let mut heights = Vec::new();
    for i in peaks(&extended, 0.0) {
        if i < values.len() || i >= values.len() * 2 {
            continue;
        }
        let height = extended[i];
        let mut left_min = height;
        let mut right_min = height;
        for &v in extended[..i].iter().rev() {
            if v > height {
                break;
            }
            left_min = left_min.min(v);
        }
        for &v in &extended[i + 1..] {
            if v > height {
                break;
            }
            right_min = right_min.min(v);
        }
        if height - left_min.max(right_min) >= 5.0 {
            heights.push(height);
        }
    }
    heights.sort_by(|a, b| b.total_cmp(a));
    match heights.as_slice() {
        [] => 0.0,
        [_] => 1.0,
        [first, second, ..] => ((first - second) / first * 1000.0).round_ties_even() / 1000.0,
    }
}

fn sector_heading(mask: &GrayImage) -> Option<Heading> {
    // 旧版半径数组为 row/2，行范围 [d*0.2,d*0.4)，即半径 21..42。
    const ROWS: usize = 42;
    let mut polar = vec![0.0; ROWS * 360];
    for row in 0..ROWS {
        let radius = (row + 42) as f64 / 2.0;
        for degree in 0..360 {
            let angle = (degree as f64).to_radians();
            polar[row * 360 + degree] = sample(
                mask,
                105.0 + radius * angle.cos(),
                105.0 - radius * angle.sin(),
            );
        }
    }
    let mut gradient = vec![0.0; polar.len()];
    for row in 0..ROWS as i32 {
        for col in 0..360 {
            for (dy, weight) in [(-1, 3.0), (0, 10.0), (1, 3.0)] {
                let yy = reflect(row + dy, ROWS as i32) as usize;
                gradient[row as usize * 360 + col as usize] += weight
                    * (polar[yy * 360 + reflect(col + 1, 360) as usize]
                        - polar[yy * 360 + reflect(col - 1, 360) as usize]);
            }
        }
    }
    let mut left = vec![0.0f64; 360];
    let mut right = vec![0.0f64; 360];
    for i in peaks(&gradient, 35.0) {
        left[i % 360] += 1.0;
    }
    for value in &mut gradient {
        *value = -*value;
    }
    for i in peaks(&gradient, 35.0) {
        right[i % 360] += 1.0;
    }
    for i in 0..360 {
        let diff = left[i] - right[i];
        left[i] = diff.max(0.0);
        right[i] = (-diff).max(0.0);
    }
    let smooth = convolve(&right, 6);
    let mut response = vec![1.0f64; 360];
    for i in 0..360 {
        for offset in -1..=1 {
            let r = (i as i32 + 105 - offset).rem_euclid(360) as usize;
            response[i] = response[i].max(left[i] * smooth[r]);
        }
    }
    let confidence = confidence(&response);
    if confidence <= 0.0 {
        return None;
    }
    // np.argmax 取第一个最大值，不能改用返回最后一个相等项的 Iterator::max。
    let mut peak = 0;
    for i in 1..360 {
        if response[i] > response[peak] {
            peak = i;
        }
    }
    Some(Heading {
        degrees: (peak as f64 + 52.5) % 360.0,
        confidence,
    })
}

pub fn crop_heading(bgr: &[u8], rect: [i32; 4]) -> Result<Option<Heading>> {
    ensure!(bgr.len() == 1920 * 1080 * 3, "导航截图必须为 1080p BGR8");
    let [x, y, width, height] = rect;
    ensure!(
        width == MINI_MAP_SIZE
            && height == MINI_MAP_SIZE
            && (0..=1920 - MINI_MAP_SIZE).contains(&x)
            && (0..=1080 - MINI_MAP_SIZE).contains(&y),
        "小地图裁剪区域越界或尺寸无效"
    );
    let rgb = RgbImage::from_fn(width as u32, height as u32, |xx, yy| {
        let at = (((y as u32 + yy) * 1920 + x as u32 + xx) * 3) as usize;
        image::Rgb([bgr[at + 2], bgr[at + 1], bgr[at]])
    });
    heading(&rgb)
}
