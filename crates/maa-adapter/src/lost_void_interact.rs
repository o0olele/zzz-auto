//! 抽奖与路径迭换输入；每次重新识别，输入结果由后续画面确认。
use crate::NativeSession;
pub use crate::lost_void_choice::ChoiceInputEnd as InteractInputEnd;
use anyhow::{Result, ensure};
use od_core::{
    InputProbeAction, MouseButton,
    lost_void_interact::{InteractAction, LostVoidInteractFlow},
    lost_void_scene::LostVoidObservation,
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
impl NativeSession {
    pub fn read_lost_void_interact(
        &self,
        flow: &LostVoidInteractFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidObservation>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_lost_void_scene()?;
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.lost_void_sight(
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
        Ok(sight.map(|s| s.observation))
    }
    pub fn act_lost_void_interact(
        &mut self,
        flow: &LostVoidInteractFlow,
        action: InteractAction,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<InteractInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<InteractInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_lost_void_scene()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(InteractInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(InteractInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(InteractInputEnd::NotPosted);
            };
            let sight = self.lost_void_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(InteractInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(InteractInputEnd::TimedOut);
            }
            let Some(sight) = sight else {
                return Ok(InteractInputEnd::Changed);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.observation)
            {
                return Ok(InteractInputEnd::Changed);
            }
            let point = match action {
                InteractAction::Start => sight.points.get("start").copied(),
                InteractAction::Back => Some([111, 50]),
            };
            let Some([x, y]) = point else {
                return Ok(InteractInputEnd::Changed);
            };
            let input = InputProbeAction::MouseButton {
                button: MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(InteractInputEnd::NotPosted);
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
                "迷失之地交互未释放鼠标"
            );
            let wait = Instant::now();
            while wait.elapsed() < Duration::from_millis(300)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(InteractInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|r| release.map(|_| r))
    }
}
