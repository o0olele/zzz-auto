//! 事件固定区域识别及同帧点击。对白期间持有 Alt，收尾始终释放。
pub use crate::lost_void_choice::ChoiceInputEnd as EventInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, Word, words},
};
use anyhow::{Result, ensure};
use od_core::{
    InputProbeAction, MouseButton,
    lost_void_event::{
        EventAction, EventOption, EventPage, EventPageKind, LostVoidEventFlow, LostVoidEventView,
        challenge_confirmation, dialogue_text, gear_complete_notice, known_narration,
    },
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(crate) struct EventRegion {
    pub page: Option<EventPage>,
    pub points: BTreeMap<String, [i32; 2]>,
}
fn joined(mut w: Vec<Word>) -> String {
    w.sort_by_key(|w| (w.center()[1] / 20, w.rect[0]));
    dialogue_text(&w.iter().map(|w| w.text.as_str()).collect::<String>())
}
fn boxes(value: &serde_json::Value) -> Result<Vec<[i32; 4]>> {
    let items = value["detail"]["filtered"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("事件模板缺少结果"))?;
    ensure!(items.len() <= 32, "事件模板结果过多");
    let mut out = Vec::new();
    for item in items {
        let rect: [i32; 4] = serde_json::from_value(item["box"].clone())?;
        ensure!(od_core::intel_board::valid_rect(rect), "事件模板超出画面");
        out.push(rect);
    }
    Ok(out)
}
fn white_text(frame: &RecognitionImage) -> Result<RecognitionImage> {
    ensure!(
        frame.buffer.width() == 1920
            && frame.buffer.height() == 1080
            && frame.buffer.image_type() == 16,
        "事件截图格式不匹配"
    );
    let bgr = frame
        .buffer
        .raw_data()
        .ok_or_else(|| anyhow::anyhow!("事件截图没有像素"))?;
    ensure!(bgr.len() == 1920 * 1080 * 3, "事件截图像素长度不匹配");
    let mut pixels = bgr.to_vec();
    for [left, top, width, height] in [
        [860, 798, 200, 48],
        [406, 878, 1060, 100],
        [1438, 536, 476, 294],
    ] {
        for y in top..top + height {
            for x in left..left + width {
                let keep = (-1..=0).any(|dy| {
                    (-1..=0).any(|dx| {
                        let (xx, yy) = (x as i32 + dx, y as i32 + dy);
                        if xx < left as i32 || yy < top as i32 {
                            return false;
                        }
                        let i = (yy as usize * 1920 + xx as usize) * 3;
                        bgr[i..i + 3].iter().all(|v| *v >= 200)
                    })
                });
                if !keep {
                    pixels[(y * 1920 + x) * 3..(y * 1920 + x) * 3 + 3].fill(0);
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
pub(crate) fn event_regions(r: &Reader<'_>) -> Result<Option<EventRegion>> {
    let challenge = words(&r.read("LostVoid.EventChallengeButton", None)?)?;
    if challenge.len() == 1 && challenge[0].text == "确认" {
        let text = joined(words(&r.read("LostVoid.EventPanelText", None)?)?);
        let kind = if challenge_confirmation(&text) {
            Some(EventPageKind::ChallengeConfirm)
        } else if gear_complete_notice(&text) {
            Some(EventPageKind::GearNotice)
        } else {
            None
        };
        if let Some(kind) = kind {
            return Ok(Some(EventRegion {
                page: Some(EventPage {
                    kind,
                    speaker: None,
                    text,
                    options: vec![],
                }),
                points: BTreeMap::from([("event:confirm".into(), challenge[0].center())]),
            }));
        }
        return Ok(Some(EventRegion {
            page: None,
            points: BTreeMap::new(),
        }));
    }
    let black = boxes(&r.read("LostVoid.EventBlackButton", None)?)?;
    if black.len() == 1 {
        let text = joined(words(
            &r.read("LostVoid.EventPanelText", Some([400, 170, 1120, 400]))?,
        )?);
        let [x, y, w, h] = black[0];
        return Ok(Some(EventRegion {
            page: Some(EventPage {
                kind: EventPageKind::BlackText,
                speaker: None,
                text: if text.is_empty() {
                    "黑屏文本继续".into()
                } else {
                    text
                },
                options: vec![],
            }),
            points: BTreeMap::from([("event:confirm".into(), [x + w / 2, y + h / 2])]),
        }));
    }
    let prepared = white_text(r.frame)?;
    let text_reader = Reader {
        session: r.session,
        frame: &prepared,
        cancel: r.cancel,
        started: r.started,
        timeout: r.timeout,
        pipeline: r.pipeline.clone(),
    };
    let speaker = joined(words(&text_reader.read("LostVoid.Speaker", None)?)?);
    let speaker = (!speaker.is_empty()
        && speaker.chars().count() <= 48
        && speaker.chars().any(char::is_alphabetic)
        && !["确定", "确认", "选择", "携带", "购买", "刷新"]
            .iter()
            .any(|s| speaker.contains(s)))
    .then_some(speaker);
    let text = joined(words(&text_reader.read("LostVoid.Dialogue", None)?)?);
    if text.is_empty() {
        return Ok(None);
    }
    let mut icons = boxes(&r.read("LostVoid.EventOptionIcon", None)?)?;
    icons.sort_by_key(|[x, y, w, h]| (y + h / 2, x + w / 2));
    if icons.len() > 8
        || icons
            .windows(2)
            .any(|p| (p[0][1] + p[0][3] / 2 - p[1][1] - p[1][3] / 2).abs() < 30)
    {
        return Ok(None);
    }
    if !known_narration(&text) && icons.is_empty() && speaker.is_none() {
        // 未知正文仍可能覆盖大世界；只阻止误判返回，不提供点击依据。
        if text.chars().filter(|c| c.is_alphabetic()).count() >= 3 {
            return Ok(Some(EventRegion {
                page: None,
                points: BTreeMap::new(),
            }));
        }
        return Ok(None);
    }
    let mut option_words = words(&text_reader.read("LostVoid.EventOptions", None)?)?;
    option_words.sort_by_key(|w| (w.center()[1], w.rect[0]));
    let mut rows = vec![Vec::<Word>::new(); icons.len()];
    if !icons.is_empty() {
        for word in option_words {
            let mut nearest: Vec<_> = icons
                .iter()
                .enumerate()
                .map(|(i, icon)| (i, (icon[1] + icon[3] / 2 - word.center()[1]).abs()))
                .collect();
            nearest.sort_by_key(|(_, d)| *d);
            if nearest[0].1 > 70 || nearest.get(1).is_some_and(|(_, d)| *d - nearest[0].1 < 10) {
                return Ok(None);
            }
            if word.center()[0] > icons[nearest[0].0][0] + icons[nearest[0].0][2] / 2 {
                rows[nearest[0].0].push(word);
            }
        }
    } else {
        for word in option_words {
            if let Some(row) = rows.last_mut().filter(|row| {
                row.last()
                    .is_some_and(|old| (old.center()[1] - word.center()[1]).abs() < 38)
            }) {
                row.push(word);
            } else {
                rows.push(vec![word]);
            }
        }
    }
    if rows.len() > 8 {
        return Ok(None);
    }
    let mut options = Vec::new();
    let mut points = BTreeMap::new();
    for (i, row) in rows.into_iter().enumerate() {
        let point = if let Some(icon) = icons.get(i) {
            [icon[0] + icon[2] / 2, icon[1] + icon[3] / 2]
        } else {
            row[0].center()
        };
        let name = joined(row);
        options.push(EventOption {
            slot: i as u8,
            text: (!name.is_empty()).then_some(name),
            icon: !icons.is_empty(),
        });
        points.insert(format!("event:option:{i}"), point);
    }
    points.insert(
        "event:advance".into(),
        if speaker.is_some() {
            [960, 872]
        } else {
            [936, 928]
        },
    );
    let page = EventPage {
        kind: EventPageKind::Dialogue,
        speaker,
        text,
        options,
    };
    if page.validate().is_err() {
        return Ok(None);
    }
    Ok(Some(EventRegion {
        page: Some(page),
        points,
    }))
}
impl NativeSession {
    pub fn inspect_lost_void_event(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidEventView>> {
        let started = Instant::now();
        self.require_lost_void_scene()?;
        if let Some(loading) =
            self.lost_void_loading_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?
        {
            return Ok(Some(LostVoidEventView {
                scene: Default::default(),
                page: None,
                loading: Some(loading),
            }));
        }
        Ok(self
            .lost_void_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?
            .map(|s| LostVoidEventView {
                scene: s.observation,
                page: s.event,
                loading: None,
            }))
    }
    pub fn read_lost_void_event(
        &self,
        flow: &LostVoidEventFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidEventView>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let view = self.inspect_lost_void_event(
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
        Ok(view)
    }
    pub fn act_lost_void_event(
        &mut self,
        flow: &LostVoidEventFlow,
        action: EventAction,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<EventInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<EventInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_lost_void_scene()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(EventInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(EventInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            if !self.combat_input(InputProbeAction::Key { key: 18 }, true, cancel)? {
                return Ok(EventInputEnd::NotPosted);
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(EventInputEnd::NotPosted);
            };
            let sight = self.lost_void_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(EventInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(EventInputEnd::TimedOut);
            }
            let Some(sight) = sight else {
                return Ok(EventInputEnd::Changed);
            };
            let view = LostVoidEventView {
                scene: sight.observation,
                page: sight.event,
                loading: None,
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &view) {
                return Ok(EventInputEnd::Changed);
            }
            let key = match action {
                EventAction::Advance => "event:advance".into(),
                EventAction::Choose(i) => format!("event:option:{i}"),
                EventAction::Confirm => "event:confirm".into(),
            };
            let Some(&[x, y]) = sight.points.get(&key) else {
                return Ok(EventInputEnd::Changed);
            };
            let input = InputProbeAction::MouseButton {
                button: MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(EventInputEnd::NotPosted);
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
                "迷失之地事件未释放鼠标"
            );
            let wait = Instant::now();
            while wait.elapsed() < Duration::from_millis(300)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(EventInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|end| release.map(|_| end))
    }
}
