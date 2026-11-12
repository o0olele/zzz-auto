//! 周常超时退出的菜单识别和投递前复验，复用冻结战斗模板及追猎 OCR。
use crate::{
    NativeSession,
    combat_vision::{CombatFrame, CombatRecognizer},
    deep_hunt::HuntInputEnd,
    suibian_adventure::{Reader, Word, point, words},
};
use anyhow::{Result, ensure};
use od_core::{
    notorious_hunt_recovery::WeeklyRecoveryView,
    notorious_hunt_timeout::{WeeklyTimeoutAction, WeeklyTimeoutFlow, WeeklyTimeoutView as V},
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

struct Sight {
    view: V,
    target: Option<[i32; 2]>,
}
impl NativeSession {
    fn weekly_timeout_sight(
        &self,
        frame: &CombatFrame,
        recognizer: &CombatRecognizer,
        expected: &str,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        ensure!(
            self.combat_bundle.as_deref() == Some(recognizer.bundle.root.as_path()),
            "周常超时退出识别器与当前战斗资源不符"
        );
        let result = (|| -> Result<Sight> {
            let r = Reader {
                session: self,
                frame: &frame.image,
                cancel,
                started,
                timeout,
                pipeline: od_core::notorious_hunt_timeout::pipeline(),
            };
            let read = |name: &str| words(&r.read(name, None)?);
            let enabled =
                |words: &[Word], text: &str, area: [i32; 4]| -> Result<Option<[i32; 2]>> {
                    let Some(target) = point(words, text, area) else {
                        return Ok(None);
                    };
                    let word = words
                        .iter()
                        .find(|w| w.center() == target)
                        .expect("唯一文字中心来自当前读数");
                    Ok(r.hit("Enabled", Some(word.rect))?.then_some(target))
                };
            let tabs = read("Tabs")?;
            let menu = point(&tabs, "战况", [1450, 26, 1600, 76]).is_some()
                && point(&tabs, "设置", [1646, 26, 1826, 76]).is_some()
                && self
                    .recognize_prepared(
                        &frame.image,
                        "TemplateMatch",
                        &serde_json::json!({
                            "roi":[82,13,68,77], "template":["deep-hunt/back.png"],
                            "threshold":0.7,"method":5,"green_mask":true
                        }),
                    )?
                    .detail["hit"]
                    == true;
            let confirm_words = read("Confirm")?;
            let confirm = enabled(&confirm_words, "确认", [1038, 600, 1224, 654])?;
            let cancel_button = point(&read("Cancel")?, "取消", [710, 584, 950, 684]);
            if confirm_words
                .iter()
                .any(|w| ["确认", "确定", "取消"].contains(&w.text.as_str()))
                || cancel_button.is_some()
            {
                let mut body = read("Question")?;
                body.sort_by_key(|w| (w.center()[1] / 20, w.rect[0]));
                let exit = od_core::lost_void_recovery_menu::confirmation_purpose(
                    &body.into_iter().map(|w| w.text).collect::<Vec<_>>(),
                ) == Some(od_core::lost_void_recovery_menu::RecoveryMode::Exit);
                return Ok(
                    if menu && confirm.is_some() && cancel_button.is_some() && exit {
                        Sight {
                            view: V::ConfirmExit,
                            target: confirm,
                        }
                    } else {
                        Sight {
                            view: V::Blocked,
                            target: None,
                        }
                    },
                );
            }
            if menu {
                let target = enabled(&read("Leave")?, "退出战斗", [1616, 1006, 1832, 1052])?;
                return Ok(Sight {
                    view: if target.is_some() {
                        V::Menu
                    } else {
                        V::Blocked
                    },
                    target,
                });
            }
            let recovery = self.inspect_weekly_hunt_recovery(
                &frame.image,
                expected,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            let view = match recovery {
                Some(WeeklyRecoveryView::Retreat) => V::Retreat,
                Some(WeeklyRecoveryView::Exit) => V::Exit,
                Some(WeeklyRecoveryView::Other | WeeklyRecoveryView::Returned { .. }) => V::Other,
                Some(WeeklyRecoveryView::Blocked) => V::Blocked,
                None => {
                    return Ok(Sight {
                        view: V::Unknown,
                        target: None,
                    });
                }
                Some(WeeklyRecoveryView::Unknown) => V::Unknown,
            };
            if view != V::Unknown {
                return Ok(Sight { view, target: None });
            }
            let area = recognizer.area("菜单")?;
            // 旧“菜单”区域只定义按钮内部点击点，宽 28px；完整模板宽 64px。
            // 匹配保留按钮周边，投递仍使用原有内部点击区域。
            let menu_search = [area[0] - 34, area[1] - 22, area[2] + 42, area[3] + 20];
            if recognizer
                .matched(self, frame, "battle/btn_menu", menu_search, 0.7)?
                .is_some()
            {
                return Ok(Sight {
                    view: V::Hud,
                    target: Some([(area[0] + area[2]) / 2, (area[1] + area[3]) / 2]),
                });
            }
            Ok(Sight {
                view: V::Unknown,
                target: None,
            })
        })();
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        result.map(Some)
    }
    pub fn read_weekly_hunt_timeout(
        &self,
        recognizer: &CombatRecognizer,
        expected: &str,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        let started = Instant::now();
        self.require_deep_hunt()?;
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let Some(frame) = self.combat_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.weekly_timeout_sight(
            &frame,
            recognizer,
            expected,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout || frame.age_ms() > 1500 {
            return Ok(None);
        }
        Ok(sight.map(|s| s.view))
    }
    pub fn act_weekly_hunt_timeout(
        &mut self,
        flow: &WeeklyTimeoutFlow,
        action: WeeklyTimeoutAction,
        recognizer: &CombatRecognizer,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<HuntInputEnd> {
        flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let result = (|| -> Result<HuntInputEnd> {
            self.require_deep_hunt()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(HuntInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(HuntInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.combat_frame(cancel)? else {
                return Ok(HuntInputEnd::NotPosted);
            };
            let sight = self.weekly_timeout_sight(
                &frame,
                recognizer,
                flow.mission_type(),
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(HuntInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(HuntInputEnd::TimedOut);
            }
            let Some(sight) = sight else {
                return Ok(HuntInputEnd::NotPosted);
            };
            if frame.age_ms() > 1500 || !flow.allows(action, sight.view) {
                return Ok(HuntInputEnd::Changed);
            }
            let Some([x, y]) = sight.target else {
                return Ok(HuntInputEnd::Changed);
            };
            let input = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(HuntInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(100)
                && started.elapsed() < timeout
                && !cancel.load(Ordering::Acquire)
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input, false, &AtomicBool::new(false))?;
            Ok(HuntInputEnd::Posted)
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}
