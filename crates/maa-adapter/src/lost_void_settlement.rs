//! 结算同帧观察；只有结果页对应按钮和新帧许可齐备才点击。
use crate::{
    NativeSession, RecognitionImage,
    combat_vision::CombatFrame,
    lost_void_roster::LostVoidHudReader,
    suibian_adventure::{Reader, Word, point, words},
};
use anyhow::Result;
use od_core::{
    InputProbeAction, MouseButton, lost_void_scene::LostVoidScreen as S, lost_void_settlement::*,
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementInputEnd {
    Posted,
    NotPosted,
    Changed,
    TimedOut,
    Unknown,
}
struct Sight {
    view: SettlementView,
    points: BTreeMap<&'static str, [i32; 2]>,
}
fn named(words: &[Word], text: &str) -> bool {
    words.iter().any(|w| w.text == text)
}
fn grid(words: &[Word]) -> Option<[RewardSlot; 8]> {
    if !words
        .iter()
        .any(|w| w.text == "阴影结晶数据" && w.inside([1480, 310, 1810, 420]))
    {
        return None;
    }
    let mut slots = [RewardSlot::Empty; 8];
    for (slot, out) in slots.iter_mut().enumerate() {
        let [x, y, w, h] = quantity_rect(slot)?;
        let candidates: Vec<_> = words
            .iter()
            .filter(|word| word.inside([x, y, x + w, y + h]))
            .collect();
        if candidates.len() != 1 {
            return None;
        }
        let text = &candidates[0].text;
        *out = if text == "EMPTY" {
            RewardSlot::Empty
        } else if !text.is_empty() && text.len() <= 9 && text.bytes().all(|c| c.is_ascii_digit()) {
            RewardSlot::Quantity(text.parse().ok()?)
        } else {
            return None;
        };
    }
    Some(slots)
}
impl NativeSession {
    pub fn inspect_lost_void_settlement(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<SettlementView>> {
        let started = Instant::now();
        self.require_lost_void_scene()?;
        let Some(mut sight) =
            self.settlement_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?
        else {
            return Ok(None);
        };
        if sight.view.page == SettlementPage::World
            && let Some(scene) =
                self.lost_void_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?
        {
            if content(scene.observation.screen) {
                sight.view.page = SettlementPage::Content(scene.observation.screen);
            } else if scene.observation.screen != S::World {
                sight.view.page = SettlementPage::Unknown;
            }
        }
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        Ok(Some(sight.view))
    }
    fn settlement_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let started = Instant::now();
        let result = (|| -> Result<Sight> {
            let reader = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::lost_void_scene::pipeline(),
            };
            let read = |name: &str| words(&reader.read(name, None)?);
            let mut sight = Sight {
                view: SettlementView {
                    page: SettlementPage::Unknown,
                    reward: None,
                    members: None,
                    bounty: None,
                },
                points: BTreeMap::new(),
            };
            let modal = read("LostVoid.ModalConfirm")?;
            let has_modal = named(&modal, "确认") || named(&modal, "确定");
            if !has_modal {
                if reader.hit("LostVoid.WorldTab", None)?
                    && reader.hit("LostVoid.BattleMenu", None)?
                {
                    sight.view.page = SettlementPage::World;
                    return Ok(sight);
                }
                let failure = read("LostVoid.FailureButtons")?;
                if named(&failure, "撤退") && named(&failure, "重播") {
                    if let Some(p) = point(&failure, "撤退", [400, 830, 900, 940]) {
                        sight.points.insert("retreat", p);
                        sight.view.page = SettlementPage::Failure;
                    }
                    return Ok(sight);
                }
                let title = read("LostVoid.ResultTitle")?;
                if named(&title, "挑战结果")
                    || named(&read("LostVoid.ResultTitleLine")?, "挑战结果")
                {
                    let confirm = read("LostVoid.ResultConfirm")?;
                    let finish = read("LostVoid.ResultFinish")?;
                    if let Some(p) = point(&confirm, "确定", [936, 855, 1338, 997]) {
                        sight.points.insert("confirm", p);
                    }
                    if let Some(p) = point(&finish, "完成", [1566, 1002, 1830, 1052]) {
                        sight.points.insert("finish", p);
                    }
                    let (confirm, finish) = (
                        sight.points.contains_key("confirm"),
                        sight.points.contains_key("finish"),
                    );
                    sight.view.page = SettlementPage::Result { confirm, finish };
                    if finish && !confirm {
                        let panel = read("LostVoid.ResultRewards")?;
                        sight.view.reward = Some(RewardRead {
                            evaluation: reader.hit("LostVoid.RewardEvaluation", None)?,
                            money: reader.hit("LostVoid.RewardMoney", None)?,
                            slots: grid(&panel),
                        });
                    }
                    return Ok(sight);
                }
            }
            if let Some(scene) =
                self.lost_void_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?
            {
                sight.view.page = match scene.observation.screen {
                    S::EntryPeriodic | S::EntryStandard => {
                        sight.view.bounty = scene.observation.bounty;
                        SettlementPage::Entry(scene.observation.screen)
                    }
                    s if content(s) => SettlementPage::Content(s),
                    S::Modal => SettlementPage::Modal,
                    _ => SettlementPage::Unknown,
                };
            }
            Ok(sight)
        })();
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let sight = result?;
        if sight.view.validate().is_err() {
            return Ok(None);
        }
        Ok(Some(sight))
    }
    pub fn read_lost_void_settlement(
        &self,
        flow: &SettlementFlow,
        reader: &mut LostVoidHudReader,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<SettlementView>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_lost_void_scene()?;
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let Some(frame) = self.combat_frame(cancel)? else {
            return Ok(None);
        };
        self.read_settlement_frame(
            flow,
            &frame,
            reader,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )
    }
    fn read_settlement_frame(
        &self,
        flow: &SettlementFlow,
        frame: &CombatFrame,
        reader: &mut LostVoidHudReader,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<SettlementView>> {
        let started = Instant::now();
        let sight = self.settlement_sight(frame.recognition_image(), cancel, timeout)?;
        let Some(mut sight) = sight else {
            return Ok(None);
        };
        if sight.view.page == SettlementPage::World
            && let Some(view) = self.inspect_lost_void_roster(
                frame,
                reader,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
        {
            if view.screen == S::World {
                sight.view.members = view
                    .members
                    .filter(|m| m.len() == usize::from(flow.request().expected_count));
            } else {
                sight.view.page = if content(view.screen) {
                    SettlementPage::Content(view.screen)
                } else if view.screen == S::Modal {
                    SettlementPage::Modal
                } else {
                    SettlementPage::Unknown
                };
            }
        }
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout || frame.age_ms() > 1500 {
            return Ok(None);
        }
        Ok(Some(sight.view))
    }
    pub fn act_lost_void_settlement(
        &mut self,
        flow: &SettlementFlow,
        action: SettlementAction,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<SettlementInputEnd> {
        use SettlementInputEnd as E;
        let started = Instant::now();
        let result = (|| -> Result<E> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_lost_void_scene()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(E::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(E::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.combat_frame(cancel)? else {
                return Ok(E::NotPosted);
            };
            let sight = self.settlement_sight(
                frame.recognition_image(),
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(E::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(E::TimedOut);
            }
            let Some(sight) = sight else {
                return Ok(E::Changed);
            };
            if frame.age_ms() > 1500 || !flow.allows(action, &sight.view) {
                return Ok(E::Changed);
            }
            let key = match action {
                SettlementAction::Confirm => "confirm",
                SettlementAction::Finish => "finish",
                SettlementAction::Retreat => "retreat",
            };
            let Some(&[x, y]) = sight.points.get(key) else {
                return Ok(E::Changed);
            };
            let input = InputProbeAction::MouseButton {
                button: MouseButton::Left,
                x,
                y,
            };
            if !matches!(self.combat_input(input, true, cancel), Ok(true)) {
                return Ok(E::Unknown);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(100)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            if !matches!(
                self.combat_input(input, false, &AtomicBool::new(false)),
                Ok(true)
            ) {
                return Ok(E::Unknown);
            }
            Ok(E::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|v| release.map(|_| v))
    }
}
