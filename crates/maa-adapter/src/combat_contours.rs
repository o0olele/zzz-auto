//! Maa 不提供按轮廓质心邻近度组合的识别；这里保留旧锁定与连携条规则。
use anyhow::{Result, ensure};
use imageproc::contours::{BorderType, find_contours};

pub fn contours(bgr: &[u8], rect: [i32; 4], center: [f64; 3], diff: [f64; 3], lock: bool) -> Result<usize> {
    od_core::Roi::from_legacy(rect).map_err(anyhow::Error::msg)?;
    ensure!(bgr.len() == 1920 * 1080 * 3, "轮廓输入必须为 1080p BGR8");
    let [left, top, right, bottom] = rect;
    let width = (right - left) as usize; let height = (bottom - top) as usize;
    let mut mask = image::GrayImage::new(width as u32, height as u32);
    for y in 0..height { for x in 0..width {
        let at = (((y as i32 + top) * 1920 + x as i32 + left) * 3) as usize;
        let hsv = crate::combat_vision::hsv([bgr[at + 2], bgr[at + 1], bgr[at]]);
        if (0..3).all(|i| hsv[i] as f64 >= (center[i] - diff[i]).max(0.0) && hsv[i] as f64 <= (center[i] + diff[i]).min(if i == 0 { 179.0 } else { 255.0 })) { mask.put_pixel(x as u32, y as u32, image::Luma([255])); }
    } }
    if lock {
        let mut eroded = mask.clone();
        for y in 0..height { for x in 0..width {
            let yes = (y.saturating_sub(1)..=(y + 1).min(height - 1)).all(|y| (x.saturating_sub(1)..=(x + 1).min(width - 1)).all(|x| mask.get_pixel(x as u32, y as u32)[0] != 0));
            eroded.put_pixel(x as u32, y as u32, image::Luma([if yes { 255 } else { 0 }]));
        } }
        for y in 0..height { for x in 0..width {
            let yes = (y.saturating_sub(1)..=(y + 1).min(height - 1)).any(|y| (x.saturating_sub(1)..=(x + 1).min(width - 1)).any(|x| eroded.get_pixel(x as u32, y as u32)[0] != 0));
            mask.put_pixel(x as u32, y as u32, image::Luma([if yes { 255 } else { 0 }]));
        } }
    }
    let mut centroids = Vec::new();
    let mut chain = 0;
    for contour in find_contours::<i32>(&mask) {
        if !lock && (contour.border_type != BorderType::Outer || contour.parent.is_some()) { continue; }
        let points = contour.points;
        if points.len() < 3 { continue; }
        let mut twice_area = 0.0; let mut mx = 0.0; let mut my = 0.0; let mut perimeter = 0.0;
        for (a,b) in points.iter().zip(points.iter().cycle().skip(1)).take(points.len()) {
            let cross = (a.x * b.y - b.x * a.y) as f64;
            twice_area += cross; mx += (a.x + b.x) as f64 * cross; my += (a.y + b.y) as f64 * cross;
            perimeter += ((a.x - b.x) as f64).hypot((a.y - b.y) as f64);
        }
        if !lock { if (500.0..=10000.0).contains(&perimeter) { chain += 1; } continue; }
        let area = twice_area.abs() / 2.0;
        let w = points.iter().map(|p| p.x).max().unwrap() - points.iter().map(|p| p.x).min().unwrap() + 1;
        let h = points.iter().map(|p| p.y).max().unwrap() - points.iter().map(|p| p.y).min().unwrap() + 1;
        if !(100.0..=1500.0).contains(&area) || !(0.8..=1.1).contains(&(w as f64 / h as f64)) { continue; }
        centroids.push(((mx / (3.0 * twice_area)) as i32, (my / (3.0 * twice_area)) as i32));
    }
    if !lock { return Ok(chain); }
    if centroids.len() > 1000 { return Ok(0); }
    if centroids.len() < 2 { return Ok(centroids.len()); }
    Ok(centroids.iter().enumerate().filter(|(i,a)| centroids.iter().enumerate().any(|(j,b)| *i != j && ((a.0 - b.0).pow(2) + (a.1 - b.1).pow(2)) <= 1)).count())
}
