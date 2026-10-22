//! 同帧读取游历状态，动态位置只在实际输入前重新读取，不存进运行检查点。
use crate::{NativeSession, RecognitionImage};
use anyhow::{Context, Result, ensure};
use od_core::{
    suibian_adventure::{
        AdventureAction as A, AdventurePage, AdventureView as V, DispatchOutcome, TempleAdventure,
        parse_count, parse_district_label, parse_mission_label, parse_preview_duration,
    },
    suibian_temple::{AdventureDuration as D, AdventureMission},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdventureInputEnd {
    Posted,
    NotPosted,
    Changed,
    TimedOut,
}
#[derive(Clone)]
pub(crate) struct Word {
    pub(crate) text: String,
    pub(crate) rect: [i32; 4],
}
impl Word {
    pub(crate) fn center(&self) -> [i32; 2] {
        let [x, y, w, h] = self.rect;
        [x + w / 2, y + h / 2]
    }
    pub(crate) fn inside(&self, [l, t, r, b]: [i32; 4]) -> bool {
        let [x, y] = self.center();
        (l..r).contains(&x) && (t..b).contains(&y)
    }
}
struct Sight {
    view: V,
    points: BTreeMap<String, [i32; 2]>,
}
pub(crate) struct Reader<'a> {
    pub(crate) session: &'a NativeSession,
    pub(crate) frame: &'a RecognitionImage,
    pub(crate) cancel: &'a AtomicBool,
    pub(crate) started: Instant,
    pub(crate) timeout: Duration,
    pub(crate) pipeline: Value,
}
impl Reader<'_> {
    pub(crate) fn read(&self, name: &str, roi: Option<[i32; 4]>) -> Result<Value> {
        ensure!(!self.cancel.load(Ordering::Acquire), "游历识别已取消");
        ensure!(self.started.elapsed() < self.timeout, "游历识别超时");
        let mut params = self.pipeline[name].clone();
        let kind = params["recognition"]
            .as_str()
            .context("游历识别区域未登记")?
            .to_owned();
        params.as_object_mut().unwrap().remove("recognition");
        if let Some(roi) = roi {
            ensure!(
                od_core::intel_board::valid_rect(roi),
                "游历动态区域超出画面"
            );
            params["roi"] = json!(roi);
        }
        let value = self
            .session
            .recognize_prepared(self.frame, &kind, &params)?
            .detail;
        ensure!(!self.cancel.load(Ordering::Acquire), "游历识别已取消");
        ensure!(self.started.elapsed() < self.timeout, "游历识别超时");
        Ok(value)
    }
    pub(crate) fn hit(&self, name: &str, roi: Option<[i32; 4]>) -> Result<bool> {
        Ok(self.read(name, roi)?["hit"] == true)
    }
}
pub(crate) fn words(value: &Value) -> Result<Vec<Word>> {
    let list = value["detail"]["filtered"]
        .as_array()
        .context("游历 OCR 无结果字段")?;
    ensure!(list.len() <= 256, "游历 OCR 结果过多");
    list.iter()
        .map(|v| {
            let rect = serde_json::from_value(v["box"].clone())?;
            ensure!(od_core::intel_board::valid_rect(rect), "游历 OCR 区域无效");
            Ok(Word {
                text: v["text"]
                    .as_str()
                    .context("游历 OCR 缺少文字")?
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect(),
                rect,
            })
        })
        .collect()
}
pub(crate) fn has(words: &[Word], text: &str, area: [i32; 4]) -> bool {
    words
        .iter()
        .any(|w| w.inside(area) && w.text.contains(text))
}
pub(crate) fn point(words: &[Word], text: &str, area: [i32; 4]) -> Option<[i32; 2]> {
    let found: Vec<_> = words
        .iter()
        .filter(|w| w.inside(area) && w.text == text)
        .collect();
    (found.len() == 1).then(|| found[0].center())
}
fn durations() -> [D; 7] {
    [
        D::Min3,
        D::Min15,
        D::Hour1,
        D::Hour2,
        D::Hour6,
        D::Hour12,
        D::Hour20,
    ]
}

impl NativeSession {
    pub fn require_suibian_adventure(&self) -> Result<()> {
        let root = super::suibian_temple::validate_suibian_temple_bundle(
            self.suibian_temple_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(root.join("suibian-temple-manifest.json"))?)?;
        ensure!(
            matches!(manifest["schema_version"].as_u64(), Some(2..=8)),
            "游历需要 schema 2 随便观资源"
        );
        Ok(())
    }
    fn inspect_adventure_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        self.adventure_sight_with_texts(frame, cancel, timeout, None)
    }
    pub(crate) fn inspect_suibian_adventure_texts(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
        texts: Vec<Word>,
    ) -> Result<Option<V>> {
        Ok(self
            .adventure_sight_with_texts(frame, cancel, timeout, Some(texts))?
            .map(|s| s.view))
    }
    fn adventure_sight_with_texts(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
        texts: Option<Vec<Word>>,
    ) -> Result<Option<Sight>> {
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let result = (|| -> Result<Sight> {
            let r = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::suibian_adventure_pipeline::pipeline(),
            };
            let texts = match texts {
                Some(t) => t,
                None => words(&r.read("Adventure.Area.Text", None)?)?,
            };
            let mut points = BTreeMap::new();
            // 已知模态先于可能仍可见的背景。奖励必须是游历专有版式。
            if has(&texts, "游历完成", [700, 285, 1200, 370])
                && has(&texts, "探索进程", [550, 370, 1400, 500])
                && has(&texts, "物资", [550, 370, 1400, 500])
                && let Some(p) = point(&texts, "确认", [800, 750, 1130, 850])
            {
                points.insert("reward".into(), p);
                return Ok(Sight {
                    view: V::Reward,
                    points,
                });
            }
            if has(&texts, "游历时间选择", [480, 260, 1000, 350]) {
                let mut options = vec![];
                let mut selected = vec![];
                for d in durations() {
                    if let Some(p) = point(&texts, d.label(), [480, 480, 1445, 760]) {
                        options.push(d);
                        points.insert(d.label().into(), p);
                        let roi = [p[0] - 140, p[1] - 55, 280, 110];
                        if od_core::intel_board::valid_rect(roi)
                            && r.hit("Adventure.Area.DurationBorder", Some(roi))?
                        {
                            selected.push(d);
                        }
                    }
                }
                if let Some(p) = point(&texts, "确认", [800, 750, 1130, 850]) {
                    points.insert("duration".into(), p);
                }
                return Ok(Sight {
                    view: V::Duration {
                        options,
                        selected: if selected.len() == 1 {
                            Some(selected[0])
                        } else {
                            None
                        },
                    },
                    points,
                });
            }
            if has(&texts, "小队游历能力", [1280, 185, 1760, 275])
                && has(&texts, "派驻邦布", [200, 870, 600, 990])
                && has(&texts, "确认选择", [1400, 870, 1730, 990])
            {
                let ready = texts.iter().any(|w| {
                    w.inside([1260, 200, 1750, 270])
                        && w.text
                            .chars()
                            .filter(char::is_ascii_digit)
                            .collect::<String>()
                            .parse::<u32>()
                            .is_ok_and(|n| n > 0)
                });
                return Ok(Sight {
                    view: V::Bangboo { ready },
                    points,
                });
            }
            let confirm = point(&texts, "确认", [550, 450, 1400, 950]);
            for (text, reason) in [
                ("邦布电量不足", DispatchOutcome::InsufficientEnergy),
                ("没有可派遣小队", DispatchOutcome::NoTeams),
                ("可派遣小队已达上限", DispatchOutcome::NoTeams),
            ] {
                if has(&texts, text, [450, 250, 1450, 750])
                    && let Some(p) = confirm
                {
                    points.insert("rejection".into(), p);
                    return Ok(Sight {
                        view: V::Rejection(reason),
                        points,
                    });
                }
            }
            if confirm.is_some() {
                return Ok(Sight {
                    view: V::Unknown,
                    points,
                });
            }
            if has(&texts, "邻里街坊", [1300, 900, 1650, 1000])
                && has(&texts, "派驻邦布", [80, 720, 280, 820])
            {
                let managed = has(&texts, "自动托管中", [1200, 630, 1740, 790])
                    || has(&texts, "托管中", [80, 940, 280, 1040]);
                let manual = has(&texts, "经营", [1240, 570, 1430, 650])
                    && has(&texts, "游历", [1510, 570, 1680, 650]);
                return Ok(Sight {
                    view: if managed {
                        V::Managed
                    } else if manual {
                        V::Entry
                    } else {
                        V::Unknown
                    },
                    points,
                });
            }
            if !has(&texts, "街区", [230, 15, 410, 85]) || !r.hit("Hosting.Area.Back", None)? {
                return Ok(Sight {
                    view: V::Unknown,
                    points,
                });
            }
            let mut counts: Vec<_> = texts
                .iter()
                .filter(|w| w.inside([1500, 10, 1740, 90]))
                .filter_map(|w| parse_count(&w.text))
                .collect();
            if counts.is_empty() {
                counts = words(&r.read("Adventure.Area.Count", None)?)?
                    .iter()
                    .filter_map(|w| parse_count(&w.text))
                    .collect();
            }
            counts.sort();
            counts.dedup();
            let [count] = counts.as_slice() else {
                return Ok(Sight {
                    view: V::Unknown,
                    points,
                });
            };
            let mut page = AdventurePage {
                count: *count,
                ..Default::default()
            };
            page.squads_open = r.hit("Adventure.Area.ListOpen", None)?
                && r.hit("Adventure.Area.ListBody", None)?;
            if page.squads_open {
                let mut rows: Vec<_> = texts
                    .iter()
                    .filter(|w| w.inside([1420, 100, 1780, 700]))
                    .filter_map(|w| parse_mission_label(&w.text).map(|m| (m, w.center())))
                    .collect();
                rows.sort_by_key(|(_, p)| p[1]);
                page.squads_readable = rows.len() == usize::from(*count);
                for (mission, p) in rows {
                    let statuses: Vec<_> = texts
                        .iter()
                        .filter(|w| w.inside([1420, p[1] + 10, 1780, p[1] + 90]))
                        .collect();
                    if let Some(w) = statuses.iter().find(|w| w.text == "游历完成") {
                        page.completed.push(mission.clone());
                        points.insert(format!("completed:{mission}"), w.center());
                    } else if !statuses
                        .iter()
                        .any(|w| w.text.contains("剩余") || w.text == "游历中")
                    {
                        page.squads_readable = false;
                    }
                }
                return Ok(Sight {
                    view: V::Page(page),
                    points,
                });
            }
            page.panel = has(&texts, "探索进程", [1350, 190, 1770, 285])
                && has(&texts, "物资", [1350, 260, 1770, 330]);
            let map_right = if page.panel { 1220 } else { 1890 };
            let mut selected_titles = vec![];
            for (key, prefix) in [
                ("CRAFT", "制造区"),
                ("COMMUNITY", "社区旧址"),
                ("RESEARCH", "科研院旧址"),
            ] {
                for area in 1..=3 {
                    let title = format!("{prefix}{area}");
                    let found: Vec<_> = texts
                        .iter()
                        .filter(|w| {
                            parse_district_label(&w.text).as_deref()
                                == Some(format!("{key}_{area}").as_str())
                                && w.inside([0, 90, map_right, 1080])
                        })
                        .collect();
                    if found.len() != 1 {
                        continue;
                    }
                    let p = found[0].center();
                    if p[1] < 110 {
                        continue;
                    }
                    let left = [p[0] - 190, p[1] - 125, 45, 75];
                    let right = [p[0] + 145, p[1] - 125, 45, 75];
                    let target = [p[0], p[1] - 100];
                    if (10..map_right - 10).contains(&target[0]) && (100..1050).contains(&target[1])
                    {
                        page.visible_titles.push(title.clone());
                        points.insert(format!("mission:{title}"), target);
                    }
                    if page.panel
                        && od_core::intel_board::valid_rect(left)
                        && od_core::intel_board::valid_rect(right)
                        && r.hit("Adventure.Area.Marker", Some(left))?
                        && r.hit("Adventure.Area.Marker", Some(right))?
                    {
                        selected_titles.push(format!("{key}_{area}"));
                    }
                }
            }
            if page.panel && selected_titles.len() == 1 {
                let mut stages = vec![];
                for stage in 1..=4 {
                    if r.hit(&format!("Adventure.Area.Stage{stage}"), None)? {
                        stages.push(stage);
                    }
                }
                if stages.len() == 1 {
                    page.selected = Some(format!("{}_{}", selected_titles[0], stages[0]));
                }
            }
            if page.panel {
                page.claimable = point(&texts, "可收获", [1400, 900, 1700, 980]).is_some();
                page.running = !page.claimable
                    && point(&texts, "提前收获", [1400, 900, 1700, 980]).is_some()
                    && has(&texts, "游历中", [1560, 390, 1750, 480]);
                page.ready = !page.claimable
                    && !page.running
                    && point(&texts, "派遣", [1400, 900, 1700, 980]).is_some();
                page.energy_shortage = has(&texts, "邦布电量不足", [1220, 340, 1780, 990]);
                page.duration = texts
                    .iter()
                    .filter(|w| w.inside([1280, 390, 1575, 480]))
                    .find_map(|w| parse_preview_duration(&w.text));
                page.bangboo_ready = texts.iter().any(|w| {
                    w.inside([1240, 490, 1760, 625])
                        && w.text.contains('-')
                        && w.text.chars().filter(char::is_ascii_digit).count() >= 3
                }) && !has(&texts, "请选择邦布", [1240, 460, 1760, 625]);
                for (key, text) in [("claim", "可收获"), ("dispatch", "派遣")] {
                    if let Some(p) = point(&texts, text, [1400, 900, 1700, 980]) {
                        points.insert(key.into(), p);
                    }
                }
            }
            Ok(Sight {
                view: V::Page(page),
                points,
            })
        })();
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        result.map(Some)
    }
    pub fn inspect_suibian_adventure(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        Ok(self
            .inspect_adventure_sight(frame, cancel, timeout)?
            .map(|s| s.view))
    }
    pub fn read_suibian_adventure(
        &self,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        let started = Instant::now();
        self.require_suibian_adventure()?;
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let read = self.inspect_adventure_sight(
            &frame.image,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        Ok(read
            .filter(|_| frame.age() <= Duration::from_millis(1500))
            .map(|s| s.view))
    }
    pub fn act_suibian_adventure(
        &mut self,
        flow: &TempleAdventure,
        action: &A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<AdventureInputEnd> {
        let started = Instant::now();
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_suibian_adventure()?;
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "游历控制器模式不匹配"
        );
        let result = (|| -> Result<AdventureInputEnd> {
            if cancel.load(Ordering::Acquire) {
                return Ok(AdventureInputEnd::NotPosted);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(AdventureInputEnd::NotPosted);
            };
            let Some(sight) = self.inspect_adventure_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            else {
                return Ok(AdventureInputEnd::NotPosted);
            };
            if started.elapsed() >= timeout {
                return Ok(AdventureInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.view) {
                return Ok(AdventureInputEnd::Changed);
            }
            let point = match action {
                A::Open => Some([1595, 605]),
                A::Squads => Some([1630, 52]),
                A::OpenCompleted(m) => sight.points.get(&format!("completed:{m}")).copied(),
                A::SelectMission(m) => AdventureMission::parse(m)
                    .and_then(|m| sight.points.get(&format!("mission:{}", m.title())).copied()),
                A::SelectStage(m) => AdventureMission::parse(m).map(|m| {
                    [[1423, 160], [1525, 160], [1625, 160], [1725, 160]][usize::from(m.stage - 1)]
                }),
                A::OpenDuration => Some([1658, 438]),
                A::SelectDuration(d) => sight.points.get(d.label()).copied(),
                A::ConfirmDuration => sight.points.get("duration").copied(),
                A::AutoSelect => Some(if matches!(sight.view, V::Bangboo { .. }) {
                    [1330, 928]
                } else {
                    [1357, 939]
                }),
                A::ConfirmBangboo => Some([1540, 931]),
                A::Claim => sight.points.get("claim").copied(),
                A::ConfirmReward => sight.points.get("reward").copied(),
                A::Dispatch => sight.points.get("dispatch").copied(),
                A::CloseRejection => sight.points.get("rejection").copied(),
                A::ClosePanel => Some([1825, 270]),
                A::Back => Some(match sight.view {
                    V::Duration { .. } => [1380, 290],
                    V::Bangboo { .. } => [1825, 255],
                    _ => [116, 52],
                }),
                A::Pan(left) => Some(if *left { [1000, 800] } else { [200, 800] }),
            };
            let Some([x, y]) = point else {
                return Ok(AdventureInputEnd::Changed);
            };
            let input = |x, y| od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input(x, y), true, cancel)? {
                return Ok(AdventureInputEnd::NotPosted);
            }
            let held = Instant::now();
            let duration = if matches!(action, A::Pan(_)) {
                300
            } else {
                100
            };
            while held.elapsed() < Duration::from_millis(duration)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                if let A::Pan(left) = action {
                    let fraction = (held.elapsed().as_millis().min(300) * 800 / 300) as i32;
                    self.combat_input(
                        input(if *left { x - fraction } else { x + fraction }, y),
                        true,
                        cancel,
                    )?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input(x, y), false, &AtomicBool::new(false))?;
            Ok(AdventureInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|end| release.map(|_| end))
    }
}
