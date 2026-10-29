//! 代理人培养标记只提供导航候选，不证明角色身份或关卡选中。
use crate::RecognitionImage;
use anyhow::{Context, Result, ensure};
use image::{GrayImage, Luma, RgbImage};
use imageproc::{
    contours::{BorderType, find_contours},
    geometry::contour_area,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMarkerArea {
    Training,
    Hunt,
    Simulation,
}
impl AgentMarkerArea {
    pub fn roi(self) -> [u32; 4] {
        match self {
            Self::Training => [1254, 376, 58, 518],
            Self::Hunt => [1254, 552, 58, 342],
            Self::Simulation => [128, 740, 1670, 36],
        }
    }
    pub fn offset(self) -> i32 {
        match self {
            Self::Training | Self::Hunt => -80,
            Self::Simulation => 80,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentMarker {
    pub bounds: [i32; 4],
    pub area_twice: u64,
    pub target: [i32; 2],
    pub fingerprint: String,
}
/// 与参考 OpenCV RGB→HSV 的 12 位整数缩放一致；低饱和、低色调背景取反。
pub fn marker_pixel([r, g, b]: [u8; 3]) -> bool {
    static TABLE: std::sync::OnceLock<([i32; 256], [i32; 256])> = std::sync::OnceLock::new();
    let (sdiv, hdiv) = TABLE.get_or_init(|| {
        let mut s = [0; 256];
        let mut h = [0; 256];
        for i in 1..256 {
            s[i] = ((255 * 4096) as f64 / i as f64).round() as i32;
            h[i] = ((30 * 4096) as f64 / i as f64).round() as i32;
        }
        (s, h)
    });
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let v = r.max(g).max(b);
    let diff = v - r.min(g).min(b);
    if diff == 0 {
        return false;
    }
    let saturation = (diff * sdiv[v as usize] + 2048) >> 12;
    let numerator = if v == r {
        g - b
    } else if v == g {
        b - r + 2 * diff
    } else {
        r - g + 4 * diff
    };
    let hue = (numerator * hdiv[diff as usize] + 2048) >> 12;
    let hue = if hue < 0 { hue + 180 } else { hue };
    hue > 10 || saturation > 10
}
fn detect(
    area: AgentMarkerArea,
    mut pixel: impl FnMut(u32, u32) -> [u8; 3],
) -> Result<Vec<AgentMarker>> {
    let [left, top, width, height] = area.roi();
    // imageproc 会将画面边缘当作背景；补一圈零以保留接触 ROI 边缘的轮廓。
    let mut mask = GrayImage::new(width + 2, height + 2);
    for y in 0..height {
        for x in 0..width {
            mask.put_pixel(
                x + 1,
                y + 1,
                Luma([if marker_pixel(pixel(left + x, top + y)) {
                    255
                } else {
                    0
                }]),
            );
        }
    }
    let mut result = Vec::new();
    // OpenCV RETR_EXTERNAL 从后扫描到的外轮廓开始返回；不采用孔洞或嵌套轮廓。
    for contour in find_contours::<i32>(&mask)
        .into_iter()
        .rev()
        .filter(|c| c.border_type == BorderType::Outer && c.parent.is_none())
    {
        let area_value = contour_area(&contour.points);
        if area_value < 800.0 {
            continue;
        }
        let min_x = contour
            .points
            .iter()
            .map(|p| p.x)
            .min()
            .context("培养标记轮廓为空")?;
        let min_y = contour.points.iter().map(|p| p.y).min().unwrap();
        let width = contour.points.iter().map(|p| p.x).max().unwrap() - min_x + 1;
        let height = contour.points.iter().map(|p| p.y).max().unwrap() - min_y + 1;
        let x = left as i32 + min_x - 1;
        let y = top as i32 + min_y - 1;
        let target = [x + width / 2, y + height / 2 + area.offset()];
        ensure!(
            (0..1920).contains(&target[0]) && (0..1080).contains(&target[1]),
            "培养标记偏移超出画面"
        );
        let mut hash = Sha256::new();
        hash.update(width.to_le_bytes());
        hash.update(height.to_le_bytes());
        for yy in y..y + height {
            for xx in x..x + width {
                hash.update(pixel(xx as u32, yy as u32));
            }
        }
        result.push(AgentMarker {
            bounds: [x, y, width, height],
            area_twice: (area_value * 2.0).round() as u64,
            target,
            fingerprint: hex::encode(hash.finalize()),
        });
    }
    Ok(result)
}
pub fn detect_agent_markers(image: &RgbImage, area: AgentMarkerArea) -> Result<Vec<AgentMarker>> {
    ensure!(
        image.dimensions() == (1920, 1080),
        "培养标记只接受 1080p 图像"
    );
    detect(area, |x, y| image.get_pixel(x, y).0)
}
pub fn inspect_agent_markers_file(
    path: &std::path::Path,
    area: AgentMarkerArea,
) -> Result<Vec<AgentMarker>> {
    od_core::config::reject_links(path)?;
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    ensure!(
        reader.into_dimensions()? == (1920, 1080),
        "培养标记只接受 1080p 图像"
    );
    detect_agent_markers(&image::open(path)?.to_rgb8(), area)
}
pub fn inspect_agent_markers(
    frame: &RecognitionImage,
    area: AgentMarkerArea,
) -> Result<Vec<AgentMarker>> {
    ensure!(
        frame.buffer.width() == 1920
            && frame.buffer.height() == 1080
            && frame.buffer.image_type() == 16,
        "培养标记图像格式不匹配"
    );
    let bgr = frame.buffer.raw_data().context("培养标记截图没有像素")?;
    ensure!(bgr.len() == 1920 * 1080 * 3, "培养标记截图像素长度不匹配");
    detect(area, |x, y| {
        let i = (y as usize * 1920 + x as usize) * 3;
        [bgr[i + 2], bgr[i + 1], bgr[i]]
    })
}
pub fn sources() -> Result<serde_json::Value> {
    Ok(serde_json::from_str(include_str!(
        "../../../resources/agent-plan/source-lock.json"
    ))?)
}
