//! 迷失之地入口输入。每次输入前重读相同页面；不消费、不出战。
use crate::{
    NativeSession, RecognitionImage,
    lost_void_scene::LostVoidSight,
    suibian_adventure::{Reader, words},
};
use anyhow::{Context, Result};
use od_core::{
    InputProbeAction, MouseButton,
    lost_void_entry::{EntryAction as A, EntryOverlayKind, LostVoidEntryFlow},
    lost_void_scene::{LostVoidObservation, LostVoidScreen},
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryInputEnd {
    Posted,
    NotPosted,
    Changed,
    PartialScroll,
    TimedOut,
}
impl NativeSession {
    /// 入口专用观察；关闭模板不向局内、领取或通用场景执行器授予输入权限。
    fn lost_void_entry_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidSight>> {
        if timeout.is_zero() || cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let started = Instant::now();
        let reader = Reader {
            session: self,
            frame,
            cancel,
            started,
            timeout,
            pipeline: od_core::lost_void_scene::pipeline(),
        };
        let close = reader.read("LostVoid.EntryOverlayClose", None)?;
        if close["hit"] != true {
            return self.lost_void_sight(frame, cancel, timeout.saturating_sub(started.elapsed()));
        }
        let mut sight = LostVoidSight {
            observation: LostVoidObservation::default(),
            points: Default::default(),
            event: None,
        };
        // 多个关闭图标或越界模板不能选择一个猜测点击。
        let matches = close["detail"]["filtered"]
            .as_array()
            .context("入口关闭模板结果缺失")?;
        if matches.len() != 1 {
            return Ok(Some(sight));
        }
        let [x, y, w, h]: [i32; 4] = serde_json::from_value(matches[0]["box"].clone())?;
        if x < 1234 || y < 160 || w <= 0 || h <= 0 || x + w > 1600 || y + h > 350 {
            return Ok(Some(sight));
        }
        let read = |name: &str| words(&reader.read(name, None)?);
        let header = read("LostVoid.Header")?;
        let footer = read("LostVoid.Footer")?;
        let has = |tokens: &[crate::suibian_adventure::Word], text: &str| {
            tokens.iter().any(|w| w.text.contains(text))
        };
        if !has(&header, "周期") || !has(&header, "常规") || !has(&footer, "探索收集") {
            return Ok(Some(sight));
        }
        // 决策层优先于两种面板。悬赏文字与关闭图标可能仍透过上层确认框可见。
        let modal = read("LostVoid.Modal")?;
        if modal
            .iter()
            .any(|word| ["确认", "确定", "取消", "购买", "出战"].contains(&word.text.as_str()))
        {
            return Ok(Some(sight));
        }
        let title = read("LostVoid.BountyPanelTitle")?;
        let claim = read("LostVoid.BountyClaim")?;
        let bounty = has(&title, "悬赏委托") && has(&claim, "全部领取");
        let kind = if bounty {
            if x < 1526 || y < 198 || x + w > 1582 || y + h > 248 {
                return Ok(Some(sight));
            }
            EntryOverlayKind::Bounty
        } else {
            // 信息提示可以是图文内容，但已有决策或奖励控件不能当作可忽略提示。
            let reward = read("LostVoid.BountyRewardTitle")?;
            if has(&title, "悬赏")
                || has(&claim, "领取")
                || ["确认", "确定", "取消", "购买", "领取", "出战"]
                    .iter()
                    .any(|t| has(&modal, t))
                || has(&reward, "获得道具")
                || has(&reward, "获得奖励")
            {
                return Ok(Some(sight));
            }
            EntryOverlayKind::Information
        };
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        sight.observation.entry_overlay = Some(kind);
        sight
            .points
            .insert("entry_overlay_close".into(), [x + w / 2, y + h / 2]);
        Ok(Some(sight))
    }
    pub fn inspect_lost_void_entry(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidObservation>> {
        self.require_lost_void_scene()?;
        Ok(self
            .lost_void_entry_sight(frame, cancel, timeout)?
            .map(|s| s.observation))
    }
    pub fn read_lost_void_entry(
        &self,
        flow: &LostVoidEntryFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidObservation>> {
        let started = Instant::now();
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_lost_void_scene()?;
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let result = self.lost_void_entry_sight(
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
        Ok(result.map(|s| s.observation))
    }
    pub fn act_lost_void_entry(
        &mut self,
        flow: &LostVoidEntryFlow,
        action: A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<EntryInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<EntryInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_lost_void_scene()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(EntryInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(EntryInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(EntryInputEnd::NotPosted);
            };
            let sight = self.lost_void_entry_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(EntryInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(EntryInputEnd::TimedOut);
            }
            let Some(sight) = sight else {
                return Ok(EntryInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.observation)
            {
                return Ok(EntryInputEnd::Changed);
            }
            let key = match action {
                A::Open => "open".into(),
                A::CombatTab => "combat".into(),
                A::Category => format!(
                    "category:{}",
                    flow.category().context("迷失之地分类未冻结")?
                ),
                A::Go => "go".into(),
                A::Confirm => "confirm".into(),
                A::Standard => "standard".into(),
                A::Periodic => "periodic".into(),
                A::Mission => match flow.request().config.mission_name {
                    od_core::lost_void::LostVoidMission::Purge => "purge",
                    od_core::lost_void::LostVoidMission::TaskForce => "task_force",
                    od_core::lost_void::LostVoidMission::MatrixAction => "matrix",
                }
                .into(),
                A::Back => "back".into(),
                A::Scroll(_) => "scroll".into(),
                A::CloseOverlay => "entry_overlay_close".into(),
            };
            let mut target = match action {
                A::Open => Some([0, 0]),
                A::Back => Some([111, 50]),
                A::Scroll(_) => Some([573, 596]),
                _ => sight.points.get(&key).copied(),
            };
            let alt = InputProbeAction::Key { key: 18 };
            if action == A::Open {
                if !self.combat_input(alt, true, cancel)? {
                    return Ok(EntryInputEnd::NotPosted);
                }
                let Some(revealed) = self.navigation_frame(cancel)? else {
                    return Ok(EntryInputEnd::NotPosted);
                };
                let sight = self.lost_void_sight(
                    &revealed.image,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                )?;
                if cancel.load(Ordering::Acquire) {
                    return Ok(EntryInputEnd::NotPosted);
                }
                if started.elapsed() >= timeout {
                    return Ok(EntryInputEnd::TimedOut);
                }
                let Some(sight) = sight else {
                    return Ok(EntryInputEnd::Changed);
                };
                if revealed.age() > Duration::from_millis(1500)
                    || sight.observation.screen != LostVoidScreen::NormalWorld
                {
                    return Ok(EntryInputEnd::Changed);
                }
                target = sight.points.get("open").copied();
            }
            let Some([x, y]) = target else {
                return Ok(EntryInputEnd::Changed);
            };
            let input = |x, y| InputProbeAction::MouseButton {
                button: MouseButton::Left,
                x,
                y,
            };
            // 只支持前台键鼠和隔离控制器，和已有手册导航采用相同输入约束。
            if !self.combat_input(input(x, y), true, cancel)? {
                return Ok(EntryInputEnd::NotPosted);
            }
            let held = Instant::now();
            let duration = if matches!(action, A::Scroll(_)) {
                Duration::from_millis(300)
            } else {
                Duration::from_millis(100)
            };
            while held.elapsed() < duration
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                if matches!(action, A::Scroll(_)) {
                    self.combat_input(
                        input(x, y - held.elapsed().as_millis().min(300) as i32),
                        true,
                        cancel,
                    )?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input(x, y), false, &AtomicBool::new(false))?;
            if action == A::Open {
                self.combat_input(alt, false, &AtomicBool::new(false))?;
            }
            if matches!(action, A::Scroll(_)) && held.elapsed() < duration {
                return Ok(EntryInputEnd::PartialScroll);
            }
            let wait = Instant::now();
            while wait.elapsed() < Duration::from_millis(500)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(EntryInputEnd::Posted)
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}
