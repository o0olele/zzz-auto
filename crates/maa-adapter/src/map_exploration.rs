//! 大地图图标与小地图追踪点识别。素材编译进程序，不依赖旧项目目录。
pub use crate::exploration_navigation::{RoadRoute, local_road_route, road_route_to};
use crate::{
    NativeSession, navigation_vision,
    suibian_adventure::{Word, point, words},
};
use anyhow::{Context, Result, ensure};
use image::{RgbImage, imageops::FilterType};
use od_core::{InputProbeAction, map_exploration::ExplorationTarget, world_patrol::PatrolPoint};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct MapIcon {
    pub center: [i32; 2],
    pub target: ExplorationTarget,
    pub score: f64,
    pub highlighted: bool,
}
struct IconTemplate {
    target: ExplorationTarget,
    width: u32,
    rows: Vec<(u64, u64)>,
    pixels: u32,
    probe: usize,
    selected: bool,
    dim: bool,
}
fn templates(mini: bool) -> &'static Vec<IconTemplate> {
    static TEMPLATES: OnceLock<Vec<IconTemplate>> = OnceLock::new();
    static MINI_TEMPLATES: OnceLock<Vec<IconTemplate>> = OnceLock::new();
    let cache = if mini { &MINI_TEMPLATES } else { &TEMPLATES };
    cache.get_or_init(|| {
        let mut out = vec![];
        for (target, bytes, widths, selected, dim) in [
            (
                ExplorationTarget::Observation,
                include_bytes!(
                    "../../../resources/source/map-exploration/observation-selected.png"
                )
                .as_slice(),
                &[44, 50, 54, 60][..],
                true,
                false,
            ),
            (
                ExplorationTarget::Observation,
                if mini {
                    include_bytes!("../../../resources/source/map-exploration/observation-mini.png")
                        .as_slice()
                } else {
                    include_bytes!("../../../resources/source/map-exploration/observation.png")
                        .as_slice()
                },
                if mini {
                    &[22, 24, 26, 28, 30, 32, 34, 36][..]
                } else {
                    &[28, 36, 44, 52, 60][..]
                },
                false,
                false,
            ),
            (
                ExplorationTarget::Cargo,
                include_bytes!("../../../resources/source/map-exploration/cargo.png").as_slice(),
                if mini {
                    &[18, 20, 22, 24, 26, 28, 30, 32][..]
                } else {
                    &[24, 28, 32, 36, 40, 44, 48, 52, 56, 60][..]
                },
                false,
                false,
            ),
            (
                ExplorationTarget::Observation,
                include_bytes!("../../../resources/source/map-exploration/observation-minimum.png")
                    .as_slice(),
                &[40, 44, 49, 53, 55, 57][..],
                false,
                true,
            ),
            (
                ExplorationTarget::Cargo,
                include_bytes!("../../../resources/source/map-exploration/cargo-minimum.png")
                    .as_slice(),
                &[35, 49][..],
                false,
                false,
            ),
        ] {
            if mini && (selected || dim) {
                continue;
            }
            let source = image::load_from_memory(bytes)
                .expect("内置地图图标无效")
                .to_rgba8();
            for &width in widths {
                let height = source.height() * width / source.width();
                let image = image::imageops::resize(&source, width, height, FilterType::Triangle);
                let mut rows = vec![];
                let mut pixels = 0;
                for y in 0..height {
                    let (mut bright, mut dark) = (0u64, 0u64);
                    for x in 0..width {
                        let pixel = image.get_pixel(x, y).0;
                        if pixel[3] < 250 {
                            continue;
                        }
                        let max = *pixel[..3].iter().max().unwrap();
                        if max
                            >= if selected {
                                185
                            } else if dim {
                                105
                            } else {
                                155
                            }
                        {
                            bright |= 1 << x;
                        }
                        if max
                            <= if selected {
                                165
                            } else if dim {
                                95
                            } else {
                                65
                            }
                        {
                            dark |= 1 << x;
                        }
                    }
                    pixels += bright.count_ones() + dark.count_ones();
                    rows.push((bright, dark));
                }
                let probe = rows
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, (b, _))| b.count_ones())
                    .unwrap()
                    .0;
                out.push(IconTemplate {
                    target,
                    width,
                    rows,
                    pixels,
                    probe,
                    selected,
                    dim,
                });
            }
        }
        out
    })
}

/// 位图逐行匹配同时约束亮色主体与深色轮廓，避免把道路或橙色战斗标记当作目标。
pub fn map_icons(image: &RgbImage, target: ExplorationTarget) -> Vec<MapIcon> {
    let mini = image.dimensions() == (210, 210);
    if !mini && image.dimensions() != (1920, 1080) {
        return vec![];
    }
    let stride = (image.width() as usize).div_ceil(64) + 1;
    let mut bright = vec![0u64; stride * image.height() as usize];
    let mut dark = bright.clone();
    let mut selected_bright = bright.clone();
    let mut selected_dark = bright.clone();
    let mut dim_bright = bright.clone();
    let mut dim_dark = bright.clone();
    for (x, y, pixel) in image.enumerate_pixels() {
        let at = y as usize * stride + x as usize / 64;
        let value = *pixel.0.iter().max().unwrap();
        if value >= 155 {
            bright[at] |= 1 << (x % 64);
        }
        if value <= 65 {
            dark[at] |= 1 << (x % 64);
        }
        if value >= 185 {
            selected_bright[at] |= 1 << (x % 64);
        }
        if value <= 165 {
            selected_dark[at] |= 1 << (x % 64);
        }
        if value >= 105 {
            dim_bright[at] |= 1 << (x % 64);
        }
        if value <= 95 {
            dim_dark[at] |= 1 << (x % 64);
        }
    }
    let read = |data: &[u64], x: usize, y: usize| {
        let at = y * stride + x / 64;
        let offset = x % 64;
        (data[at] >> offset)
            | if offset == 0 {
                0
            } else {
                data[at + 1] << (64 - offset)
            }
    };
    let mut candidates = vec![];
    for template in templates(mini).iter().filter(|t| t.target == target) {
        let (bright, dark) = if template.selected {
            (&selected_bright, &selected_dark)
        } else if template.dim {
            (&dim_bright, &dim_dark)
        } else {
            (&bright, &dark)
        };
        let allowed = (f64::from(template.pixels)
            * if mini && target == ExplorationTarget::Observation {
                0.14
            } else if template.dim {
                0.15
            } else {
                0.105
            }) as u32;
        let [left, top, right, bottom] = if mini {
            [3, 3, 207, 207]
        } else {
            [65, 130, 1370, 950]
        };
        for y in top..bottom - template.rows.len() {
            for x in left..right - template.width as usize {
                let (b, d) = template.rows[template.probe];
                let errors = (b & !read(&bright, x, y + template.probe)).count_ones()
                    + (d & !read(&dark, x, y + template.probe)).count_ones();
                // 单行误差是整图误差的下界，必须沿用整图预算。
                // 固定 6 像素会误杀道路背景变化、整图仍合格的灰色奇点。
                if errors > allowed {
                    continue;
                }
                let mut errors = 0;
                for (dy, &(b, d)) in template.rows.iter().enumerate() {
                    errors += (b & !read(&bright, x, y + dy)).count_ones()
                        + (d & !read(&dark, x, y + dy)).count_ones();
                    if errors > allowed {
                        break;
                    }
                }
                if errors <= allowed {
                    candidates.push(MapIcon {
                        center: [
                            x as i32 + template.width as i32 / 2,
                            y as i32 + template.rows.len() as i32 / 2,
                        ],
                        target,
                        score: 1.0 - f64::from(errors) / f64::from(template.pixels),
                        highlighted: template.selected,
                    });
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut out: Vec<MapIcon> = vec![];
    for icon in candidates {
        if !out.iter().any(|old| {
            (old.center[0] - icon.center[0]).pow(2) + (old.center[1] - icon.center[1]).pow(2)
                < if mini { 16 * 16 } else { 32 * 32 }
        }) {
            out.push(icon);
            if out.len() == 64 {
                break;
            }
        }
    }
    out
}

/// 返回玩家附近各个独立方向的绿点路径，不凭最近的一颗点决定方向。
pub fn guide_routes(rgb: &RgbImage) -> Vec<PatrolPoint> {
    if rgb.dimensions() != (210, 210) {
        return vec![];
    }
    let mut mask = vec![false; 210 * 210];
    for (x, y, pixel) in rgb.enumerate_pixels() {
        let [r, g, b] = pixel.0.map(u32::from);
        if (x as i32 - 105).pow(2) + (y as i32 - 105).pow(2) < 94 * 94
            && g >= 135
            && g * 100 > r * 112
            && g * 100 > b * 150
        {
            mask[y as usize * 210 + x as usize] = true;
        }
    }
    let mut dots = vec![];
    for index in 0..mask.len() {
        if !mask[index] {
            continue;
        }
        let mut queue = VecDeque::from([index]);
        mask[index] = false;
        let (mut count, mut sx, mut sy) = (0, 0, 0);
        let (mut minx, mut maxx, mut miny, mut maxy) = (210, 0, 210, 0);
        while let Some(at) = queue.pop_front() {
            let (x, y) = (at % 210, at / 210);
            count += 1;
            sx += x;
            sy += y;
            minx = minx.min(x);
            maxx = maxx.max(x);
            miny = miny.min(y);
            maxy = maxy.max(y);
            for yy in y.saturating_sub(1)..=(y + 1).min(209) {
                for xx in x.saturating_sub(1)..=(x + 1).min(209) {
                    if mask[yy * 210 + xx] {
                        mask[yy * 210 + xx] = false;
                        queue.push_back(yy * 210 + xx);
                    }
                }
            }
        }
        let (width, height) = (maxx - minx + 1, maxy - miny + 1);
        if (4..=100).contains(&count)
            && (2..=13).contains(&width)
            && (2..=13).contains(&height)
            && width.max(height) <= width.min(height) * 2
        {
            dots.push(PatrolPoint {
                x: (sx / count) as i32,
                y: (sy / count) as i32,
            });
        }
    }
    let origin = PatrolPoint { x: 105, y: 105 };
    let mut routes = vec![];
    dots.sort_by(|a, b| a.distance(origin).total_cmp(&b.distance(origin)));
    for seed in dots
        .iter()
        .filter(|p| (8.0..=38.0).contains(&p.distance(origin)))
    {
        let mut current = *seed;
        let mut connected = 1;
        // 最近的独立图标不能遮蔽后面的引导线；逐个验证近处候选能否连成路线。
        // 首颗点被朝向光锥染黄时，允许从 30 像素外的首颗绿点继续连接。
        while connected < 2 || current.distance(origin) < 30.0 {
            let next = dots
                .iter()
                .filter(|p| {
                    p.distance(origin) > current.distance(origin) + 0.5
                        && p.distance(current) <= 20.0
                })
                .min_by(|a, b| a.distance(current).total_cmp(&b.distance(current)));
            let Some(next) = next else {
                break;
            };
            current = *next;
            connected += 1;
        }
        if connected >= 2 {
            let bearing = origin.heading(current);
            if !routes.iter().any(|point| {
                let delta = (origin.heading(*point) - bearing + 180.0).rem_euclid(360.0) - 180.0;
                delta.abs() < 45.0
            }) {
                routes.push(current);
            }
        }
    }
    routes
}

/// 只有单一方向的引导线可直接跟随；双向或分叉异常交给道路定位。
pub fn guide_point(rgb: &RgbImage) -> Option<PatrolPoint> {
    let routes = guide_routes(rgb);
    let [point] = routes.as_slice() else {
        return None;
    };
    Some(*point)
}

pub struct ExplorationSight {
    texts: Vec<Word>,
    pub map_open: bool,
    pub icons: Vec<MapIcon>,
    pub mini_map: bool,
    pub world_controls: bool,
    pub interaction_f_key: bool,
    pub map_button: Option<[i32; 2]>,
    pub navigation_overview: bool,
    pub heading: Option<f64>,
    pub guide: Option<PatrolPoint>,
    pub guide_ambiguous: bool,
    pub road_route: Option<RoadRoute>,
    pub world_marker: Option<WorldMarker>,
    pub mini_frame: Option<RgbImage>,
    pub close_details: Option<[i32; 2]>,
    pub zoom: Option<ZoomControl>,
    pub cargo_count: Option<[u16; 2]>,
    /// 保留本次同帧地图，供有限的本地识别诊断使用。
    pub map_frame: Option<RgbImage>,
    captured: Instant,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct WorldMarker {
    pub point: [i32; 2],
    pub meters: f64,
}

/// 距离文字旁必须同时命中观测奇点同心环，不能把任意任务距离当作目标。
pub fn observation_world_marker(rgb: &RgbImage, distance: [i32; 2]) -> Option<[i32; 2]> {
    if rgb.dimensions() != (1920, 1080) {
        return None;
    }
    type MarkerMask = (u32, Vec<(u32, u32, bool)>);
    static MASKS: OnceLock<Vec<MarkerMask>> = OnceLock::new();
    let masks = MASKS.get_or_init(|| {
        let source = image::load_from_memory(include_bytes!(
            "../../../resources/source/map-exploration/observation-world-marker.png"
        ))
        .expect("世界观测奇点标记无效")
        .to_rgba8();
        [36, 40, 44, 48]
            .into_iter()
            .map(|size| {
                let scaled = image::imageops::resize(&source, size, size, FilterType::Triangle);
                let pixels = scaled
                    .enumerate_pixels()
                    .filter_map(|(x, y, p)| {
                        let min = *p.0[..3].iter().min().unwrap();
                        let max = *p.0[..3].iter().max().unwrap();
                        (p[3] >= 250 && (min >= 165 || max <= 90)).then_some((x, y, min >= 165))
                    })
                    .collect();
                (size, pixels)
            })
            .collect()
    });
    let mut candidates = vec![];
    for (size, mask) in masks {
        let allowed = mask.len() / 8;
        for cy in (distance[1] - 22).max(230)..=(distance[1] + 10).min(950) {
            // OCR 有时把圆环并入距离文本框，中心会向标记一侧偏移。
            for cx in (distance[0] - 100).max(320)..=(distance[0] - 10).min(1870) {
                let left = cx - *size as i32 / 2;
                let top = cy - *size as i32 / 2;
                let mut errors = 0;
                for &(x, y, bright) in mask {
                    let p = rgb.get_pixel(left as u32 + x, top as u32 + y).0;
                    let min = *p.iter().min().unwrap();
                    let max = *p.iter().max().unwrap();
                    errors += usize::from(if bright {
                        min < 140 || max - min > 65
                    } else {
                        max > 120
                    });
                    if errors > allowed {
                        break;
                    }
                }
                if errors <= allowed {
                    candidates.push((errors as f64 / mask.len() as f64, [cx, cy]));
                }
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (_, best) = *candidates.first()?;
    candidates
        .iter()
        .all(|(_, p)| (p[0] - best[0]).abs() < 16 && (p[1] - best[1]).abs() < 16)
        .then_some(best)
}

/// 目标选中后，右下角普通攻击槽会变成 F 交互；该字形常被整帧 OCR 漏掉。
pub fn interaction_f_key(rgb: &RgbImage) -> bool {
    if rgb.dimensions() != (1920, 1080) {
        return false;
    }
    static MASK: OnceLock<Vec<(u32, u32, bool)>> = OnceLock::new();
    let mask = MASK.get_or_init(|| {
        let source = image::load_from_memory(include_bytes!(
            "../../../resources/source/map-exploration/interaction-f-key.png"
        ))
        .expect("F 交互图标无效")
        .to_rgba8();
        source
            .enumerate_pixels()
            .filter(|(_, _, p)| p[3] >= 250)
            .map(|(x, y, p)| (x, y, *p.0[..3].iter().min().unwrap() >= 175))
            .collect()
    });
    let white = mask.iter().filter(|(_, _, bright)| *bright).count();
    if white < 15 {
        return false;
    }
    for top in 1009..=1019 {
        for left in 1433..=1443 {
            let (mut missed_white, mut missed_black) = (0, 0);
            for &(x, y, bright) in mask {
                let p = rgb.get_pixel(left + x, top + y).0;
                let min = *p.iter().min().unwrap();
                let max = *p.iter().max().unwrap();
                if bright {
                    missed_white += usize::from(min < 150 || max - min > 60);
                } else {
                    missed_black += usize::from(max > 110);
                }
                if missed_white > white / 10 + 1 || missed_black > (mask.len() - white) / 12 + 1 {
                    break;
                }
            }
            if missed_white <= white / 10 + 1 && missed_black <= (mask.len() - white) / 12 + 1 {
                return true;
            }
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrivalStep {
    Waiting,
    Turned,
    Faced,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct ZoomControl {
    pub knob: [i32; 2],
    pub minimum: [i32; 2],
    pub at_minimum: bool,
}

/// 缩放滑块位于底部固定栏；圆形内圈与黑色外圈同时成立才提供拖动位置。
pub fn zoom_control(rgb: &RgbImage) -> Option<ZoomControl> {
    if rgb.dimensions() != (1920, 1080) {
        return None;
    }
    let inner = [
        (0, -7),
        (5, -5),
        (7, 0),
        (5, 5),
        (0, 7),
        (-5, 5),
        (-7, 0),
        (-5, -5),
    ];
    let outer = [
        (0, -13),
        (9, -9),
        (13, 0),
        (9, 9),
        (0, 13),
        (-9, 9),
        (-13, 0),
        (-9, -9),
    ];
    let mut best = None;
    for y in 1003i32..1062 {
        for x in 824i32..1092 {
            let inside = inner
                .iter()
                .filter(|&&(dx, dy)| {
                    let p = rgb.get_pixel((x + dx) as u32, (y + dy) as u32).0;
                    let max = *p.iter().max().unwrap();
                    let min = *p.iter().min().unwrap();
                    (65..=235).contains(&max) && max - min <= 25
                })
                .count();
            if inside < 6 {
                continue;
            }
            let outside = outer
                .iter()
                .filter(|&&(dx, dy)| {
                    rgb.get_pixel((x + dx) as u32, (y + dy) as u32)
                        .0
                        .iter()
                        .all(|v| *v <= 90)
                })
                .count();
            let score = inside + outside;
            if outside >= 6 && score >= 14 && best.is_none_or(|(old, _, _)| score > old) {
                best = Some((score, x, y));
            }
        }
    }
    best.map(|(_, x, y)| ZoomControl {
        knob: [x, y],
        minimum: [834, y],
        at_minimum: x <= 841,
    })
}

fn cargo_count(texts: &[Word]) -> Option<[u16; 2]> {
    let mut words: Vec<_> = texts
        .iter()
        .filter(|word| word.inside([1440, 250, 1545, 325]))
        .collect();
    words.sort_by_key(|word| word.center()[0]);
    let text: String = words
        .iter()
        .flat_map(|word| word.text.chars())
        .filter(|c| c.is_ascii_digit() || *c == '/')
        .collect();
    let (current, total) = text.split_once('/')?;
    let current: u16 = current.parse().ok()?;
    let total: u16 = total.parse().ok()?;
    (current <= total && total > 0 && total <= 200).then_some([current, total])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_counter_uses_left_car_counter_and_ignores_gold_blocks() {
        let words = [
            Word {
                text: "09/17".into(),
                rect: [1450, 273, 74, 28],
            },
            Word {
                text: "01/02".into(),
                rect: [1593, 273, 77, 28],
            },
        ];
        assert_eq!(cargo_count(&words), Some([9, 17]));
        assert_eq!(cargo_count(&words[1..]), None);
        let full = [Word {
            text: "17/17".into(),
            rect: [1450, 273, 74, 28],
        }];
        assert_eq!(cargo_count(&full), Some([17, 17]));
    }
}
impl ExplorationSight {
    pub fn script_observation(&self, target: ExplorationTarget) -> serde_json::Value {
        use base64::Engine as _;
        json!({"fresh":self.fresh(),"world_controls":self.world_controls,"map_open":self.map_open,
            "mini_map":self.mini_map,"heading":self.heading,"guide":self.guide,
            "guide_ambiguous":self.guide_ambiguous,"road_route":self.road_route,"world_marker":self.world_marker,
            "arrival_point":self.arrival_point(target),"interact":self.interact(target),
            "mini_rgb_b64":self.mini_frame.as_ref().map(|mini|base64::engine::general_purpose::STANDARD.encode(mini.as_raw()))})
    }
    pub fn target_label(&self, target: ExplorationTarget) -> Option<[i32; 2]> {
        self.texts
            .iter()
            .filter(|word| word.inside([500, 200, 1920, 980]))
            .filter(|word| {
                let text: String = word.text.chars().filter(|c| !c.is_whitespace()).collect();
                text.contains(target.name())
                    || (word.rect[0] + word.rect[2] >= 1900
                        && text.chars().count() >= 3
                        && target.name().starts_with(&text))
            })
            .min_by_key(|word| (word.center()[0] - 960).abs())
            .map(Word::center)
    }
    pub fn target_distance(&self) -> Option<(f64, [i32; 2])> {
        let mut matches = self
            .texts
            .iter()
            .filter(|word| word.inside([300, 180, 1920, 1000]))
            .filter_map(|word| {
                let text: String = word
                    .text
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .map(|c| {
                        if matches!(c, '．' | ',' | '，' | ':' | '：') {
                            '.'
                        } else {
                            c
                        }
                    })
                    .collect();
                let number = text
                    .strip_suffix('m')
                    .or_else(|| text.strip_suffix('M'))
                    .or_else(|| text.strip_suffix('米'))?;
                if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    return None;
                }
                let meters: f64 = number.parse().ok()?;
                (meters.is_finite() && (0.0..=2000.0).contains(&meters))
                    .then_some((meters, word.center()))
            });
        let found = matches.next()?;
        matches.next().is_none().then_some(found)
    }
    pub fn arrival_point(&self, target: ExplorationTarget) -> Option<[i32; 2]> {
        if self.map_open || !self.world_controls {
            return None;
        }
        let distance = self.target_distance();
        if distance.is_some_and(|(meters, _)| meters > 3.0) {
            return None;
        }
        self.target_label(target)
            .or_else(|| distance.map(|(_, point)| [(point[0] + 42).min(1910), point[1]]))
    }
    pub fn non_combat_ui(&self) -> bool {
        self.map_open
            || self.navigation_overview
            || self.nearby_transport_button(false).is_some()
            || self.world_controls
            || (self.mini_map
                && ["T", "F1", "F2", "F3", "F4"]
                    .iter()
                    .filter(|label| self.has(label, [1300, 0, 1910, 210]))
                    .count()
                    >= 3)
    }
    pub fn point(&self, text: &str, rect: [i32; 4]) -> Option<[i32; 2]> {
        point(&self.texts, text, rect)
    }
    pub fn has(&self, text: &str, rect: [i32; 4]) -> bool {
        self.texts
            .iter()
            .any(|w| w.inside(rect) && w.text.contains(text))
    }
    pub fn selected(&self, target: ExplorationTarget) -> bool {
        self.has(target.name(), [1380, 115, 1860, 220])
    }
    pub fn locked(&self) -> bool {
        self.has("未解锁", [1390, 340, 1860, 880])
    }
    pub fn track_button(&self) -> Option<[i32; 2]> {
        ["追踪", "开始追踪"]
            .iter()
            .find_map(|s| self.point(s, [1400, 850, 1830, 990]))
    }
    pub fn tracking(&self) -> bool {
        self.has("停止追踪", [1400, 850, 1830, 990])
    }
    pub fn stop_tracking_button(&self) -> Option<[i32; 2]> {
        self.point("停止追踪", [1400, 850, 1830, 990])
    }
    pub fn nearby_transport_button(&self, confirm: bool) -> Option<[i32; 2]> {
        // 只处理追踪后的这一种弹窗，不能泛点其他界面的确认按钮。
        if !self.has("已开始追踪", [500, 400, 1450, 620])
            || !self.has("传送至附近", [500, 400, 1450, 620])
        {
            return None;
        }
        self.point(
            if confirm { "确认" } else { "取消" },
            if confirm {
                [980, 570, 1280, 710]
            } else {
                [650, 570, 950, 710]
            },
        )
    }
    pub fn unlocked(&self) -> bool {
        !self.locked()
            && (["前往", "传送"]
                .iter()
                .any(|s| self.has(s, [1400, 850, 1830, 990]))
                || ["已解锁", "已激活", "已开启"]
                    .iter()
                    .any(|s| self.has(s, [1390, 340, 1860, 880])))
    }
    pub fn interact(&self, target: ExplorationTarget) -> bool {
        !self.map_open
            && self.world_controls
            && self.target_label(target).is_some()
            && (self.point("F", [500, 200, 1920, 980]).is_some()
                || self.interaction_f_key
                || self.point("F", [1370, 970, 1530, 1080]).is_some()
                || self.texts.iter().any(|word| {
                    word.inside([500, 200, 1920, 980])
                        && word.text.replace(' ', "").starts_with('F')
                        && word.text.contains(target.name())
                })
                || ["解锁", "激活", "开启", "打开", "交互"]
                    .iter()
                    .any(|s| self.has(s, [1100, 300, 1830, 900])))
    }
    pub fn reward(&self, target: ExplorationTarget) -> bool {
        let texts: &[&str] = match target {
            ExplorationTarget::Observation => {
                &["解锁成功", "激活成功", "观测奇点已解锁", "观测奇点已激活"]
            }
            ExplorationTarget::Cargo => &["获得物品", "获得奖励", "开启成功"],
        };
        !self.map_open
            && (texts.iter().any(|s| self.has(s, [300, 100, 1650, 900]))
                || (target == ExplorationTarget::Cargo
                    && self.point("获得", [0, 100, 1650, 1000]).is_some()))
    }
    pub fn fresh(&self) -> bool {
        self.captured.elapsed() <= Duration::from_millis(1500)
    }
}

fn navigation_overview(texts: &[Word], back: bool, local_map: bool) -> bool {
    back && !local_map
        && [
            "六分街",
            "澄辉坪",
            "录像店",
            "布亚斯特城区",
            "维尔乌姆城区",
            "航天城",
            "斯卡莫空洞",
            "光映广场",
        ]
        .iter()
        .filter(|name| texts.iter().any(|word| word.text.contains(**name)))
        .count()
            >= 3
}

impl NativeSession {
    pub fn inspect_map_exploration_script(
        &self,
        target: ExplorationTarget,
    ) -> Result<serde_json::Value> {
        let sight = self
            .read_map_exploration_with_guide_policy(target, false, &AtomicBool::new(false))?
            .context("Python 空洞探索截图读取超时")?;
        Ok(sight.script_observation(target))
    }
    pub fn inspect_map_exploration(&self, target: ExplorationTarget) -> Result<serde_json::Value> {
        let sight = self
            .read_map_exploration(target, &AtomicBool::new(false))?
            .context("空洞探索截图读取超时")?;
        Ok(
            json!({"map_open":sight.map_open, "icons":sight.icons, "mini_map":sight.mini_map,
            "map_button":sight.map_button, "navigation_overview":sight.navigation_overview,
            "world_controls":sight.world_controls,
            "interaction_f_key":sight.interaction_f_key,
            "zoom":sight.zoom, "cargo_count":sight.cargo_count,
            "heading":sight.heading, "guide":sight.guide, "road_route":sight.road_route, "selected":sight.selected(target),
            "guide_ambiguous":sight.guide_ambiguous,
            "world_marker":sight.world_marker,
            "target_label":sight.target_label(target), "target_distance":sight.target_distance(),
            "arrival_point":sight.arrival_point(target),
            "locked":sight.locked(), "tracking":sight.tracking(), "track_button":sight.track_button(),
            "close_details":sight.close_details, "unlocked":sight.unlocked(),
            "nearby_transport_confirm":sight.nearby_transport_button(true),
            "nearby_transport_cancel":sight.nearby_transport_button(false),
            "non_combat_ui":sight.non_combat_ui(),
            "interact":sight.interact(target), "reward":sight.reward(target), "fresh":sight.fresh()}),
        )
    }
    pub fn exploration_battle_ready(
        &self,
        recognizer: &mut crate::combat_vision::CombatRecognizer,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        let Some(mut frame) = self.combat_frame(cancel)? else {
            return Ok(false);
        };
        if self.patrol_world_in_combat(&frame)? {
            return Ok(false);
        }
        let ready = recognizer.align_patrol_frame(self, &mut frame, cancel)?;
        Ok(ready && frame.age_ms() <= 1000 && !cancel.load(Ordering::Acquire))
    }
    pub fn read_map_exploration(
        &self,
        target: ExplorationTarget,
        cancel: &AtomicBool,
    ) -> Result<Option<ExplorationSight>> {
        self.read_map_exploration_with_guide_policy(target, true, cancel)
    }
    pub fn read_map_exploration_with_guide_policy(
        &self,
        target: ExplorationTarget,
        trust_guide: bool,
        cancel: &AtomicBool,
    ) -> Result<Option<ExplorationSight>> {
        let captured = Instant::now();
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let text = words(
            &self
                .recognize_prepared(
                    &frame.image,
                    "OCR",
                    &json!({"model":"ppocrv5", "threshold":0.6}),
                )?
                .detail,
        )?;
        let bgr = frame
            .image
            .buffer
            .raw_data()
            .context("空洞探索截图无像素")?;
        ensure!(
            bgr.len() == 1920 * 1080 * 3,
            "空洞探索需要 1920×1080 客户区"
        );
        let rgb = RgbImage::from_fn(1920, 1080, |x, y| {
            let at = ((y * 1920 + x) * 3) as usize;
            image::Rgb([bgr[at + 2], bgr[at + 1], bgr[at]])
        });
        let back =
            self.patrol_match(&frame.image, "menu/back", [20, 0, 180, 130], 0.7)?["hit"] == true;
        let map_open = point(&text, "快捷导航", [1300, 0, 1740, 125]).is_some()
            && !text.iter().any(|word| {
                word.inside([1240, 10, 1510, 120])
                    && crate::world_patrol::filter_title_matches(&word.text)
            })
            && back;
        let navigation_overview = navigation_overview(&text, back, map_open);
        let mut mini = None;
        let mut map_button = None;
        let mut map_control_visible = false;
        if !map_open {
            let matched =
                self.patrol_match(&frame.image, "normal_world/map", [1696, 319, 162, 150], 0.5)?;
            map_control_visible = matched["hit"] == true;
            if matched["hit"] == true {
                let [x, y, _, _]: [i32; 4] =
                    serde_json::from_value(matched["detail"]["best"]["box"].clone())?;
                let (x, y) = (x - 169, y - 151);
                if x >= 0 && y >= 0 && x + 210 <= 1920 && y + 210 <= 1080 {
                    let crop =
                        image::imageops::crop_imm(&rgb, x as u32, y as u32, 210, 210).to_image();
                    if navigation_vision::player_found(&crop) {
                        mini = Some(crop);
                        map_button = Some([x + 105, y + 105]);
                    }
                }
            }
            if mini.is_none() {
                let crop = image::imageops::crop_imm(&rgb, 1534, 201, 210, 210).to_image();
                if navigation_vision::player_found(&crop) {
                    mini = Some(crop);
                    map_button = Some([1639, 306]);
                }
            }
        }
        // 使用已经带固定区域回退的小地图结果；旧跑图探测器若动态匹配
        // 命中错误位置，会跳过固定区域回退，不能用来排除探索 HUD。
        let world_controls = (mini.is_some() || map_control_visible)
            && (self.patrol_match(
                &frame.image,
                "normal_world_investigation/function_menu",
                [1755, 20, 150, 145],
                0.8,
            )?["hit"]
                == true
                || self.patrol_match(
                    &frame.image,
                    "normal_world/function_menu",
                    [1755, 20, 150, 145],
                    0.8,
                )?["hit"]
                    == true);
        let heading = mini
            .as_ref()
            .map(navigation_vision::heading)
            .transpose()?
            .flatten()
            .map(|h| h.degrees);
        let interaction_f_key = world_controls && interaction_f_key(&rgb);
        let routes = mini.as_ref().map(guide_routes).unwrap_or_default();
        let guide_ambiguous = routes.len() > 1;
        let guide = if let [point] = routes.as_slice()
            && trust_guide
        {
            Some(*point)
        } else {
            None
        };
        let road_route = if guide.is_none() && world_controls && heading.is_some() {
            mini.as_ref()
                .and_then(|mini| local_road_route(mini, target))
        } else {
            None
        };
        let icons = if map_open {
            map_icons(&rgb, target)
        } else {
            vec![]
        };
        let zoom = map_open.then(|| zoom_control(&rgb)).flatten();
        let cargo_count = map_open.then(|| cargo_count(&text)).flatten();
        let close_details = if map_open {
            let matched = self.patrol_match(
                &frame.image,
                "normal_world_investigation/btn_area_info_close",
                [1760, 100, 110, 120],
                0.7,
            )?;
            if matched["hit"] == true {
                let [x, y, w, h]: [i32; 4] =
                    serde_json::from_value(matched["detail"]["best"]["box"].clone())?;
                Some([x + w / 2, y + h / 2])
            } else {
                None
            }
        } else {
            None
        };
        if cancel.load(Ordering::Acquire) || captured.elapsed() > Duration::from_millis(1500) {
            return Ok(None);
        }
        let mut sight = ExplorationSight {
            texts: text,
            map_open,
            icons,
            mini_map: mini.is_some(),
            world_controls,
            interaction_f_key,
            map_button,
            navigation_overview,
            heading,
            guide,
            guide_ambiguous,
            road_route,
            world_marker: None,
            mini_frame: None,
            close_details,
            zoom,
            cargo_count,
            map_frame: None,
            captured,
        };
        if !map_open
            && world_controls
            && guide.is_none()
            && sight.road_route.is_none()
            && target == ExplorationTarget::Observation
            && mini
                .as_ref()
                .is_some_and(|mini| map_icons(mini, target).len() == 1)
            && let Some((meters, distance)) = sight.target_distance()
            && (3.0..=35.0).contains(&meters)
            && let Some(point) = observation_world_marker(&rgb, distance)
        {
            sight.world_marker = Some(WorldMarker { point, meters });
        }
        sight.map_frame = map_open.then_some(rgb);
        sight.mini_frame = mini;
        if cancel.load(Ordering::Acquire) || !sight.fresh() {
            return Ok(None);
        }
        Ok(Some(sight))
    }
    /// 小地图不可用时只接受经过同帧核对的近距世界目标；对准后持续前进。
    pub fn exploration_walk_marker(
        &mut self,
        sight: &ExplorationSight,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        let result = (|| {
            if !sight.fresh() || !sight.world_controls || cancel.load(Ordering::Acquire) {
                return Ok(false);
            }
            let Some(marker) = sight.world_marker else {
                return Ok(false);
            };
            ensure!(!self.tasker.running(), "空洞探索移动前须等待 Pipeline 收尾");
            if let Some(window) = &self.game_window {
                od_platform::check_keyboard_target(window, true)?;
            }
            let delta = marker.point[0] - 960;
            if delta.abs() > 140 {
                self.release_inputs()?;
                self.combat_turn(
                    (f64::from(delta) * 0.35).round().clamp(-180.0, 180.0) as i32,
                    cancel,
                )?;
                return Ok(false);
            }
            if !self.patrol_is_walking() {
                self.combat_input(InputProbeAction::Key { key: 87 }, true, cancel)?;
            }
            Ok(true)
        })();
        match result {
            Ok(true) if !cancel.load(Ordering::Acquire) => Ok(true),
            Ok(_) => self.release_inputs().map(|()| false),
            Err(error) => {
                self.release_inputs()?;
                Err(error)
            }
        }
    }
    pub fn exploration_walk(
        &mut self,
        sight: &ExplorationSight,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let result = (|| {
            if !sight.fresh() || !sight.mini_map || cancel.load(Ordering::Acquire) {
                self.reset_patrol_steering();
                return Ok(false);
            }
            let point = sight
                .guide
                .or_else(|| sight.road_route.as_ref().map(|route| route.waypoint));
            let Some((guide, heading)) = point.zip(sight.heading) else {
                self.reset_patrol_steering();
                return Ok(false);
            };
            ensure!(!self.tasker.running(), "空洞探索移动前须等待 Pipeline 收尾");
            if let Some(window) = &self.game_window {
                od_platform::check_keyboard_target(window, true)?;
            }
            let origin = PatrolPoint { x: 105, y: 105 };
            let steering = self
                .patrol_steering
                .observe(heading, origin.heading(guide), origin.distance(guide))
                .map_err(anyhow::Error::msg)?;
            if !steering.forward {
                self.release_inputs()?;
            }
            if let Some(dx) = steering.dx {
                self.combat_turn(dx, cancel)?;
            }
            if steering.forward && !cancel.load(Ordering::Acquire) {
                if !self.patrol_is_walking() {
                    self.combat_input(InputProbeAction::Key { key: 87 }, true, cancel)?;
                }
            }
            Ok(steering.forward)
        })();
        match result {
            Ok(true) if !cancel.load(Ordering::Acquire) => Ok(()),
            Ok(_) => self.release_inputs(),
            Err(error) => {
                self.reset_patrol_steering();
                match self.release_inputs() {
                    Ok(()) => Err(error),
                    Err(cleanup) => {
                        Err(anyhow::anyhow!("{error:#}；移动输入收尾失败：{cleanup:#}"))
                    }
                }
            }
        }
    }
    /// 到点后先校正镜头，再短按前进改变人物朝向；不凭距离直接按 F。
    pub fn exploration_face_target(
        &mut self,
        sight: &ExplorationSight,
        target: ExplorationTarget,
        cancel: &AtomicBool,
    ) -> Result<ArrivalStep> {
        let result = (|| {
            if cancel.load(Ordering::Acquire) || !sight.fresh() {
                return Ok(ArrivalStep::Waiting);
            }
            let Some(point) = sight.arrival_point(target) else {
                return Ok(ArrivalStep::Waiting);
            };
            self.release_inputs()?;
            let delta = point[0] - 960;
            if delta.abs() > 140 {
                self.combat_turn(
                    (f64::from(delta) * 0.35).round().clamp(-180.0, 180.0) as i32,
                    cancel,
                )?;
                return Ok(ArrivalStep::Turned);
            }
            self.combat_input(InputProbeAction::Key { key: 87 }, true, cancel)?;
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(80) && !cancel.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(ArrivalStep::Faced)
        })();
        let release = self.release_inputs();
        result.and_then(|step| release.map(|()| step))
    }
}
