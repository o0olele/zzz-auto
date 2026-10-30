//! 迷失之地恢复菜单：固定上下文、用途确认和新帧输入，未知投递不重发。
use crate::{
    NativeSession, RecognitionImage,
    combat_vision::CombatRecognizer,
    lost_void_roster::LostVoidHudReader,
    suibian_adventure::{Reader, Word, words},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    InputProbeAction, MouseButton, lost_void_recovery_menu::*, lost_void_scene::LostVoidScreen as S,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryInputEnd {
    Posted,
    NotPosted,
    Changed,
    TimedOut,
    Unknown,
}
struct Sight {
    view: RecoveryView,
    points: BTreeMap<RecoveryAction, [i32; 2]>,
}
fn named(ws: &[Word], text: &str) -> bool {
    ws.iter().any(|w| w.text == text)
}
fn unique(ws: &[Word], text: &str, area: [i32; 4]) -> Option<[i32; 2]> {
    let found: Vec<_> = ws
        .iter()
        .filter(|w| w.text == text && w.inside(area))
        .collect();
    (found.len() == 1).then(|| found[0].center())
}
fn page(screen: S) -> RecoveryPage {
    match screen {
        S::World => RecoveryPage::World,
        S::Gear | S::Event | S::CommonChoice => RecoveryPage::Initial(screen),
        S::BattleResult | S::BattleFail => RecoveryPage::Result(screen),
        _ => RecoveryPage::Unknown,
    }
}
impl NativeSession {
    fn recovery_hud(&self, frame: &RecognitionImage, r: &CombatRecognizer) -> Result<bool> {
        ensure!(
            self.combat_bundle.as_deref() == Some(r.bundle.root.as_path()),
            "恢复战斗识别器与当前资源不符"
        );
        for (key, area) in [
            ("battle/btn_normal_attack", "按键-普通攻击"),
            ("battle/agent_dead", "代理人阵亡"),
        ] {
            let Some(t) = r.bundle.spec.templates.get(key) else {
                continue;
            };
            let roi = od_core::Roi::from_legacy(r.area(area)?).map_err(anyhow::Error::msg)?;
            let path = t
                .image
                .strip_prefix("image/")
                .context("恢复战斗模板路径无效")?;
            if self.recognize_prepared(frame,"TemplateMatch",&json!({"roi":roi,"template":path,"threshold":0.7,"method":5,"green_mask":t.mask.is_some()}))?.detail["hit"]==true {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn recovery_sight(
        &self,
        frame: &RecognitionImage,
        recognizer: Option<&CombatRecognizer>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let started = Instant::now();
        let result = (|| -> Result<Sight> {
            let r = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::lost_void_scene::pipeline(),
            };
            let read = |name: &str| words(&r.read(&format!("LostVoid.{name}"), None)?);
            let mut s = Sight {
                view: RecoveryView {
                    page: RecoveryPage::Unknown,
                    menu_button: false,
                    members: None,
                    loading: None,
                },
                points: BTreeMap::new(),
            };
            let confirm = read("RecoveryConfirm")?;
            let cancel_words = read("RecoveryCancel")?;
            let modal = confirm.iter().chain(&cancel_words).any(|w| {
                ["确认", "确定", "取消", "确定并出战", "确定並出战"].contains(&w.text.as_str())
            });
            let tabs = read("RecoveryTabs")?;
            let menu =
                named(&tabs, "战况") && named(&tabs, "设置") && r.hit("LostVoid.Back", None)?;
            if modal {
                if menu
                    && unique(&cancel_words, "取消", [710, 584, 950, 684]).is_some()
                    && unique(&confirm, "确认", [1020, 584, 1260, 684]).is_some()
                {
                    let mut body = read("RecoveryQuestion")?;
                    body.sort_by_key(|w| (w.center()[1] / 20, w.rect[0]));
                    if let Some(mode) =
                        confirmation_purpose(&body.into_iter().map(|w| w.text).collect::<Vec<_>>())
                        && let Some(p) = unique(
                            &read("RecoveryConfirmEnabled")?,
                            "确认",
                            [1020, 584, 1260, 684],
                        )
                    {
                        s.view.page = RecoveryPage::Confirm(mode);
                        s.points.insert(RecoveryAction::Confirm, p);
                    }
                }
                return Ok(s);
            }
            if menu {
                let actions = read("RecoveryActionsEnabled")?;
                for (action, text, area) in [
                    (
                        RecoveryAction::Restart,
                        "重新开始",
                        [1288, 1002, 1514, 1054],
                    ),
                    (RecoveryAction::Exit, "退出战斗", [1616, 1006, 1832, 1052]),
                ] {
                    if let Some(p) = unique(&actions, text, area) {
                        s.points.insert(action, p);
                    }
                }
                s.view.page = RecoveryPage::Menu;
                return Ok(s);
            }
            let menu_hit = r.read("LostVoid.BattleMenu", None)?;
            if menu_hit["hit"] == true {
                let rect: [i32; 4] =
                    serde_json::from_value(menu_hit["detail"]["best"]["box"].clone())?;
                ensure!(
                    od_core::intel_board::valid_rect(rect),
                    "恢复菜单按钮范围无效"
                );
                let menu_point = [rect[0] + rect[2] / 2, rect[1] + rect[3] / 2];
                if r.hit("LostVoid.WorldTab", None)? {
                    s.view.page = RecoveryPage::World;
                    s.view.menu_button = true;
                    s.points.insert(RecoveryAction::OpenMenu, menu_point);
                    return Ok(s);
                }
                if let Some(scene) =
                    self.lost_void_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?
                {
                    s.view.page = page(scene.observation.screen);
                    if scene.observation.screen == S::Unknown
                        && let Some(recognizer) = recognizer
                        && self.recovery_hud(frame, recognizer)?
                    {
                        s.view.page = RecoveryPage::BattleHud;
                    }
                    if matches!(s.view.page, RecoveryPage::World | RecoveryPage::BattleHud) {
                        s.view.menu_button = true;
                        s.points.insert(RecoveryAction::OpenMenu, menu_point);
                    }
                    return Ok(s);
                }
            }
            let failure = read("FailureButtons")?;
            if named(&failure, "撤退") && named(&failure, "重播") {
                s.view.page = RecoveryPage::Result(S::BattleFail);
                return Ok(s);
            }
            if named(&read("ResultTitle")?, "挑战结果")
                || named(&read("ResultTitleLine")?, "挑战结果")
            {
                s.view.page = RecoveryPage::Result(S::BattleResult);
                return Ok(s);
            }
            if let Some(loading) = self.lost_void_loading_sight(
                frame,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )? {
                s.view.page = RecoveryPage::Loading;
                s.view.loading = Some(loading);
                return Ok(s);
            }
            if let Some(scene) =
                self.lost_void_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?
            {
                s.view.page = page(scene.observation.screen);
            }
            Ok(s)
        })();
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let sight = result?;
        sight.view.validate().map_err(anyhow::Error::msg)?;
        Ok(Some(sight))
    }
    pub fn inspect_lost_void_recovery(
        &self,
        frame: &RecognitionImage,
        recognizer: Option<&CombatRecognizer>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<RecoveryView>> {
        let started = Instant::now();
        self.require_lost_void_scene()?;
        let Some(mut s) = self.recovery_sight(
            frame,
            recognizer,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?
        else {
            return Ok(None);
        };
        if s.view.page == RecoveryPage::World
            && let Some(scene) =
                self.lost_void_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?
        {
            s.view.page = page(scene.observation.screen);
            if s.view.page != RecoveryPage::World {
                s.view.menu_button = false;
            }
        }
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        Ok(Some(s.view))
    }
    pub fn read_lost_void_recovery(
        &self,
        flow: &RecoveryFlow,
        reader: &mut LostVoidHudReader,
        recognizer: &CombatRecognizer,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<RecoveryView>> {
        let started = Instant::now();
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_lost_void_scene()?;
        let Some(frame) = self.combat_frame(cancel)? else {
            return Ok(None);
        };
        let Some(mut s) = self.recovery_sight(
            frame.recognition_image(),
            Some(recognizer),
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?
        else {
            return Ok(None);
        };
        if s.view.page == RecoveryPage::World
            && let Some(v) = self.inspect_lost_void_roster(
                &frame,
                reader,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
        {
            s.view.page = page(v.screen);
            if s.view.page == RecoveryPage::World {
                s.view.members = v.members;
            } else {
                s.view.menu_button = false;
            }
        }
        if s.view.page == RecoveryPage::Menu {
            let command =
                if flow.request().mode().map_err(anyhow::Error::msg)? == RecoveryMode::Restart {
                    RecoveryAction::Restart
                } else {
                    RecoveryAction::Exit
                };
            if !s.points.contains_key(&command) {
                s.view.page = RecoveryPage::Unknown;
            }
        }
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout || frame.age_ms() > 1500 {
            return Ok(None);
        }
        s.view.validate().map_err(anyhow::Error::msg)?;
        Ok(Some(s.view))
    }
    pub fn act_lost_void_recovery(
        &mut self,
        flow: &RecoveryFlow,
        action: RecoveryAction,
        recognizer: &CombatRecognizer,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<RecoveryInputEnd> {
        use RecoveryInputEnd as E;
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
            let Some(s) = self.recovery_sight(
                frame.recognition_image(),
                Some(recognizer),
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            else {
                return Ok(E::Changed);
            };
            if action == RecoveryAction::OpenMenu && s.view.page == RecoveryPage::World {
                let Some(scene) = self.lost_void_sight(
                    frame.recognition_image(),
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                )?
                else {
                    return Ok(E::Changed);
                };
                if scene.observation.screen != S::World {
                    return Ok(E::Changed);
                }
            }
            if cancel.load(Ordering::Acquire) {
                return Ok(E::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(E::TimedOut);
            }
            if frame.age_ms() > 1500 || !flow.allows(action, &s.view) {
                return Ok(E::Changed);
            }
            let Some(&[x, y]) = s.points.get(&action) else {
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
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}
