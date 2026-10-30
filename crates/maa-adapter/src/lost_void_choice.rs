//! 通用选择的同帧目录识别及输入；坐标只在输入前本帧有效。
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, Word, words},
};
use anyhow::{Result, ensure};
use od_core::{
    InputProbeAction, MouseButton,
    lost_void_choice::{
        ChoiceAction, ChoiceCard, ChoicePage, LostVoidChoiceFlow, LostVoidChoiceView,
        identify_artifact,
    },
    lost_void_scene::LostVoidScreen,
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceInputEnd {
    Posted,
    NotPosted,
    Changed,
    TimedOut,
}
struct ChoiceSight {
    view: LostVoidChoiceView,
    points: BTreeMap<String, [i32; 2]>,
}
impl NativeSession {
    pub fn inspect_lost_void_choice(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidChoiceView>> {
        self.require_lost_void_scene()?;
        Ok(self
            .lost_void_choice_sight(frame, cancel, timeout)?
            .map(|s| s.view))
    }
    fn lost_void_choice_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<ChoiceSight>> {
        let started = Instant::now();
        let Some(scene) = self.lost_void_sight(frame, cancel, timeout)? else {
            return Ok(None);
        };
        let mut sight = ChoiceSight {
            view: LostVoidChoiceView {
                scene: scene.observation,
                page: None,
            },
            points: scene.points,
        };
        if sight.view.scene.screen != LostVoidScreen::CommonChoice {
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
            let read = |n: &str| words(&r.read(&format!("LostVoid.{n}"), None)?);
            let mut names = read("CommonNames")?;
            // 检测器可能截掉名称首字；只对名称形状的单行补读，仍须完整匹配目录。
            let detected = names.clone();
            for word in &detected {
                if identify_artifact(&word.text)
                    .map_err(anyhow::Error::msg)?
                    .is_some()
                    || !word.text.starts_with(['[', '【', '「'])
                {
                    continue;
                }
                let [x, y, w, h] = word.rect;
                let left = (x - 12).max(80);
                let top = (y - 4).max(370);
                let right = (x + w + 12).min(1911);
                let bottom = (y + h + 4).min(610);
                let reread = words(&r.read(
                    "LostVoid.CommonNameLine",
                    Some([left, top, right - left, bottom - top]),
                )?)?;
                names.extend(reread);
            }
            let mut candidates = Vec::<(od_core::lost_void::ArtifactCandidate, Word)>::new();
            for word in &names {
                if let Some(a) = identify_artifact(&word.text).map_err(anyhow::Error::msg)? {
                    candidates.push((a, word.clone()));
                }
                // 分类和名称被检测器切成相邻块时，仅在同一行的小间隙内拼回完整名称。
                for right in &names {
                    let gap = right.rect[0] - word.rect[0] - word.rect[2];
                    if !(0..=35).contains(&gap) || (right.center()[1] - word.center()[1]).abs() > 14
                    {
                        continue;
                    }
                    if let Some(a) = identify_artifact(&format!("{}{}", word.text, right.text))
                        .map_err(anyhow::Error::msg)?
                    {
                        let mut w = word.clone();
                        w.rect[2] = right.rect[0] + right.rect[2] - w.rect[0];
                        candidates.push((a, w));
                    }
                }
            }
            candidates.sort_by_key(|(_, w)| w.center()[0]);
            let mut unique = Vec::<(od_core::lost_void::ArtifactCandidate, Word)>::new();
            for (a, w) in candidates {
                if let Some((old, old_word)) = unique
                    .last_mut()
                    .filter(|(_, old_word)| (old_word.center()[0] - w.center()[0]).abs() < 110)
                {
                    if old.category != a.category || old.name != a.name {
                        return Ok(());
                    }
                    if a.is_primary && !old.is_primary {
                        *old = a;
                        *old_word = w;
                    }
                } else {
                    unique.push((a, w));
                }
            }
            if unique.is_empty() || unique.len() > 8 {
                return Ok(());
            }
            if sight.view.scene.choice == Some(od_core::lost_void_scene::ChoiceKind::GearBranch) {
                for (artifact, word) in &mut unique {
                    let cx = word.center()[0];
                    let left = cx + 1;
                    let top = (word.rect[1] - 200).max(210);
                    let right = (cx + 230).min(1911);
                    let bottom = (word.rect[1] + 140).min(830);
                    let roi = [left, top, right - left, bottom - top];
                    let mut suffixes = Vec::new();
                    for (node, suffix) in [("BranchA", "a"), ("BranchB", "b")] {
                        let found = r.read(&format!("LostVoid.{node}"), Some(roi))?;
                        if found["hit"] != true {
                            continue;
                        }
                        let rect: [i32; 4] =
                            serde_json::from_value(found["detail"]["best"]["box"].clone())?;
                        if !od_core::intel_board::valid_rect(rect)
                            || rect[0] <= cx
                            || rect[0] + rect[2] > right
                        {
                            return Ok(());
                        }
                        suffixes.push(suffix);
                    }
                    if suffixes.len() != 1 {
                        return Ok(());
                    }
                    let Some(branch) = identify_artifact(&format!(
                        "[{}]{}-{}",
                        artifact.category, artifact.name, suffixes[0]
                    ))
                    .map_err(anyhow::Error::msg)?
                    else {
                        return Ok(());
                    };
                    *artifact = branch;
                }
            }
            let mut cards: Vec<_> = unique
                .iter()
                .enumerate()
                .map(|(i, (a, _))| {
                    let mut a = a.clone();
                    a.slot = i as u8;
                    ChoiceCard {
                        artifact: a,
                        selected: false,
                        blocked: false,
                    }
                })
                .collect();
            for marker in read("CommonMarkers")?
                .iter()
                .filter(|w| ["NEW!", "NEW", "已选择", "有同流派武备"].contains(&w.text.as_str()))
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
                let c = &mut cards[distances[0].0];
                match marker.text.as_str() {
                    "NEW!" | "NEW" => c.artifact.is_new = true,
                    "已选择" => c.selected = true,
                    _ => c.blocked = true,
                }
            }
            let enabled = read("CommonEnabled")?;
            let confirm_enabled = enabled
                .iter()
                .filter(|w| {
                    w.text == "确定"
                        || od_core::lost_void_scene::parse_selection_count(&w.text).is_some()
                })
                .count()
                == 1;
            let page = ChoicePage {
                kind: sight.view.scene.choice.expect("通用选择标题"),
                cards,
                count: sight.view.scene.selection_count,
                confirm_enabled,
            };
            // 已留档的无标题领奖只允许单个已知藏品和白色确认，不据此发起选择。
            if sight.view.scene.title.is_none()
                && (page.cards.len() != 1 || page.count.is_some() || !page.confirm_enabled)
            {
                return Ok(());
            }
            if page.validate().is_err() {
                return Ok(());
            }
            for (i, (_, w)) in unique.iter().enumerate() {
                sight.points.insert(format!("pick:{i}"), w.center());
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
    pub fn read_lost_void_choice(
        &self,
        flow: &LostVoidChoiceFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidChoiceView>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_lost_void_scene()?;
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.lost_void_choice_sight(
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
    pub fn act_lost_void_choice(
        &mut self,
        flow: &LostVoidChoiceFlow,
        action: ChoiceAction,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<ChoiceInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<ChoiceInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_lost_void_scene()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(ChoiceInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(ChoiceInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(ChoiceInputEnd::NotPosted);
            };
            let sight = self.lost_void_choice_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(ChoiceInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(ChoiceInputEnd::TimedOut);
            }
            let Some(sight) = sight else {
                return Ok(ChoiceInputEnd::Changed);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.view) {
                return Ok(ChoiceInputEnd::Changed);
            }
            let key = match action {
                ChoiceAction::Pick(s) => format!("pick:{s}"),
                ChoiceAction::Confirm => "confirm".into(),
            };
            let Some(&[x, y]) = sight.points.get(&key) else {
                return Ok(ChoiceInputEnd::Changed);
            };
            let input = InputProbeAction::MouseButton {
                button: MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(ChoiceInputEnd::NotPosted);
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
                "迷失之地选择未释放鼠标"
            );
            let wait = Instant::now();
            while wait.elapsed() < Duration::from_millis(300)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(ChoiceInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|r| release.map(|_| r))
    }
}
