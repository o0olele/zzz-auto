//! 3D 地图与小地图的同帧读取。所有坐标在当前截图内验证，输入共用 Maa 清理。
use crate::{
    NativeSession, RecognitionImage, navigation_vision,
    suibian_adventure::{Word, point, words},
};
use anyhow::{Context, Result, ensure};
use image::{GrayImage, RgbImage};
use od_core::{
    InputProbeAction, MouseButton,
    world_patrol::{PatrolArea, PatrolPoint},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const AREA_LIST_RECT: [i32; 4] = [1402, 330, 1852, 952];
const AREA_TITLE_RECT: [i32; 4] = [135, 5, 1300, 60];
pub const MAX_AREA_SELECTION_INPUTS: usize = 20;
pub const MAX_FILTER_INPUTS: usize = 3;
pub const MAX_DETAIL_CLOSE_INPUTS: usize = 3;
pub const MAX_SEARCH_ROUNDS: usize = 8;
pub const MAX_SEARCH_INPUTS: usize = 96;

#[derive(Debug, Default)]
pub struct PatrolSearchState {
    rounds: usize,
    inputs: usize,
    tried: Vec<[i32; 2]>,
    pending: bool,
    pending_reads: usize,
    reference: Option<PatrolPoint>,
    close_attempts: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatrolSearchStep {
    Waiting,
    DetailsClosed,
    IconSelected,
    Panned,
    Found,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatrolMoveStep {
    Waiting,
    Turned,
    Walked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatrolFilterPhase {
    Open,
    Option,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatrolFilterStep {
    Waiting,
    Opening,
    Opened,
    Selected,
    Closing,
    Closed,
}

/// 同一标题可能被 OCR 连同漏斗图标识别成“▼标识点筛选”。仅忽略符号和空白。
pub fn filter_title_matches(text: &str) -> bool {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        == "标识点筛选"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatrolAreaAction {
    Wait,
    CloseDetails([i32; 2]),
    AlreadySelected,
    Select([i32; 2]),
    Scroll { start: [i32; 2], end: [i32; 2] },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatrolAreaStep {
    Waiting,
    DetailsClosed,
    Selected,
    Scrolled,
}

fn area_name_key(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !"[]【】()（）".contains(*c))
        .collect()
}

#[derive(Debug)]
pub struct PatrolSight {
    texts: Vec<Word>,
    pub mini_map: Option<[i32; 4]>,
    pub heading: Option<f64>,
    pub map_open: bool,
    pub normal_world: bool,
    pub back: bool,
    pub position: Option<PatrolPoint>,
    pub area_info_open: bool,
    pub filter_open: bool,
    filter_close: Option<[i32; 2]>,
    area_info_close: Option<[i32; 2]>,
    transport_icons: Vec<[i32; 2]>,
    captured: Instant,
}
impl PatrolSight {
    pub fn age(&self) -> Duration {
        self.captured.elapsed()
    }
    pub fn point(&self, text: &str, rect: [i32; 4]) -> Option<[i32; 2]> {
        point(&self.texts, text, rect)
    }
    /// 详情和筛选面板不属于区域列表；缺少列表证据时不滚动。
    pub fn area_action(&self, target: &str, attempts: usize) -> Result<PatrolAreaAction> {
        if !self.map_open {
            return Ok(PatrolAreaAction::Wait);
        }
        ensure!(
            attempts < MAX_AREA_SELECTION_INPUTS,
            "锄大地区域选择达到 20 次输入上限，已停止滚动"
        );
        if let Some(close) = self.area_info_close {
            return Ok(PatrolAreaAction::CloseDetails(close));
        }
        if self.area_info_open || self.filter_open {
            return Ok(PatrolAreaAction::Wait);
        }
        let target = area_name_key(target);
        let mut title: Vec<_> = self
            .texts
            .iter()
            .filter(|word| word.inside(AREA_TITLE_RECT))
            .collect();
        title.sort_by_key(|word| word.rect[0]);
        if !target.is_empty()
            && area_name_key(
                &title
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<String>(),
            ) == target
        {
            return Ok(PatrolAreaAction::AlreadySelected);
        }
        let candidates: Vec<_> = self
            .texts
            .iter()
            .filter(|word| word.inside(AREA_LIST_RECT) && area_name_key(&word.text) == target)
            .collect();
        if candidates.len() == 1 {
            return Ok(PatrolAreaAction::Select(candidates[0].center()));
        }
        if !candidates.is_empty() {
            return Ok(PatrolAreaAction::Wait);
        }
        let mut rows: Vec<_> = self
            .texts
            .iter()
            .filter(|word| {
                word.inside(AREA_LIST_RECT)
                    && word.text != "前往"
                    && word
                        .text
                        .chars()
                        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
                        .count()
                        >= 2
            })
            .map(|word| word.center()[1])
            .collect();
        rows.sort();
        rows.dedup_by(|next, previous| (*next - *previous).abs() < 30);
        if rows.len() < 3 {
            return Ok(PatrolAreaAction::Wait);
        }
        Ok(PatrolAreaAction::Scroll {
            start: [1630, 630],
            end: [1630, if attempts % 12 < 6 { 350 } else { 900 }],
        })
    }
}
/// 模板候选的相对坐标与大地图图标一致时投票。并列候选不产生位置。
pub fn locate_icons(
    area: &PatrolArea,
    previous: PatrolPoint,
    max_distance: f64,
    icons: &[(String, [i32; 2])],
) -> Option<PatrolPoint> {
    if !max_distance.is_finite() || !(0.0..=800.0).contains(&max_distance) || icons.len() > 256 {
        return None;
    }
    let mut candidates: Vec<(PatrolPoint, BTreeSet<usize>)> = vec![];
    for (index, (id, [x, y])) in icons.iter().enumerate() {
        if !(0..210).contains(x) || !(0..210).contains(y) {
            continue;
        }
        for icon in area.icons.iter().filter(|i| i.template_id == *id) {
            let p = PatrolPoint {
                x: icon.lm_pos.x - x + 105,
                y: icon.lm_pos.y - y + 105,
            };
            if p.x < 0
                || p.y < 0
                || p.x >= area.width as i32
                || p.y >= area.height as i32
                || p.distance(previous) > max_distance
            {
                continue;
            }
            if let Some((_, votes)) = candidates.iter_mut().find(|(at, _)| at.distance(p) < 10.0) {
                votes.insert(index);
            } else {
                candidates.push((p, BTreeSet::from([index])));
            }
        }
    }
    candidates.sort_by_key(|(_, votes)| std::cmp::Reverse(votes.len()));
    let best = candidates.first()?;
    if candidates
        .get(1)
        .is_some_and(|(_, votes)| votes.len() == best.1.len())
    {
        None
    } else {
        Some(best.0)
    }
}
fn crop(bgr: &[u8], rect: [i32; 4]) -> Result<RgbImage> {
    let [x, y, w, h] = rect;
    ensure!(
        bgr.len() == 1920 * 1080 * 3
            && x >= 0
            && y >= 0
            && w == 210
            && h == 210
            && x + w <= 1920
            && y + h <= 1080,
        "小地图裁剪无效"
    );
    Ok(RgbImage::from_fn(210, 210, |xx, yy| {
        let at = (((y as u32 + yy) * 1920 + x as u32 + xx) * 3) as usize;
        image::Rgb([bgr[at + 2], bgr[at + 1], bgr[at]])
    }))
}
/// 图标缺失时只接受有足够道路像素且候选明显分离的局部道路匹配。
pub fn locate_road(
    map: &GrayImage,
    mini: &RgbImage,
    previous: PatrolPoint,
    radius: i32,
) -> Option<PatrolPoint> {
    if mini.dimensions() != (210, 210)
        || !(1..=160).contains(&radius)
        || previous.x < 0
        || previous.y < 0
        || previous.x as u32 >= map.width()
        || previous.y as u32 >= map.height()
    {
        return None;
    }
    let mut roads = GrayImage::new(210, 210);
    for (x, y, pixel) in mini.enumerate_pixels() {
        let min = *pixel.0.iter().min()?;
        let max = *pixel.0.iter().max()?;
        if max <= 40
            && max - min <= 5
            && (x as i32 - 105).pow(2) + (y as i32 - 105).pow(2) <= 98 * 98
            && (x as i32 - 207).pow(2) + (y as i32 - 189).pow(2) >= 50 * 50
        {
            roads.put_pixel(x, y, image::Luma([255]));
        }
    }
    for (x, y, pixel) in navigation_vision::player_mask(mini).enumerate_pixels() {
        if pixel[0] > 0 {
            roads.put_pixel(x, y, *pixel);
        }
    }
    // 原版在道路阈值后先填平斜纹间隙，再去除噪点；直接稀疏采样会丢失大部分道路。
    roads = navigation_vision::morphology(&navigation_vision::morphology(&roads, true), false);
    roads = navigation_vision::morphology(&navigation_vision::morphology(&roads, false), true);
    let mut samples = vec![];
    for y in (8..202i32).step_by(5) {
        for x in (8..202i32).step_by(5) {
            if (x - 105).pow(2) + (y - 105).pow(2) > 98 * 98
                || (x - 207).pow(2) + (y - 189).pow(2) < 50 * 50
            {
                continue;
            }
            samples.push((x - 105, y - 105, roads.get_pixel(x as u32, y as u32)[0] > 0));
        }
    }
    let count = samples.iter().filter(|(_, _, road)| *road).count();
    if count < 40 || count > samples.len() * 9 / 10 {
        return None;
    }
    let mut matches = vec![];
    for dy in (-radius..=radius).step_by(3) {
        for dx in (-radius..=radius).step_by(3) {
            let p = PatrolPoint {
                x: previous.x + dx,
                y: previous.y + dy,
            };
            if p.x < 105
                || p.y < 105
                || p.x + 105 >= map.width() as i32
                || p.y + 105 >= map.height() as i32
                || p.distance(previous) > f64::from(radius)
            {
                continue;
            }
            let mut intersection = 0;
            let mut map_count = 0;
            for &(x, y, road) in &samples {
                let on = map.get_pixel((p.x + x) as u32, (p.y + y) as u32)[0] > 127;
                intersection += usize::from(on && road);
                map_count += usize::from(on);
            }
            // 二值图的归一化相关系数，沿用原版 TM_CCOEFF_NORMED 的评分含义。
            // 小地图有条纹和 HUD 遮挡，不能用道路交并比替代相关性。
            let n = samples.len() as f64;
            let a = count as f64;
            let b = map_count as f64;
            let variance = (a * (1.0 - a / n) * b * (1.0 - b / n)).sqrt();
            if variance > 0.0 {
                matches.push(((intersection as f64 - a * b / n) / variance, p));
            }
        }
    }
    matches.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (score, p) = *matches.first()?;
    let other = matches
        .iter()
        .find(|(_, other)| other.distance(p) > 15.0)
        .map_or(0.0, |(s, _)| *s);
    (score >= 0.1 && score - other >= 0.04).then_some(p)
}
pub fn validate_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("缺少锄大地资源，请重新生成")?;
    let manifest_path = root.join("world-patrol-manifest.json");
    od_core::config::reject_links(&manifest_path)?;
    ensure!(
        std::fs::metadata(&manifest_path)?.len() < 2 * 1024 * 1024,
        "锄大地清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
    let sources: Value = serde_json::from_str(include_str!(
        "../../../resources/world-patrol/source-lock.json"
    ))?;
    ensure!(
        manifest["schema_version"] == 1
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["sources"] == sources,
        "锄大地资源版本或来源不一致"
    );
    let mut required = BTreeMap::new();
    let images: BTreeMap<String, String> = serde_json::from_str(include_str!(
        "../../../resources/world-patrol/image-lock.json"
    ))?;
    required.extend(images);
    required.insert(
        "catalog.json".into(),
        hex::encode(Sha256::digest(include_bytes!(
            "../../../resources/world-patrol/catalog.json"
        ))),
    );
    required.insert(
        "pipeline/world-patrol.json".into(),
        hex::encode(Sha256::digest(serde_json::to_vec_pretty(
            &od_core::world_patrol_pipeline::pipeline(),
        )?)),
    );
    for (name, hash) in sources.as_object().context("锄大地来源无效")? {
        if name.ends_with("road_mask.png") {
            required.insert(
                format!(
                    "maps/{}",
                    name.strip_prefix("assets/game_data/world_patrol/")
                        .context("地图路径无效")?
                ),
                hash.as_str().context("地图散列无效")?.into(),
            );
        }
    }
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let model = lock["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR 清单")?;
    for file in model["files"].as_array().unwrap() {
        if let Some(path) = file["bundle_path"].as_str() {
            required.insert(path.into(), file["sha256"].as_str().unwrap().into());
        }
    }
    let mut seen = BTreeSet::new();
    let files = manifest["files"].as_array().context("锄大地清单缺少文件")?;
    ensure!(files.len() < 256, "锄大地文件过多");
    for file in files {
        let name = file["path"].as_str().context("锄大地路径无效")?;
        ensure!(
            !name.contains(['\\', ':'])
                && Path::new(name)
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                && seen.insert(name),
            "锄大地文件路径无效"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        let bytes = std::fs::read(path)?;
        let hash = hex::encode(Sha256::digest(&bytes));
        ensure!(
            file["sha256"] == hash && file["bytes"] == bytes.len() as u64,
            "锄大地文件校验失败：{name}"
        );
        let expected = required.remove(name).context("锄大地含未知文件")?;
        ensure!(hash == expected, "锄大地固定文件不匹配：{name}");
    }
    ensure!(required.is_empty(), "锄大地缺少必要文件");
    for entry in walkdir::WalkDir::new(&root).min_depth(1) {
        let entry = entry?;
        od_core::config::reject_links(entry.path())?;
        let name = entry
            .path()
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        ensure!(
            if entry.file_type().is_file() {
                name == "world-patrol-manifest.json" || seen.contains(name.as_str())
            } else {
                seen.iter().any(|p| p.starts_with(&format!("{name}/")))
            },
            "锄大地含未登记文件或目录"
        );
    }
    Ok(root)
}
impl NativeSession {
    pub fn reset_patrol_steering(&mut self) {
        self.patrol_steering.invalidate();
    }
    pub fn patrol_is_walking(&self) -> bool {
        self.held_keys.contains(&87)
    }
    pub fn load_world_patrol_bundle(&mut self, root: &Path) -> Result<()> {
        self.release_inputs()?;
        let root = validate_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("锄大地路径无效")?)?
                .wait()
                .is_success(),
            "锄大地资源加载失败"
        );
        Ok(())
    }
    pub(crate) fn patrol_match(
        &self,
        image: &RecognitionImage,
        template: &str,
        roi: [i32; 4],
        threshold: f64,
    ) -> Result<Value> {
        Ok(self.recognize_prepared(image,"TemplateMatch",&json!({"template":format!("world-patrol/{template}.png"),"roi":roi,"threshold":threshold,"green_mask":true,"method":5,"order_by":"Score"}))?.detail)
    }
    pub fn patrol_world_in_combat(
        &self,
        frame: &crate::combat_vision::CombatFrame,
    ) -> Result<bool> {
        let bgr = frame.image.buffer.raw_data().context("缺少战斗画面像素")?;
        let matched =
            self.patrol_match(&frame.image, "normal_world/map", [1696, 319, 162, 150], 0.5)?;
        let mini = if matched["hit"] == true {
            let [x, y, _, _]: [i32; 4] =
                serde_json::from_value(matched["detail"]["best"]["box"].clone())?;
            crop(bgr, [x - 169, y - 151, 210, 210])
                .ok()
                .is_some_and(|rgb| navigation_vision::player_found(&rgb))
        } else {
            false
        };
        // 动态模板命中并不代表裁出的区域确有玩家箭头；失败时仍须回退。
        let mini = mini
            || navigation_vision::player_found(&crop(bgr, navigation_vision::STATIC_MAP_RECT)?);
        // 探索菜单和地图按钮同时可见也能证明已回到探索；玩家箭头在
        // 战斗结束动画中可能暂时缺失，不能因此继续困在战斗阶段。
        let exploring = (mini || matched["hit"] == true)
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
        Ok(exploring)
    }
    pub fn patrol_battle_ready(
        &self,
        recognizer: &mut crate::combat_vision::CombatRecognizer,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        let Some(mut frame) = self.combat_frame(cancel)? else {
            return Ok(false);
        };
        let ready = recognizer.align_patrol_frame(self, &mut frame, cancel)?;
        Ok(ready && frame.age_ms() <= 1000 && !cancel.load(Ordering::Acquire))
    }
    pub fn read_world_patrol(
        &self,
        area: Option<&PatrolArea>,
        previous: Option<PatrolPoint>,
        radius: f64,
        root: &Path,
        cancel: &AtomicBool,
    ) -> Result<Option<PatrolSight>> {
        let captured = Instant::now();
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let text = if area.is_none() {
            words(
                &self
                    .recognize_prepared(
                        &frame.image,
                        "OCR",
                        &json!({"model":"ppocrv5","threshold":0.6}),
                    )?
                    .detail,
            )?
        } else {
            vec![]
        };
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let bgr = frame.image.buffer.raw_data().context("锄大地截图无像素")?;
        let mut rect = None;
        let result =
            self.patrol_match(&frame.image, "normal_world/map", [1696, 319, 162, 150], 0.5)?;
        if result["hit"] == true {
            let [x, y, _, _]: [i32; 4] =
                serde_json::from_value(result["detail"]["best"]["box"].clone())?;
            let at = [x - 169, y - 151, 210, 210];
            if let Ok(rgb) = crop(bgr, at)
                && navigation_vision::player_found(&rgb)
            {
                rect = Some(at);
            }
        }
        if rect.is_none()
            && navigation_vision::player_found(&crop(bgr, navigation_vision::STATIC_MAP_RECT)?)
        {
            rect = Some(navigation_vision::STATIC_MAP_RECT);
        }
        let mut heading = None;
        let mut position = None;
        if let Some(rect) = rect {
            let rgb = crop(bgr, rect)?;
            heading = navigation_vision::heading(&rgb)?.map(|h| h.degrees);
            if let Some((area, previous)) = area.zip(previous) {
                let mut detected = vec![];
                let templates: BTreeSet<_> = area
                    .icons
                    .iter()
                    .filter(|i| i.lm_pos.distance(previous) <= radius + 150.0)
                    .map(|i| &i.template_id)
                    .collect();
                for id in templates {
                    if cancel.load(Ordering::Acquire) {
                        return Ok(None);
                    }
                    let result =
                        self.patrol_match(&frame.image, &format!("map/{id}"), rect, 0.7)?;
                    if let Some(items) = result["detail"]["filtered"].as_array() {
                        for item in items.iter().take(128) {
                            let [x, y, w, h]: [i32; 4] =
                                serde_json::from_value(item["box"].clone())?;
                            detected.push((id.clone(), [x + w / 2 - rect[0], y + h / 2 - rect[1]]));
                        }
                    }
                }
                position = locate_icons(area, previous, radius, &detected);
                if position.is_none() {
                    let path =
                        root.join(format!("maps/{}/{}/road_mask.png", area.entry_id, area.id));
                    let map = image::open(path)?.to_luma8();
                    position = locate_road(&map, &rgb, previous, (radius as i32).clamp(1, 160));
                }
            }
        }
        let back =
            self.patrol_match(&frame.image, "menu/back", [20, 0, 180, 110], 0.7)?["hit"] == true;
        let normal_world = if area.is_none() {
            self.patrol_match(
                &frame.image,
                "normal_world/function_menu",
                [1776, 28, 93, 89],
                0.7,
            )?["hit"]
                == true
                && self.patrol_match(
                    &frame.image,
                    "normal_world/preset_config",
                    [1225, 44, 280, 58],
                    0.7,
                )?["hit"]
                    == true
        } else {
            false
        };
        let filter_open = back
            && text
                .iter()
                .any(|word| word.inside([1240, 10, 1510, 120]) && filter_title_matches(&word.text));
        let map_open =
            point(&text, "快捷导航", [1490, 10, 1690, 95]).is_some() && back && !filter_open;
        let filter_close = if filter_open {
            let matched =
                self.patrol_match(&frame.image, "menu/btn_close", [1755, 5, 135, 110], 0.7)?;
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
        let area_info_close = if map_open {
            let matched = self.patrol_match(
                &frame.image,
                "normal_world_investigation/btn_area_info_close",
                [1760, 100, 110, 100],
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
        let area_info_open = area_info_close.is_some()
            || (map_open
                && point(&text, "前往", [1542, 874, 1746, 930]).is_some()
                && text.iter().any(|word| word.inside([1442, 118, 1794, 180])));
        let transport_icons = if map_open {
            self.patrol_transport_icons_in(&frame.image)?
        } else {
            vec![]
        };
        if cancel.load(Ordering::Acquire) || captured.elapsed() > Duration::from_millis(1500) {
            return Ok(None);
        }
        Ok(Some(PatrolSight {
            texts: text,
            mini_map: rect,
            heading,
            map_open,
            normal_world,
            back,
            position,
            area_info_open,
            filter_open,
            filter_close,
            area_info_close,
            transport_icons,
            captured,
        }))
    }
    /// 与执行器及截图回归共用同一决策和输入入口，每次只发送一个动作。
    pub fn patrol_area_step(
        &mut self,
        sight: &PatrolSight,
        target: &str,
        attempts: usize,
        cancel: Arc<AtomicBool>,
    ) -> Result<PatrolAreaStep> {
        if cancel.load(Ordering::Acquire) || sight.captured.elapsed() > Duration::from_millis(1500)
        {
            return Ok(PatrolAreaStep::Waiting);
        }
        Ok(match sight.area_action(target, attempts)? {
            PatrolAreaAction::Wait => PatrolAreaStep::Waiting,
            PatrolAreaAction::AlreadySelected => PatrolAreaStep::Selected,
            PatrolAreaAction::CloseDetails(point) => {
                self.patrol_click(point, &cancel)?;
                PatrolAreaStep::DetailsClosed
            }
            PatrolAreaAction::Select(point) => {
                self.patrol_click(point, &cancel)?;
                PatrolAreaStep::Selected
            }
            PatrolAreaAction::Scroll { start, end } => {
                self.patrol_drag(start, end, cancel)?;
                PatrolAreaStep::Scrolled
            }
        })
    }
    /// 筛选的打开、选择和关闭必须各自回读；点击关闭本身不代表面板已关闭。
    pub fn patrol_filter_step(
        &mut self,
        sight: &PatrolSight,
        phase: PatrolFilterPhase,
        hollow: bool,
        attempts: usize,
        cancel: Arc<AtomicBool>,
    ) -> Result<PatrolFilterStep> {
        if cancel.load(Ordering::Acquire) || sight.captured.elapsed() > Duration::from_millis(1500)
        {
            return Ok(PatrolFilterStep::Waiting);
        }
        match phase {
            PatrolFilterPhase::Open if sight.filter_open => Ok(PatrolFilterStep::Opened),
            PatrolFilterPhase::Open if sight.map_open => {
                ensure!(
                    attempts < MAX_FILTER_INPUTS,
                    "锄大地筛选面板未打开，已停止重复点击"
                );
                self.patrol_click([1826, 46], &cancel)?;
                Ok(PatrolFilterStep::Opening)
            }
            PatrolFilterPhase::Option if sight.filter_open => {
                if let Some(point) = sight.point(
                    if hollow { "裂隙信标" } else { "传送" },
                    [1282, 226, 1850, 954],
                ) {
                    self.patrol_click(point, &cancel)?;
                    Ok(PatrolFilterStep::Selected)
                } else {
                    Ok(PatrolFilterStep::Waiting)
                }
            }
            PatrolFilterPhase::Close if sight.filter_open => {
                ensure!(
                    attempts < MAX_FILTER_INPUTS,
                    "锄大地筛选面板未关闭，已停止重复点击"
                );
                if let Some(point) = sight.filter_close {
                    self.patrol_click(point, &cancel)?;
                    Ok(PatrolFilterStep::Closing)
                } else {
                    Ok(PatrolFilterStep::Waiting)
                }
            }
            PatrolFilterPhase::Close if sight.map_open => Ok(PatrolFilterStep::Closed),
            _ => Ok(PatrolFilterStep::Waiting),
        }
    }
    /// 连续有效帧之间保留 W，小幅纠偏时继续前进；不可靠、急转或取消时松键。
    pub fn patrol_move_step(
        &mut self,
        sight: &PatrolSight,
        target: PatrolPoint,
        cancel: &AtomicBool,
    ) -> Result<PatrolMoveStep> {
        let result = (|| {
            if cancel.load(Ordering::Acquire)
                || sight.captured.elapsed() > Duration::from_millis(1500)
                || sight.mini_map.is_none()
            {
                return Ok(PatrolMoveStep::Waiting);
            }
            ensure!(!self.tasker.running(), "锄大地步行前须等待 Pipeline 收尾");
            if let Some(window) = &self.game_window {
                od_platform::check_keyboard_target(window, true)?;
            }
            let Some((position, heading)) = sight.position.zip(sight.heading) else {
                return Ok(PatrolMoveStep::Waiting);
            };
            if !heading.is_finite() || position.distance(target) < 10.0 {
                return Ok(PatrolMoveStep::Waiting);
            }
            let steering = self
                .patrol_steering
                .observe(heading, position.heading(target), position.distance(target))
                .map_err(anyhow::Error::msg)?;
            if !steering.forward {
                self.release_inputs()?;
            }
            if let Some(dx) = steering.dx {
                self.combat_turn(dx, cancel)?;
            }
            if steering.forward {
                if !self.held_keys.contains(&87) {
                    self.combat_input(InputProbeAction::Key { key: 87 }, true, cancel)?;
                }
                Ok(PatrolMoveStep::Walked)
            } else {
                Ok(PatrolMoveStep::Turned)
            }
        })();
        match result {
            Ok(PatrolMoveStep::Walked) if !cancel.load(Ordering::Acquire) => {
                Ok(PatrolMoveStep::Walked)
            }
            Ok(step) => {
                if step != PatrolMoveStep::Turned || cancel.load(Ordering::Acquire) {
                    self.reset_patrol_steering();
                }
                self.release_inputs().map(|()| {
                    if cancel.load(Ordering::Acquire) {
                        PatrolMoveStep::Waiting
                    } else {
                        step
                    }
                })
            }
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
    /// 每次平移前先检查当前可见信标；旧详情只关闭，不能反复当作导航参考。
    pub fn patrol_search_step(
        &mut self,
        sight: &PatrolSight,
        area: &PatrolArea,
        target: &str,
        state: &mut PatrolSearchState,
        cancel: Arc<AtomicBool>,
    ) -> Result<PatrolSearchStep> {
        if cancel.load(Ordering::Acquire)
            || sight.captured.elapsed() > Duration::from_millis(1500)
            || !sight.map_open
            || sight.filter_open
        {
            return Ok(PatrolSearchStep::Waiting);
        }
        let target_icon = area
            .icons
            .iter()
            .find(|icon| !target.is_empty() && icon.icon_name == target)
            .context("锄大地目标传送点不在当前区域")?;
        let go = sight.point("前往", [1542, 874, 1746, 930]).is_some();
        if sight.area_info_open && go && sight.point(target, [1442, 118, 1794, 180]).is_some() {
            return Ok(PatrolSearchStep::Found);
        }
        ensure!(
            state.rounds < MAX_SEARCH_ROUNDS && state.inputs < MAX_SEARCH_INPUTS,
            "锄大地传送点搜索达到上限，已停止拖动"
        );
        if sight.area_info_open {
            if state.pending {
                state.pending = false;
                if go && state.reference.is_none() {
                    state.reference = area.icons.iter().find_map(|icon| {
                        (!icon.icon_name.is_empty()
                            && sight
                                .point(&icon.icon_name, [1442, 118, 1794, 180])
                                .is_some())
                        .then_some(icon.lm_pos)
                    });
                }
            }
            if let Some(close) = sight.area_info_close {
                ensure!(
                    state.close_attempts < MAX_DETAIL_CLOSE_INPUTS,
                    "锄大地传送点详情未关闭，已停止重复点击"
                );
                state.close_attempts += 1;
                state.inputs += 1;
                self.patrol_click(close, &cancel)?;
                return Ok(PatrolSearchStep::DetailsClosed);
            }
            return Ok(PatrolSearchStep::Waiting);
        }
        state.close_attempts = 0;
        if state.pending {
            state.pending_reads += 1;
            if state.pending_reads < 3 {
                return Ok(PatrolSearchStep::Waiting);
            }
            state.pending = false;
        }
        if let Some(&icon) = sight.transport_icons.iter().find(|icon| {
            !state
                .tried
                .iter()
                .any(|tried| (tried[0] - icon[0]).pow(2) + (tried[1] - icon[1]).pow(2) < 25 * 25)
        }) {
            state.tried.push(icon);
            state.pending = true;
            state.pending_reads = 0;
            state.inputs += 1;
            self.patrol_click(icon, &cancel)?;
            return Ok(PatrolSearchStep::IconSelected);
        }
        let [dx, dy] = if let Some(reference) = state.reference.take() {
            let dx = target_icon.lm_pos.x - reference.x;
            let dy = target_icon.lm_pos.y - reference.y;
            let scale = f64::from(dx.abs().max(dy.abs()).max(1));
            [
                -(f64::from(dx) * 300.0 / scale) as i32,
                -(f64::from(dy) * 300.0 / scale) as i32,
            ]
        } else {
            [
                [300, 0],
                [-300, 0],
                [0, 300],
                [0, -300],
                [300, 300],
                [-300, -300],
                [300, -300],
                [-300, 300],
            ][state.rounds % MAX_SEARCH_ROUNDS]
        };
        state.inputs += 1;
        self.patrol_drag([723, 542], [723 + dx, 542 + dy], cancel)?;
        state.rounds += 1;
        state.tried.clear();
        Ok(PatrolSearchStep::Panned)
    }
    fn patrol_transport_icons_in(&self, image: &RecognitionImage) -> Result<Vec<[i32; 2]>> {
        let r = self.patrol_match(image, "map/3d_map_tp_icon_1", [66, 65, 1314, 935], 0.5)?;
        let mut out = vec![];
        if let Some(items) = r["detail"]["filtered"].as_array() {
            for item in items.iter().take(64) {
                let [x, y, w, h]: [i32; 4] = serde_json::from_value(item["box"].clone())?;
                out.push([x + w / 2, y + h / 2]);
            }
        }
        Ok(out)
    }
    pub fn patrol_click(&mut self, [x, y]: [i32; 2], cancel: &AtomicBool) -> Result<()> {
        ensure!(
            (0..1920).contains(&x) && (0..1080).contains(&y),
            "锄大地点选越界"
        );
        let result = (|| {
            if self.game_window.is_some() {
                self.prepare_input_target(
                    od_core::map_transport::MapConfig::recovery_inputs()[0],
                    cancel,
                    &mut None,
                )?;
            }
            self.combat_input(InputProbeAction::Key { key: 18 }, true, cancel)?;
            self.combat_input(
                InputProbeAction::MouseButton {
                    button: MouseButton::Left,
                    x,
                    y,
                },
                true,
                cancel,
            )?;
            std::thread::sleep(Duration::from_millis(80));
            Ok(())
        })();
        let release = self.release_inputs();
        result.and(release)
    }
    pub fn patrol_key(&mut self, key: i32, cancel: &AtomicBool) -> Result<()> {
        ensure!(
            [27, 70, 78, 87, 65, 83, 68, 32, 9].contains(&key),
            "锄大地按键无效"
        );
        let result = (|| {
            self.combat_input(InputProbeAction::Key { key }, true, cancel)?;
            std::thread::sleep(Duration::from_millis(80));
            Ok(())
        })();
        let release = self.release_inputs();
        result.and(release)
    }
    pub fn patrol_drag(
        &mut self,
        start: [i32; 2],
        end: [i32; 2],
        cancel: Arc<AtomicBool>,
    ) -> Result<()> {
        ensure!(
            [start, end]
                .iter()
                .all(|p| (0..1920).contains(&p[0]) && (0..1080).contains(&p[1])),
            "锄大地拖动越界"
        );
        if self.game_window.is_some() {
            self.prepare_input_target(
                od_core::map_transport::MapConfig::recovery_inputs()[0],
                &cancel,
                &mut None,
            )?;
        }
        let report=self.run_pipeline_detailed("Patrol.Drag",&crate::PipelineInputPlan{keys:BTreeSet::new(),contacts:BTreeSet::from([0])},cancel,Duration::from_secs(3),&json!({"Patrol.Drag":{"recognition":"DirectHit","action":"Swipe","begin":start,"end":end,"duration":300}}).to_string(),false)?;
        ensure!(
            report.report.end == crate::PipelineEnd::Succeeded
                || report.report.end == crate::PipelineEnd::Cancelled,
            "锄大地拖动失败"
        );
        Ok(())
    }
}
