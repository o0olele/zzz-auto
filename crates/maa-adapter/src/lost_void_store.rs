//! 商店同列价格与按钮配对，输入前核对完整交易页面。
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, Word, words},
};
use anyhow::{Result, ensure};
use od_core::{
    InputProbeAction, MouseButton,
    lost_void_choice::identify_artifact,
    lost_void_scene::LostVoidScreen as S,
    lost_void_store::{
        LostVoidStoreFlow, LostVoidStoreView, StoreAction, StoreCurrency, StoreDialog, StoreOffer,
        StorePage, StorePurpose, parse_store_number,
    },
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
pub fn prepare_lost_void_store_text(frame: &RecognitionImage) -> Result<RecognitionImage> {
    ensure!(
        frame.buffer.width() == 1920
            && frame.buffer.height() == 1080
            && frame.buffer.image_type() == 16,
        "商店截图格式不匹配"
    );
    let bgr = frame
        .buffer
        .raw_data()
        .ok_or_else(|| anyhow::anyhow!("商店截图没有像素"))?;
    ensure!(bgr.len() == 1920 * 1080 * 3, "商店截图像素长度不匹配");
    let mut pixels = bgr.to_vec();
    for [left, top, width, height] in [
        [90, 735, 1830, 105],
        [1480, 998, 150, 60],
        [1678, 1002, 152, 48],
    ] {
        for y in top..top + height {
            for x in left..left + width {
                // 对白字掩码膨胀两像素核后保留原色，包含细字的灰色边缘而不带入原价。
                let keep = (-1..=0).any(|dy| {
                    (-1..=0).any(|dx| {
                        let xx = x as i32 + dx;
                        let yy = y as i32 + dy;
                        if xx < left as i32 || yy < top as i32 {
                            return false;
                        }
                        let i = (yy as usize * 1920 + xx as usize) * 3;
                        bgr[i..i + 3].iter().all(|v| *v >= 200)
                    })
                });
                if !keep {
                    let i = (y * 1920 + x) * 3;
                    pixels[i..i + 3].fill(0);
                }
            }
        }
    }
    let mut buffer = maa_framework::buffer::MaaImageBuffer::new()?;
    buffer.set_raw_data(&pixels, 1920, 1080, 16)?;
    Ok(RecognitionImage {
        buffer,
        prepare_us: 0,
    })
}
fn bright_count(frame: &RecognitionImage, [x, y, w, h]: [i32; 4]) -> usize {
    let Some(pixels) = frame.buffer.raw_data() else {
        return 0;
    };
    let mut count = 0;
    for yy in y..y + h {
        for xx in x..x + w {
            let i = (yy as usize * 1920 + xx as usize) * 3;
            count += usize::from(pixels[i..i + 3].iter().all(|v| *v >= 200));
        }
    }
    count
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreInputEnd {
    Posted,
    NotPosted,
    Changed,
    PartialScroll,
    TimedOut,
}
struct Sight {
    view: LostVoidStoreView,
    points: BTreeMap<String, [i32; 2]>,
}
fn unique_number(words: &[Word]) -> Option<u32> {
    let values: Vec<_> = words
        .iter()
        .filter_map(|w| parse_store_number(&w.text))
        .collect();
    (values.len() == 1).then(|| values[0])
}
fn health_values(words: &[Word]) -> Vec<u8> {
    let mut values = Vec::new();
    for w in words {
        let text = w.text.replace('％', "%");
        let parts: Vec<_> = if text.contains('%') {
            text.split('%').filter(|s| !s.is_empty()).collect()
        } else {
            vec![text.as_str()]
        };
        for p in parts {
            if let Some(v) = parse_store_number(p)
                && v <= 100
            {
                values.push(v as u8);
            }
        }
    }
    values
}
impl NativeSession {
    pub fn inspect_lost_void_store(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidStoreView>> {
        self.require_lost_void_scene()?;
        Ok(self
            .lost_void_store_sight(frame, cancel, timeout)?
            .map(|s| s.view))
    }
    fn lost_void_store_sight(
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
            view: LostVoidStoreView {
                scene: scene.observation,
                page: None,
                dialog: None,
            },
            points: scene.points,
        };
        if ![S::Store, S::Modal].contains(&sight.view.scene.screen) {
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
            let read = |name: &str| words(&r.read(&format!("LostVoid.{name}"), None)?);
            if sight.view.scene.screen == S::Modal {
                let modal = read("Modal")?;
                let text = modal.iter().map(|w| w.text.as_str()).collect::<String>();
                let purpose = match (text.contains("购买"), text.contains("刷新")) {
                    (true, false) => Some(StorePurpose::Buy),
                    (false, true) => Some(StorePurpose::Refresh),
                    _ => None,
                };
                let confirm = modal
                    .iter()
                    .filter(|w| w.text == "确认" && w.inside([1034, 600, 1230, 654]))
                    .collect::<Vec<_>>();
                if let Some(purpose) = purpose
                    && confirm.len() == 1
                {
                    let digits = regex::Regex::new(r"[0-9]{1,8}")?;
                    let costs = digits
                        .find_iter(&text)
                        .filter_map(|s| s.as_str().parse::<u32>().ok())
                        .filter(|c| *c <= 1_000_000)
                        .collect::<Vec<_>>();
                    if !costs.is_empty() && costs.len() <= 16 {
                        sight.view.dialog = Some(StoreDialog { purpose, costs });
                        sight.points.insert("confirm".into(), confirm[0].center());
                    }
                }
                return Ok(());
            }
            let gold = r.hit("LostVoid.StoreGold", None)?;
            let blood = r.hit("LostVoid.StoreBlood", None)?;
            let currency = match (gold, blood) {
                (true, false) => Some(StoreCurrency::Gold),
                (false, true) => Some(StoreCurrency::Blood),
                _ => None,
            };
            let mut names = read("StoreNames")?;
            let detected = names.clone();
            for w in &detected {
                if identify_artifact(&w.text)
                    .map_err(anyhow::Error::msg)?
                    .is_some()
                {
                    continue;
                }
                if !w.text.starts_with(['[', '【', '「']) {
                    continue;
                }
                let [x, y, w, h] = w.rect;
                let left = (x - 10).max(90);
                let top = (y - 3).max(360);
                let right = (x + w + 10).min(1910);
                let bottom = (y + h + 3).min(540);
                names.extend(words(&r.read(
                    "LostVoid.CommonNameLine",
                    Some([left, top, right - left, bottom - top]),
                )?)?);
            }
            let mut candidates = Vec::<(od_core::lost_void::ArtifactCandidate, Word)>::new();
            for w in &names {
                if let Some(a) = identify_artifact(&w.text).map_err(anyhow::Error::msg)? {
                    candidates.push((a, w.clone()));
                }
                for right in &names {
                    let gap = right.rect[0] - w.rect[0] - w.rect[2];
                    if !(0..=35).contains(&gap) || (right.center()[1] - w.center()[1]).abs() > 14 {
                        continue;
                    }
                    if let Some(a) = identify_artifact(&format!("{}{}", w.text, right.text))
                        .map_err(anyhow::Error::msg)?
                    {
                        let mut combined = w.clone();
                        combined.rect[2] = right.rect[0] + right.rect[2] - w.rect[0];
                        candidates.push((a, combined));
                    }
                }
            }
            candidates.sort_by_key(|(_, w)| w.center()[0]);
            let mut unique = Vec::<(od_core::lost_void::ArtifactCandidate, Word)>::new();
            for (a, w) in candidates {
                if let Some((old, _)) = unique
                    .last()
                    .filter(|(_, old)| (old.center()[0] - w.center()[0]).abs() < 110)
                {
                    if old.category != a.category || old.name != a.name {
                        return Ok(());
                    }
                } else {
                    unique.push((a, w));
                }
            }
            if unique.is_empty() || unique.len() > 8 {
                return Ok(());
            }
            for marker in read("CommonMarkers")?
                .iter()
                .filter(|w| ["NEW", "NEW!"].contains(&w.text.as_str()))
            {
                let mut distances: Vec<_> = unique
                    .iter()
                    .enumerate()
                    .map(|(i, (_, w))| (i, (w.center()[0] - marker.center()[0]).abs()))
                    .collect();
                distances.sort_by_key(|(_, d)| *d);
                if distances[0].1 > 180
                    || distances
                        .get(1)
                        .is_some_and(|(_, d)| *d - distances[0].1 < 30)
                {
                    return Ok(());
                }
                unique[distances[0].0].0.is_new = true;
            }
            let prepared = prepare_lost_void_store_text(frame)?;
            let text_reader = Reader {
                session: self,
                frame: &prepared,
                cancel,
                started,
                timeout,
                pipeline: od_core::lost_void_scene::pipeline(),
            };
            let special = read("StoreSpecial")?.iter().any(|w| w.text == "零号严选");
            let mut offers = Vec::new();
            for (i, (mut a, w)) in unique.into_iter().enumerate() {
                a.slot = i as u8;
                let cx = w.center()[0];
                let left = (cx - if cx < 490 { 60 } else { 20 }).max(90);
                let right = (cx + 70).min(1920);
                let price_words = words(&text_reader.read(
                    "LostVoid.StorePriceLine",
                    Some([left, 735, right - left, 45]),
                )?)?;
                let price = unique_number(&price_words);
                let left = (cx - 100).max(90);
                let right = (cx + 100).min(1910);
                let buy_roi = [left, 780, right - left, 60];
                let buy = words(&text_reader.read("LostVoid.StoreBuyLine", Some(buy_roi))?)?;
                let available =
                    bright_count(frame, buy_roi) >= 30 && buy.len() == 1 && buy[0].text == "购买";
                if available {
                    sight
                        .points
                        .insert(format!("buy:{i}"), [left + (right - left) / 2, 810]);
                }
                offers.push(StoreOffer {
                    artifact: a,
                    price,
                    available,
                    special: special && cx < 490,
                });
            }
            let balance = if currency == Some(StoreCurrency::Gold) {
                unique_number(&read("StoreBalance")?)
            } else {
                None
            };
            let health = if currency == Some(StoreCurrency::Blood) {
                health_values(&read("StoreHealth")?)
            } else {
                vec![]
            };
            let refresh = words(&text_reader.read("LostVoid.StoreRefreshLine", None)?)?;
            let refreshes = refresh
                .iter()
                .filter(|w| w.text == "刷新")
                .collect::<Vec<_>>();
            if refreshes.len() == 1 {
                sight.points.insert("refresh".into(), refreshes[0].center());
            }
            let page = StorePage {
                currency,
                balance,
                health,
                offers,
                refresh_price: unique_number(&words(
                    &text_reader.read("LostVoid.StoreCostLine", None)?,
                )?),
                refresh_enabled: refreshes.len() == 1
                    && bright_count(frame, [1678, 1002, 152, 48]) >= 30,
                back: r.hit("LostVoid.Back", None)?,
            };
            if page.validate().is_ok() {
                sight.view.page = Some(page);
            }
            Ok(())
        })();
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        result?;
        Ok(Some(sight))
    }
    pub fn read_lost_void_store(
        &self,
        flow: &LostVoidStoreFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidStoreView>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_lost_void_scene()?;
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.lost_void_store_sight(
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
    pub fn act_lost_void_store(
        &mut self,
        flow: &LostVoidStoreFlow,
        action: StoreAction,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<StoreInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<StoreInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_lost_void_scene()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(StoreInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(StoreInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(StoreInputEnd::NotPosted);
            };
            let sight = self.lost_void_store_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(StoreInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(StoreInputEnd::TimedOut);
            }
            let Some(sight) = sight else {
                return Ok(StoreInputEnd::Changed);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.view) {
                return Ok(StoreInputEnd::Changed);
            }
            let target = match action {
                StoreAction::Buy(slot) => sight.points.get(&format!("buy:{slot}")).copied(),
                StoreAction::Refresh => sight.points.get("refresh").copied(),
                StoreAction::Confirm => sight.points.get("confirm").copied(),
                StoreAction::Scroll => Some([960, 540]),
                StoreAction::Back => Some([111, 50]),
            };
            let Some([x, y]) = target else {
                return Ok(StoreInputEnd::Changed);
            };
            let input = |x, y| InputProbeAction::MouseButton {
                button: MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input(x, y), true, cancel)? {
                return Ok(StoreInputEnd::NotPosted);
            }
            let held = Instant::now();
            let duration = if action == StoreAction::Scroll {
                300
            } else {
                100
            };
            while held.elapsed() < Duration::from_millis(duration)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                if action == StoreAction::Scroll {
                    let dx = (held.elapsed().as_millis().min(300) * 400 / 300) as i32;
                    self.combat_input(input(x - dx, y), true, cancel)?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            ensure!(
                self.combat_input(input(x, y), false, &AtomicBool::new(false))?,
                "商店输入未释放鼠标"
            );
            if action == StoreAction::Scroll && held.elapsed() < Duration::from_millis(duration) {
                return Ok(StoreInputEnd::PartialScroll);
            }
            let wait = Instant::now();
            while wait.elapsed() < Duration::from_millis(500)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(StoreInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|r| release.map(|_| r))
    }
}
