//! 战略、周期增益和本周 UP 的同帧观察。只交接出战页，不发送出战。
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, Word, words},
};
use anyhow::{Context, Result, ensure};
use image::{GrayImage, Luma};
use imageproc::{
    contours::{BorderType, find_contours},
    geometry::contour_area,
};
use od_core::{
    InputProbeAction, MouseButton,
    lost_void::LostVoidMission as M,
    lost_void_scene::{LostVoidObservation, LostVoidScreen as S},
    lost_void_strategy::{
        LostVoidStrategyFlow, LostVoidStrategyView, MissionSettings, StrategyAction as A,
        StrategyCandidate, StrategyPage, StrategyProgress as P,
    },
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyInputEnd {
    Posted,
    NotPosted,
    Changed,
    PartialScroll,
    TimedOut,
}
struct Sight {
    view: LostVoidStrategyView,
    points: BTreeMap<String, [i32; 2]>,
}

fn pixels(frame: &RecognitionImage) -> Result<&[u8]> {
    ensure!(
        frame.buffer.width() == 1920
            && frame.buffer.height() == 1080
            && frame.buffer.image_type() == 16,
        "战略截图须为 1920×1080 BGR"
    );
    let b = frame.buffer.raw_data().context("战略截图没有像素")?;
    ensure!(b.len() == 1920 * 1080 * 3, "战略截图像素长度无效");
    Ok(b)
}
fn rgb(b: &[u8], x: usize, y: usize) -> [u8; 3] {
    let i = (y * 1920 + x) * 3;
    [b[i + 2], b[i + 1], b[i]]
}
fn hsv([r, g, b]: [u8; 3]) -> [i32; 3] {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let v = r.max(g).max(b);
    let d = v - r.min(g).min(b);
    if d == 0 {
        return [0, 0, v];
    }
    let scale = (30.0 * 4096.0 / f64::from(d)).round() as i32;
    let n = if v == r {
        g - b
    } else if v == g {
        b - r + 2 * d
    } else {
        r - g + 4 * d
    };
    let mut h = (n * scale + 2048) >> 12;
    if h < 0 {
        h += 180;
    }
    [h, (f64::from(d) * 255.0 / f64::from(v)).round() as i32, v]
}
// OpenCV 默认矩形核锚点 floor(k/2)，边界腐蚀视为 255、膨胀视为 0。
// 逐行/列前缀和避免在每个像素重复扫描整个核，保留四像素偶数核的偏移。
fn morph(src: &GrayImage, k: i32, erode: bool) -> GrayImage {
    let (w, h) = (src.width() as i32, src.height() as i32);
    let mut current = src.clone();
    for horizontal in [true, false] {
        let mut out = GrayImage::new(w as u32, h as u32);
        let (lines, len) = if horizontal { (h, w) } else { (w, h) };
        let mut sums = vec![0i32; len as usize + 1];
        for line in 0..lines {
            sums[0] = 0;
            for i in 0..len {
                let (x, y) = if horizontal { (i, line) } else { (line, i) };
                sums[i as usize + 1] =
                    sums[i as usize] + i32::from(current.get_pixel(x as u32, y as u32)[0] != 0);
            }
            for i in 0..len {
                let a = i - k / 2;
                let z = a + k;
                let l = a.max(0);
                let r = z.min(len);
                let inside = r - l;
                let count = sums[r as usize] - sums[l as usize];
                let hit = if erode { count == inside } else { count > 0 };
                let (x, y) = if horizontal { (i, line) } else { (line, i) };
                out.put_pixel(x as u32, y as u32, Luma([if hit { 255 } else { 0 }]));
            }
        }
        current = out;
    }
    current
}
fn boxes(mask: GrayImage, min: f64, max: f64, square: bool) -> Vec<[i32; 4]> {
    let mut padded = GrayImage::new(mask.width() + 2, mask.height() + 2);
    image::imageops::replace(&mut padded, &mask, 1, 1);
    let mut found = Vec::new();
    for c in find_contours::<i32>(&padded)
        .into_iter()
        .filter(|c| c.border_type == BorderType::Outer && c.parent.is_none())
    {
        let area = contour_area(&c.points);
        if !(min..=max).contains(&area) {
            continue;
        }
        let x = c.points.iter().map(|p| p.x).min().unwrap() - 1;
        let y = c.points.iter().map(|p| p.y).min().unwrap() - 1;
        let w = c.points.iter().map(|p| p.x).max().unwrap() - x;
        let h = c.points.iter().map(|p| p.y).max().unwrap() - y;
        if square && !(0.9..=1.1).contains(&(f64::from(w) / f64::from(h))) {
            continue;
        }
        found.push([x + 2, y + 206, w, h]);
    }
    found.sort_by_key(|r| r[0]);
    found
}
/// 来源固定为三条调查战略轮廓流水线。缺少等级圈圈保持未知。
pub fn inspect_lost_void_strategy_markers(frame: &RecognitionImage) -> Result<Vec<(P, [i32; 4])>> {
    let b = pixels(frame)?;
    let mut masks = [
        GrayImage::new(1918, 152),
        GrayImage::new(1918, 152),
        GrayImage::new(1918, 152),
    ];
    for y in 0..152 {
        for x in 0..1918 {
            let [h, s, v] = hsv(rgb(b, x + 2, y + 206));
            // H=179±15 经过 180 回绕；不是将红色色相直接截断。
            for (i, hit) in [
                (
                    0,
                    (h >= 164 || h <= 14) && (8..=110).contains(&s) && v <= 126,
                ),
                (1, h <= 150 && s <= 10),
                (2, h == 0 && s == 0 && (150..=250).contains(&v)),
            ] {
                if hit {
                    masks[i].put_pixel(x as u32, y as u32, Luma([255]));
                }
            }
        }
    }
    let new = boxes(
        morph(&morph(&masks[0], 3, true), 3, false),
        3000.0,
        4000.0,
        false,
    );
    let frames = boxes(
        morph(&morph(&masks[1], 3, true), 5, false),
        2000.0,
        20000.0,
        true,
    );
    let mut digit = morph(&masks[2], 4, true);
    for _ in 0..3 {
        digit = morph(&digit, 5, false);
    }
    let digits = boxes(digit, 900.0, 10000.0, false);
    let mut out: Vec<_> = new.into_iter().map(|r| (P::New, r)).collect();
    for r @ [x, y, w, h] in frames {
        if out.iter().any(|(_, a)| {
            (a[0] + a[2] / 2 - x).abs() < w / 2 && (a[1] + a[3] / 2 - y).abs() < h / 2
        }) {
            continue;
        }
        let has_digit = digits.iter().any(|&[a, b, c, d]| {
            x < a + c / 2 && a + c / 2 < x + w && y < b + d / 2 && b + d / 2 < y + h
        });
        out.push((if has_digit { P::Max } else { P::NotMax }, r));
    }
    out.sort_by_key(|(_, r)| r[0]);
    Ok(out)
}
fn buff_state(frame: &RecognitionImage) -> Result<(bool, Option<u8>)> {
    let b = pixels(frame)?;
    let mut selected = Vec::new();
    for (i, cy) in [453, 597, 741].into_iter().enumerate() {
        let mut ring = 0;
        let mut inner = 0;
        for dy in -22i32..=22 {
            for dx in -22i32..=22 {
                let d = dx * dx + dy * dy;
                let [r, g, bb] = rgb(b, (1769 + dx) as usize, (cy + dy) as usize);
                let green = i32::from(g) > i32::from(r) + 25
                    && i32::from(g) > i32::from(bb) + 20
                    && g >= 90;
                if (14 * 14..=21 * 21).contains(&d) && green {
                    ring += 1;
                }
                if d <= 9 * 9 && green {
                    inner += 1;
                }
            }
        }
        if ring < 45 {
            return Ok((false, None));
        }
        if inner >= 20 {
            selected.push(i as u8 + 1);
        }
    }
    Ok((
        selected.len() <= 1,
        (selected.len() == 1).then(|| selected[0]),
    ))
}
fn unique(ws: &[Word], text: &str) -> Option<[i32; 2]> {
    let found: Vec<_> = ws.iter().filter(|w| w.text == text).collect();
    (found.len() == 1).then(|| found[0].center())
}
impl NativeSession {
    fn strategy_up(&self, r: &Reader<'_>, hint: Option<&[String]>) -> Result<Option<Vec<String>>> {
        let agents = od_core::random_play::agents();
        let mut up = Vec::new();
        for slot in 0..3usize {
            let roi = [990 + 277 * slot as i32, 396, 277, 202];
            // Maa 5.14.3 的 SIFT 使用 ROI 掩码，仍对整张图提取特征。
            // 先从同帧裁出本卡，保持原比例；输出框只在这张卡内校验。
            let b = pixels(r.frame)?;
            let mut card = Vec::with_capacity((roi[2] * roi[3] * 3) as usize);
            for y in roi[1]..roi[1] + roi[3] {
                let begin = (y as usize * 1920 + roi[0] as usize) * 3;
                card.extend_from_slice(&b[begin..begin + roi[2] as usize * 3]);
            }
            let mut buffer = maa_framework::buffer::MaaImageBuffer::new()?;
            buffer.set_raw_data(&card, roi[2], roi[3], 16)?;
            let card = RecognitionImage {
                buffer,
                prepare_us: 0,
            };
            let mut ids = Vec::new();
            for agent in agents
                .iter()
                .filter(|a| hint.is_none_or(|h| h.get(slot) == Some(&a.id)))
            {
                ensure!(
                    !r.cancel.load(Ordering::Acquire) && r.started.elapsed() < r.timeout,
                    "UP 识别取消或超时"
                );
                let templates: Vec<_> = agent
                    .templates
                    .iter()
                    .map(|t| format!("random-play/avatar_{t}.png"))
                    .collect();
                let detected = self.recognize_prepared(
                    &card,
                    "FeatureMatch",
                    &json!({"template":templates,"roi":[0,0,roi[2],roi[3]],
                    "detector":"SIFT","ratio":0.5,"count":4,"green_mask":true,"order_by":"Score"}),
                )?;
                if detected.detail["hit"] != true {
                    continue;
                }
                // 名册内不同皮肤可命中同一人；不同人同时命中属于歧义。
                let hits = detected.detail["detail"]["filtered"]
                    .as_array()
                    .context("UP 特征结果缺失")?;
                if hits.iter().any(|h| {
                    serde_json::from_value::<[i32; 4]>(h["box"].clone()).is_ok_and(
                        |[x, y, w, h]| {
                            w >= 40
                                && h >= 40
                                && x >= 0
                                && y >= 0
                                && x + w <= roi[2]
                                && y + h <= roi[3]
                        },
                    )
                }) {
                    ids.push(agent.id.clone());
                }
            }
            if ids.len() != 1 || up.contains(&ids[0]) {
                return Ok(None);
            }
            up.push(ids.remove(0));
        }
        Ok(Some(up))
    }
    pub fn inspect_lost_void_up_agents(
        &self,
        frame: &RecognitionImage,
        hint: Option<&[String]>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Vec<String>>> {
        self.require_lost_void_scene()?;
        if let Some(hint) = hint {
            MissionSettings {
                mission: M::TaskForce,
                buff_markers_valid: false,
                selected_buff: None,
                up_agents: Some(hint.to_vec()),
                next_enabled: false,
            }
            .validate()
            .map_err(anyhow::Error::msg)?;
        }
        self.strategy_up(
            &Reader {
                session: self,
                frame,
                cancel,
                started: Instant::now(),
                timeout,
                pipeline: od_core::lost_void_scene::pipeline(),
            },
            hint,
        )
    }
    fn strategy_sight(
        &self,
        frame: &RecognitionImage,
        hint: Option<&[String]>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let r = Reader {
            session: self,
            frame,
            cancel,
            started,
            timeout,
            pipeline: od_core::lost_void_scene::pipeline(),
        };
        let read = |n: &str| words(&r.read(&format!("LostVoid.{n}"), None)?);
        let mut s = Sight {
            view: LostVoidStrategyView {
                scene: LostVoidObservation::default(),
                page: None,
                settings: None,
            },
            points: BTreeMap::new(),
        };
        let modal = read("ModalConfirm")?;
        if unique(&modal, "确定").is_some() || unique(&modal, "确认").is_some() {
            return Ok(Some(s));
        }
        let title = read("PrepareTitle")?;
        let header = read("PrepareHeader")?;
        let footer = read("PrepareFooter")?;
        let purge = title.iter().any(|w| w.text.starts_with("战线肃清"));
        let task_force =
            unique(&title, "特遣调查").is_some() && unique(&footer, "特遣任务").is_some();
        let mission = if purge && !task_force {
            Some(M::Purge)
        } else if task_force && !purge {
            Some(M::TaskForce)
        } else {
            None
        };
        if let Some(mission) = mission
            && unique(&header, "街区").is_some()
            && r.hit("LostVoid.Back", None)?
        {
            let next = read("PrepareNextEnabled")?;
            let p = unique(&next, "下一步");
            s.view.scene.screen = if mission == M::Purge {
                S::Purge
            } else {
                S::TaskForce
            };
            let (buff_markers_valid, selected_buff) = if mission == M::Purge {
                buff_state(frame)?
            } else {
                (false, None)
            };
            let up_agents = if mission == M::TaskForce {
                self.strategy_up(&r, hint)?
            } else {
                None
            };
            s.view.settings = Some(MissionSettings {
                mission,
                buff_markers_valid,
                selected_buff,
                up_agents,
                next_enabled: p.is_some(),
            });
            s.points.insert("open".into(), [1752, 247]);
            for (i, y) in [453, 597, 741].into_iter().enumerate() {
                s.points.insert(format!("buff:{}", i + 1), [1769, y]);
            }
            if let Some(p) = p {
                s.points.insert("next".into(), p);
            }
        } else if unique(&footer, "出战").is_some() && unique(&footer, "预备编队").is_some() {
            s.view.scene.screen = S::Deploy;
        } else {
            let names = read("StrategyNames")?;
            let catalog = od_core::lost_void::strategies().map_err(anyhow::Error::msg)?;
            let mut named: Vec<_> = names
                .into_iter()
                .filter(|w| catalog.contains(&w.text))
                .collect();
            named.sort_by_key(|w| w.center()[0]);
            let confirm = read("StrategyConfirm")?;
            if named.len() >= 2 && unique(&confirm, "确定").is_some() {
                let enabled = read("StrategyConfirmEnabled")?;
                let p = unique(&enabled, "确定");
                let markers = inspect_lost_void_strategy_markers(frame)?;
                let mut candidates = Vec::new();
                for (i, w) in named.iter().enumerate() {
                    let x = w.center()[0];
                    let l = if i == 0 {
                        0
                    } else {
                        (named[i - 1].center()[0] + x) / 2
                    };
                    let z = if i + 1 == named.len() {
                        1920
                    } else {
                        (named[i + 1].center()[0] + x) / 2
                    };
                    let matched: Vec<_> = markers
                        .iter()
                        .filter(|(_, r)| l < r[0] + r[2] / 2 && r[0] + r[2] / 2 < z)
                        .collect();
                    let progress = if matched.len() == 1 {
                        matched[0].0
                    } else {
                        P::Unknown
                    };
                    candidates.push(StrategyCandidate {
                        slot: i as u8,
                        name: w.text.clone(),
                        progress,
                    });
                    let point = if matched.len() == 1 {
                        let r = matched[0].1;
                        [r[0] + r[2] / 2, r[1] + r[3] / 2]
                    } else {
                        w.center()
                    };
                    s.points.insert(format!("pick:{i}"), point);
                }
                let page = StrategyPage {
                    candidates,
                    selected_name: None,
                    confirm_enabled: p.is_some(),
                };
                page.validate().map_err(anyhow::Error::msg)?;
                s.view.scene.screen = S::Strategy;
                s.view.page = Some(page);
                if let Some(p) = p {
                    s.points.insert("confirm".into(), p);
                }
            }
        }
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        Ok(Some(s))
    }
    pub fn inspect_lost_void_strategy(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidStrategyView>> {
        self.require_lost_void_scene()?;
        Ok(self
            .strategy_sight(frame, None, cancel, timeout)?
            .map(|s| s.view))
    }
    pub fn read_lost_void_strategy(
        &self,
        flow: &LostVoidStrategyFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidStrategyView>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_lost_void_scene()?;
        let started = Instant::now();
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let Some(mut s) = self.strategy_sight(
            &frame.image,
            flow.up_agents(),
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?
        else {
            return Ok(None);
        };
        // 首次完整名册扫描只用于发现候选，随后用新帧逐槽验证三个已知身份。
        if frame.age() > Duration::from_millis(1500) {
            let hint = s
                .view
                .settings
                .as_ref()
                .and_then(|s| s.up_agents.as_deref());
            let Some(hint) = hint else { return Ok(None) };
            let Some(fresh) = self.navigation_frame(cancel)? else {
                return Ok(None);
            };
            let Some(verified) = self.strategy_sight(
                &fresh.image,
                Some(hint),
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            else {
                return Ok(None);
            };
            if fresh.age() > Duration::from_millis(1500) {
                return Ok(None);
            }
            s = verified;
        }
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        Ok(Some(s.view))
    }
    pub fn act_lost_void_strategy(
        &mut self,
        flow: &LostVoidStrategyFlow,
        action: A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<StrategyInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<StrategyInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_lost_void_scene()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(StrategyInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(StrategyInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(StrategyInputEnd::NotPosted);
            };
            let Some(s) = self.strategy_sight(
                &frame.image,
                flow.up_agents(),
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            else {
                return Ok(StrategyInputEnd::NotPosted);
            };
            if cancel.load(Ordering::Acquire) {
                return Ok(StrategyInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(StrategyInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &s.view) {
                return Ok(StrategyInputEnd::Changed);
            }
            let key = match action {
                A::Open => "open".into(),
                A::Pick(i) => format!("pick:{i}"),
                A::Confirm => "confirm".into(),
                A::Buff(i) => format!("buff:{i}"),
                A::Next => "next".into(),
                A::ScrollLeft | A::ScrollRight => "scroll".into(),
            };
            let scroll = matches!(action, A::ScrollLeft | A::ScrollRight);
            let point = if scroll {
                Some([960, 432])
            } else {
                s.points.get(&key).copied()
            };
            let Some([x, y]) = point else {
                return Ok(StrategyInputEnd::Changed);
            };
            let input = |x| InputProbeAction::MouseButton {
                button: MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input(x), true, cancel)? {
                return Ok(StrategyInputEnd::NotPosted);
            }
            let held = Instant::now();
            let duration = Duration::from_millis(if scroll { 300 } else { 100 });
            while held.elapsed() < duration
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                if scroll {
                    let sign = if action == A::ScrollLeft { -1 } else { 1 };
                    self.combat_input(
                        input(x + sign * (held.elapsed().as_millis().min(300) as i32) * 800 / 300),
                        true,
                        cancel,
                    )?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            // 完整拖动明确投递终点；中断的拖动仍保留意图，不复用旧坐标。
            if scroll
                && held.elapsed() >= duration
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                self.combat_input(
                    input(x + if action == A::ScrollLeft { -800 } else { 800 }),
                    true,
                    cancel,
                )?;
            }
            self.combat_input(input(x), false, &AtomicBool::new(false))?;
            if scroll
                && (held.elapsed() < duration
                    || cancel.load(Ordering::Acquire)
                    || started.elapsed() >= timeout)
            {
                return Ok(StrategyInputEnd::PartialScroll);
            }
            let wait = Instant::now();
            while wait.elapsed() < Duration::from_millis(300)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(StrategyInputEnd::Posted)
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}
