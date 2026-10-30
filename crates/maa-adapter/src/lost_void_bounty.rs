//! 悬赏领取的固定区域观察及新帧输入；不推断奖励实际到账。
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, Word, point, words},
};
use anyhow::Result;
use od_core::{
    InputProbeAction, MouseButton, lost_void_bounty::*, lost_void_scene::LostVoidScreen as S,
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BountyInputEnd {
    Posted,
    NotPosted,
    Changed,
    TimedOut,
    Unknown,
}
struct Sight {
    view: BountyView,
    points: BTreeMap<BountyAction, [i32; 2]>,
}
fn single<'a>(tokens: &'a [Word], text: &str) -> Option<&'a Word> {
    let mut matching = tokens.iter().filter(|w| w.text == text);
    let word = matching.next()?;
    matching.next().is_none().then_some(word)
}
fn same_word(tokens: &[Word], word: &Word) -> bool {
    let Some(other) = single(tokens, &word.text) else {
        return false;
    };
    let [x, y] = other.center();
    let [a, b] = word.center();
    (x - a).abs() <= 12 && (y - b).abs() <= 8
}
impl NativeSession {
    fn bounty_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        if timeout.is_zero() || cancel.load(Ordering::Acquire) {
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
                view: BountyView {
                    page: BountyPage::Unknown,
                    panel_ready: false,
                    claim: ClaimState::Unknown,
                    open_button: false,
                    close_button: false,
                    bounty: None,
                },
                points: BTreeMap::new(),
            };
            let confirm = read("LostVoid.ModalConfirm")?;
            if confirm
                .iter()
                .any(|w| ["确认", "确定"].contains(&w.text.as_str()))
            {
                let modal = read("LostVoid.Modal")?;
                if single(&modal, "取消").is_some() {
                    return Ok(sight);
                }
            }
            let reward = read("LostVoid.BountyRewardTitle")?;
            let dismiss = read("LostVoid.BountyRewardConfirmEnabled")?;
            if (single(&reward, "获得").is_some() || single(&reward, "获得道具").is_some())
                && let Some(p) = point(&dismiss, "确认", [882, 708, 1084, 758])
            {
                sight.view.page = BountyPage::Reward;
                sight.points.insert(BountyAction::DismissReward, p);
                return Ok(sight);
            }
            let title = read("LostVoid.BountyPanelTitle")?;
            let claim = read("LostVoid.BountyClaim")?;
            if single(&title, "悬赏委托").is_some()
                && let Some(word) = single(&claim, "全部领取")
            {
                // 同帧文字、固定布局及关闭图标共同确认完整面板；缺按钮不推导不可领。
                if reader.hit("LostVoid.BountyPanelClose", None)? {
                    sight.view.page = BountyPage::Panel;
                    sight.view.panel_ready = true;
                    sight.view.close_button = true;
                    sight.points.insert(BountyAction::ClosePanel, [1554, 223]);
                    let enabled = same_word(&read("LostVoid.BountyClaimEnabled")?, word);
                    let disabled = same_word(&read("LostVoid.BountyClaimDisabled")?, word);
                    sight.view.claim = match (enabled, disabled) {
                        (true, false) => ClaimState::Available,
                        (false, true) => ClaimState::Unavailable,
                        _ => ClaimState::Unknown,
                    };
                    if enabled && !disabled {
                        sight.points.insert(BountyAction::ClaimAll, word.center());
                    }
                }
                return Ok(sight);
            }
            if let Some(scene) =
                self.lost_void_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?
            {
                sight.view.page = match scene.observation.screen {
                    S::EntryStandard => BountyPage::EntryStandard,
                    S::EntryPeriodic => BountyPage::EntryPeriodic,
                    S::Handbook => BountyPage::Handbook,
                    S::NormalWorld => BountyPage::NormalWorld,
                    _ => BountyPage::Unknown,
                };
                if matches!(
                    sight.view.page,
                    BountyPage::EntryStandard | BountyPage::EntryPeriodic
                ) {
                    sight.view.bounty = scene.observation.bounty;
                    if let Some(p) = point(
                        &read("LostVoid.BountyOpen")?,
                        "悬赏委托",
                        [1181, 19, 1346, 65],
                    ) {
                        sight.view.open_button = true;
                        sight.points.insert(BountyAction::Open, p);
                    }
                    if reader.hit("LostVoid.Back", None)? {
                        sight.points.insert(BountyAction::BackEntry, [111, 50]);
                    }
                } else if sight.view.page == BountyPage::Handbook
                    && reader.hit("LostVoid.Back", None)?
                {
                    sight.points.insert(BountyAction::BackHandbook, [111, 50]);
                }
            }
            Ok(sight)
        })();
        if started.elapsed() >= timeout || cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let sight = result?;
        sight.view.validate().map_err(anyhow::Error::msg)?;
        Ok(Some(sight))
    }
    pub fn inspect_lost_void_bounty(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<BountyView>> {
        let started = Instant::now();
        self.require_lost_void_scene()?;
        Ok(self
            .bounty_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?
            .map(|s| s.view))
    }
    pub fn read_lost_void_bounty(
        &self,
        flow: &BountyFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<BountyView>> {
        let started = Instant::now();
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_lost_void_scene()?;
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.bounty_sight(
            &frame.image,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        if started.elapsed() >= timeout
            || cancel.load(Ordering::Acquire)
            || frame.age() > Duration::from_millis(1500)
        {
            return Ok(None);
        }
        Ok(sight.map(|s| s.view))
    }
    pub fn act_lost_void_bounty(
        &mut self,
        flow: &BountyFlow,
        action: BountyAction,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<BountyInputEnd> {
        use BountyInputEnd as E;
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
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(E::NotPosted);
            };
            let Some(sight) = self.bounty_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            else {
                return Ok(E::Changed);
            };
            if cancel.load(Ordering::Acquire) {
                return Ok(E::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(E::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.view) {
                return Ok(E::Changed);
            }
            let Some(&[x, y]) = sight.points.get(&action) else {
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
                && started.elapsed() < timeout
                && !cancel.load(Ordering::Acquire)
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
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}
