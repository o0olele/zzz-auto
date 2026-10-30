//! 武备槽位视觉、名称复读及同帧输入。输入坐标不进入检查点。
pub use crate::lost_void_choice::ChoiceInputEnd as GearInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, words},
};
use anyhow::{Context, Result, ensure};
use image::{GrayImage, Luma};
use imageproc::{
    contours::{BorderType, find_contours},
    geometry::contour_area,
};
use od_core::{
    InputProbeAction, MouseButton,
    lost_void_choice::identify_artifact,
    lost_void_gear::{
        GearAction, GearPage, GearSlot, LostVoidGearFlow, LostVoidGearView, valid_gear,
    },
    lost_void_scene::LostVoidScreen as S,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

fn level_pixel([r, g, b]: [u8; 3]) -> bool {
    // 参考 OpenCV 8 位 HSV 色相缩放，等级检测只限制 H=128..138。
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let v = r.max(g).max(b);
    let d = v - r.min(g).min(b);
    if d == 0 {
        return false;
    }
    let scale = ((30 * 4096) as f64 / f64::from(d)).round() as i32;
    let n = if v == r {
        g - b
    } else if v == g {
        b - r + 2 * d
    } else {
        r - g + 4 * d
    };
    let h = (n * scale + 2048) >> 12;
    let h = if h < 0 { h + 180 } else { h };
    (128..=138).contains(&h)
}
fn components(mut mask: GrayImage, min: f64, max: f64, offset: [i32; 2]) -> Vec<[i32; 4]> {
    // 补零边界，保留触及原裁剪边缘的外轮廓。
    let mut padded = GrayImage::new(mask.width() + 2, mask.height() + 2);
    image::imageops::replace(&mut padded, &mask, 1, 1);
    mask = padded;
    let mut boxes = Vec::new();
    for c in find_contours::<i32>(&mask)
        .into_iter()
        .filter(|c| c.border_type == BorderType::Outer && c.parent.is_none())
    {
        let area = contour_area(&c.points);
        if area < min || area > max {
            continue;
        }
        let x = c.points.iter().map(|p| p.x).min().unwrap();
        let y = c.points.iter().map(|p| p.y).min().unwrap();
        let w = c.points.iter().map(|p| p.x).max().unwrap() - x + 1;
        let h = c.points.iter().map(|p| p.y).max().unwrap() - y + 1;
        boxes.push([offset[0] + x - 1, offset[1] + y - 1, w, h]);
    }
    boxes.sort_by_key(|r| r[0]);
    boxes
}
fn detect_slots(frame: &RecognitionImage) -> Result<Vec<(GearSlot, [i32; 4])>> {
    ensure!(
        frame.buffer.width() == 1920
            && frame.buffer.height() == 1080
            && frame.buffer.image_type() == 16,
        "武备截图格式不匹配"
    );
    let bgr = frame.buffer.raw_data().context("武备截图没有像素")?;
    ensure!(bgr.len() == 1920 * 1080 * 3, "武备截图像素长度不匹配");
    let rgb = |x: usize, y: usize| {
        let i = (y * 1920 + x) * 3;
        [bgr[i + 2], bgr[i + 1], bgr[i]]
    };
    let mut mask = GrayImage::new(1550, 180);
    for y in 0..180 {
        for x in 0..1550 {
            let p = rgb(x + 190, y + 892);
            mask.put_pixel(
                x as u32,
                y as u32,
                Luma([if *p.iter().max().unwrap() >= 75 {
                    255
                } else {
                    0
                }]),
            );
        }
    }
    for _ in 0..2 {
        let mut expanded = GrayImage::new(1550, 180);
        for y in 0i32..180 {
            for x in 0i32..1550 {
                let hit = (-1..=1).any(|dy| {
                    (-1..=1).any(|dx| {
                        let (xx, yy) = (x + dx, y + dy);
                        (0..1550).contains(&xx)
                            && (0..180).contains(&yy)
                            && mask.get_pixel(xx as u32, yy as u32)[0] != 0
                    })
                });
                if hit {
                    expanded.put_pixel(x as u32, y as u32, Luma([255]));
                }
            }
        }
        mask = expanded;
    }
    let boxes = components(mask, 2000.0, 100000.0, [190, 892]);
    ensure!(
        !boxes.is_empty() && boxes.len() <= 12 && boxes.iter().all(|r| r[2] >= 60 && r[3] >= 60),
        "武备槽位数量或大小不明确"
    );
    ensure!(
        boxes.windows(2).all(|p| p[0][0] + p[0][2] < p[1][0]),
        "武备槽位互相重叠"
    );
    let mut level = GrayImage::new(1426, 38);
    for y in 0..38 {
        for x in 0..1426 {
            if level_pixel(rgb(x + 216, y + 922)) {
                level.put_pixel(x as u32, y as u32, Luma([255]));
            }
        }
    }
    let levels = components(level, 300.0, 10000.0, [216, 922]);
    let mut result = Vec::new();
    for (i, &[x, y, w, h]) in boxes.iter().enumerate() {
        let owned = levels
            .iter()
            .any(|&[a, b, c, d]| x < a + c && a < x + w && y < b + d && b < y + h);
        let (cx, cy) = (((x + w / 2 + 4) / 8) * 8, 990);
        let mut hash = Sha256::new();
        // 只采内部，排除选中边框及上方等级标记。
        for yy in cy - 20..cy + 20 {
            for xx in cx - 20..cx + 20 {
                hash.update(rgb(xx as usize, yy as usize).map(|v| v >> 4));
            }
        }
        result.push((
            GearSlot {
                slot: i as u8,
                owned,
                signature: hex::encode(hash.finalize()),
            },
            [x, y, w, h],
        ));
    }
    Ok(result)
}
pub fn inspect_lost_void_gear_slots(frame: &RecognitionImage) -> Result<Vec<(GearSlot, [i32; 4])>> {
    detect_slots(frame)
}
struct Sight {
    view: LostVoidGearView,
    points: BTreeMap<String, [i32; 2]>,
}
impl NativeSession {
    pub fn inspect_lost_void_gear(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidGearView>> {
        self.require_lost_void_scene()?;
        Ok(self
            .lost_void_gear_sight(frame, cancel, timeout)?
            .map(|s| s.view))
    }
    fn lost_void_gear_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        let started = Instant::now();
        let Some(scene) = self.lost_void_sight(frame, cancel, timeout)? else {
            return Ok(None);
        };
        let mut sight = Sight {
            view: LostVoidGearView {
                scene: scene.observation,
                page: None,
            },
            points: scene.points,
        };
        if sight.view.scene.screen != S::Gear {
            return Ok(Some(sight));
        }
        let result = (|| -> Result<()> {
            let r = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::lost_void_scene::pipeline(),
            };
            let mut names = words(&r.read("LostVoid.GearName", None)?)?;
            names.sort_by_key(|w| (w.center()[1] / 20, w.center()[0]));
            let mut hits = names
                .iter()
                .filter_map(|w| identify_artifact(&w.text).ok().flatten())
                .filter(valid_gear)
                .collect::<Vec<_>>();
            if hits.is_empty()
                && let Some(a) =
                    identify_artifact(&names.iter().map(|w| w.text.as_str()).collect::<String>())
                        .map_err(anyhow::Error::msg)?
                && valid_gear(&a)
            {
                hits.push(a);
            }
            hits.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
            hits.dedup();
            if hits.len() > 1 {
                return Ok(());
            }
            let slots = match detect_slots(frame) {
                Ok(s) => s,
                Err(_) => return Ok(()),
            };
            let enabled = words(&r.read("LostVoid.GearEnabled", None)?)?;
            let page = GearPage {
                slots: slots.iter().map(|(s, _)| s.clone()).collect(),
                name: hits.pop(),
                carry_enabled: enabled.iter().filter(|w| w.text == "携带").count() == 1,
                back: r.hit("LostVoid.Back", None)?,
            };
            if page.validate().is_err() {
                return Ok(());
            }
            for (s, [x, y, w, h]) in slots {
                sight
                    .points
                    .insert(format!("view:{}", s.slot), [x + w / 2, y + h / 2]);
            }
            sight.view.page = Some(page);
            Ok(())
        })();
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        result?;
        Ok(Some(sight))
    }
    pub fn read_lost_void_gear(
        &self,
        flow: &LostVoidGearFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidGearView>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_lost_void_scene()?;
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.lost_void_gear_sight(
            &frame.image,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        if frame.age() > Duration::from_millis(1500)
            || cancel.load(Ordering::Acquire)
            || started.elapsed() >= timeout
        {
            return Ok(None);
        }
        Ok(sight.map(|s| s.view))
    }
    pub fn act_lost_void_gear(
        &mut self,
        flow: &LostVoidGearFlow,
        action: GearAction,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<GearInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<GearInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_lost_void_scene()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(GearInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(GearInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(GearInputEnd::NotPosted);
            };
            let sight = self.lost_void_gear_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(GearInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(GearInputEnd::TimedOut);
            }
            let Some(sight) = sight else {
                return Ok(GearInputEnd::Changed);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.view) {
                return Ok(GearInputEnd::Changed);
            }
            let point = match action {
                GearAction::View { slot, .. } => sight.points.get(&format!("view:{slot}")).copied(),
                GearAction::Carry => sight.points.get("carry").copied(),
                GearAction::Confirm => sight.points.get("confirm").copied(),
                GearAction::Back => Some([111, 50]),
            };
            let Some([x, y]) = point else {
                return Ok(GearInputEnd::Changed);
            };
            let input = InputProbeAction::MouseButton {
                button: MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(GearInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(100)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            ensure!(
                self.combat_input(input, false, &AtomicBool::new(false))?,
                "武备输入未释放鼠标"
            );
            let wait = Instant::now();
            let settle = if matches!(action, GearAction::View { .. }) {
                500
            } else {
                300
            };
            while wait.elapsed() < Duration::from_millis(settle)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(GearInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|r| release.map(|_| r))
    }
}
