//! 导航采用固定 YOLO、战斗模板和同帧名牌；每次输入重新确认目标。
use crate::{
    NativeSession, RawRecognitionControl,
    combat_vision::{CombatFrame, CombatRecognizer},
    suibian_adventure::{Reader, words},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    InputProbeAction,
    lost_void_motion::{
        MOTION_CONFIRM_MS, MOTION_SAMPLE_MS, MOTION_UNKNOWN_MS, MotionAction, MotionDetector,
        MotionDirection, MotionResult,
    },
    lost_void_navigation::{NavAction, NavDetection, NavFlow, NavFocus, NavView},
    lost_void_npc::parse_npc_target,
    lost_void_scene::LostVoidScreen as S,
};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavInputEnd {
    MotionObserved { result: MotionResult },
    Posted,
    NotPosted,
    Changed,
    InterruptedMovement,
    TimedOut,
}
impl NativeSession {
    pub fn require_lost_void_navigation(&self, recognizer: &CombatRecognizer) -> Result<()> {
        self.require_lost_void_scene()?;
        let root = self
            .detector_bundle
            .as_deref()
            .context("导航须先载入固定迷失之地模型")?;
        crate::yolo::validate_challenge_detector(root)?;
        ensure!(
            self.combat_bundle.as_deref() == Some(recognizer.bundle.root.as_path()),
            "导航战斗识别器与当前资源不符"
        );
        crate::combat_vision::CombatBundle::load(&recognizer.bundle.root)?;
        Ok(())
    }
    pub fn inspect_lost_void_navigation(
        &self,
        frame: &CombatFrame,
        recognizer: &mut CombatRecognizer,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<NavView>> {
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        ensure!(
            self.detector_bundle.is_some()
                && self.combat_bundle.as_deref() == Some(recognizer.bundle.root.as_path()),
            "导航所需固定资源尚未载入"
        );
        if let Some(loading) = self.inspect_lost_void_loading(&frame.image, cancel, timeout)? {
            return Ok(Some(NavView {
                screen: S::Unknown,
                loading: Some(loading),
                detections: vec![],
                interact: false,
                attack: false,
                battle: false,
                focus: None,
            }));
        }
        let Some(scene) = self.inspect_lost_void_npc(
            &frame.image,
            false,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?
        else {
            return Ok(None);
        };
        let mut view = NavView {
            screen: scene.screen,
            loading: None,
            detections: vec![],
            interact: false,
            attack: false,
            battle: false,
            focus: None,
        };
        if scene.screen != S::World {
            return Ok(Some(view));
        }
        let control = RawRecognitionControl::default();
        let finished = AtomicBool::new(false);
        let result = std::thread::scope(|scope| {
            let watch = control.clone();
            let ended = &finished;
            scope.spawn(move || {
                while !ended.load(Ordering::Acquire) {
                    if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
                        let _ = watch.cancel();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            });
            let result = self.recognize_prepared_cancellable(
                &frame.image,
                "LegacyYoloDetect",
                &json!({"model":crate::yolo::LOST_VOID_MODEL,"threshold":0.6,"iou":0.5}),
                &control,
            );
            finished.store(true, Ordering::Release);
            result
        });
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let result = result?;
        for d in result.detail["detail"]["filtered"]
            .as_array()
            .context("导航模型没有完整结果")?
        {
            view.detections.push(NavDetection {
                label: d["label"].as_str().context("导航模型没有类别")?.into(),
                rect: serde_json::from_value(d["box"].clone())?,
                score: d["score"].as_f64().context("导航模型没有置信度")?,
            });
        }
        od_core::lost_void_navigation::validate_detections(&view.detections)
            .map_err(anyhow::Error::msg)?;
        view.interact = recognizer
            .matched(
                self,
                frame,
                "battle/btn_interact",
                recognizer.area("按键-交互")?,
                0.7,
            )?
            .is_some();
        view.attack = recognizer
            .matched(
                self,
                frame,
                "battle/btn_normal_attack",
                recognizer.area("按键-普通攻击")?,
                0.7,
            )?
            .is_some();
        let boss = recognizer
            .matched(self, frame, "battle/boss_life", [1620, 16, 1720, 106], 0.7)?
            .is_some();
        let reader = Reader {
            session: self,
            frame: &frame.image,
            cancel,
            started,
            timeout,
            pipeline: od_core::lost_void_scene::pipeline(),
        };
        let banners = words(&reader.read("LostVoid.NavHint", None)?)?;
        view.battle = boss
            || banners.iter().any(|w| {
                matches!(
                    w.text.trim_end_matches(['!', '！', '。']),
                    "战斗开始" | "侦测到最后的敌人"
                )
            });
        if view.interact {
            let labels = words(&reader.read("LostVoid.NavFocus", None)?)?;
            let marked = |s: &str| {
                (s.starts_with('>') && s.ends_with('<')) || (s.starts_with('<') && s.ends_with('>'))
            };
            let has_marked = labels
                .iter()
                .any(|w| marked(&w.text) && parse_npc_target(&w.text).is_some());
            let mut targets = BTreeSet::new();
            let mut found = None;
            for word in labels
                .into_iter()
                .filter(|w| !has_marked || marked(&w.text))
            {
                if let Some(target) = parse_npc_target(&word.text) {
                    targets.insert(target.clone());
                    found = Some(NavFocus {
                        target,
                        rect: word.rect,
                    });
                }
            }
            if targets.len() == 1 {
                view.focus = found;
            }
        }
        // 普攻按钮本身不能证明遭遇战斗。仅取闪光事件，不触发完整头像名册扫描。
        if view.attack && !view.battle && !view.interact && view.detections.is_empty() {
            recognizer.reset();
            let finished = AtomicBool::new(false);
            let handle = recognizer.cancellation_handle();
            let observed = std::thread::scope(|scope| {
                let ended = &finished;
                scope.spawn(move || {
                    while !ended.load(Ordering::Acquire) {
                        if (cancel.load(Ordering::Acquire) || started.elapsed() >= timeout)
                            && let Ok(raw) = handle.lock()
                            && let Some(raw) = raw.as_ref()
                        {
                            let _ = raw.cancel();
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                });
                let result = recognizer.observe(
                    self,
                    frame,
                    started.elapsed().as_secs_f64(),
                    &BTreeMap::from([
                        ("flash_enabled".into(), 1.0),
                        ("check_dodge_interval".into(), 0.0),
                    ]),
                    true,
                    cancel,
                );
                finished.store(true, Ordering::Release);
                result
            });
            if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
                return Ok(None);
            }
            view.battle = observed?
                .states
                .iter()
                .any(|s| !s.clear && matches!(s.name.as_str(), "闪避识别-红光" | "闪避识别-黄光"));
        }
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        view.validate().map_err(anyhow::Error::msg)?;
        Ok(Some(view))
    }
    pub fn read_lost_void_navigation(
        &self,
        flow: &NavFlow,
        recognizer: &mut CombatRecognizer,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<NavView>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let Some(frame) = self.combat_frame(cancel)? else {
            return Ok(None);
        };
        let view = self.inspect_lost_void_navigation(
            &frame,
            recognizer,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        if frame.age_ms() > 1500 || cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        Ok(view)
    }
    pub fn act_lost_void_navigation(
        &mut self,
        flow: &NavFlow,
        action: &NavAction,
        recognizer: &mut CombatRecognizer,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<NavInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<NavInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(NavInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(NavInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.combat_frame(cancel)? else {
                return Ok(NavInputEnd::NotPosted);
            };
            let view = self.inspect_lost_void_navigation(
                &frame,
                recognizer,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(NavInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(NavInputEnd::TimedOut);
            }
            let Some(view) = view else {
                return Ok(NavInputEnd::NotPosted);
            };
            if frame.age_ms() > 1500 || !flow.allows(action, &view) {
                return Ok(NavInputEnd::Changed);
            }
            let (input, ms) = match action {
                NavAction::Motion {
                    action: MotionAction::Probe { direction },
                } => {
                    return self.probe_lost_void_motion(
                        flow,
                        *direction,
                        recognizer,
                        cancel,
                        timeout.saturating_sub(started.elapsed()),
                        &frame,
                    );
                }
                NavAction::Motion {
                    action: MotionAction::ClearObstacle,
                } => (
                    flow.motion()
                        .attack_input()
                        .context("清障攻击没有冻结绑定")?,
                    200,
                ),
                NavAction::Turn { dx } => {
                    let scaled = (i64::from(*dx) * i64::from(flow.request().turn_sensitivity_milli)
                        / 1000) as i32;
                    return Ok(
                        if self
                            .combat_turn(if scaled == 0 { dx.signum() } else { scaled }, cancel)?
                        {
                            NavInputEnd::Posted
                        } else {
                            NavInputEnd::NotPosted
                        },
                    );
                }
                NavAction::Walk { key, ms, .. } => (InputProbeAction::Key { key: *key }, *ms),
                NavAction::Interact { .. } => (InputProbeAction::Key { key: 70 }, 200),
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(NavInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(u64::from(ms))
                && started.elapsed() < timeout
                && !cancel.load(Ordering::Acquire)
            {
                std::thread::sleep(Duration::from_millis(2));
            }
            let complete = held.elapsed() >= Duration::from_millis(u64::from(ms));
            self.combat_input(input, false, &AtomicBool::new(false))?;
            Ok(
                if complete
                    || matches!(
                        action,
                        NavAction::Interact { .. }
                            | NavAction::Motion {
                                action: MotionAction::ClearObstacle
                            }
                    )
                {
                    NavInputEnd::Posted
                } else {
                    NavInputEnd::InterruptedMovement
                },
            )
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
    /// 调用方已保存意图并核对新帧。持续采样时保持镜头不动，每张帧仍检查页面和窗口。
    fn probe_lost_void_motion(
        &mut self,
        flow: &NavFlow,
        direction: MotionDirection,
        recognizer: &mut CombatRecognizer,
        cancel: &AtomicBool,
        timeout: Duration,
        fresh: &CombatFrame,
    ) -> Result<NavInputEnd> {
        let started = Instant::now();
        for key in direction.keys() {
            if fresh.age_ms() > 1500
                || cancel.load(Ordering::Acquire)
                || started.elapsed() >= timeout
            {
                return Ok(NavInputEnd::InterruptedMovement);
            }
            if !self.combat_input(InputProbeAction::Key { key: *key }, true, cancel)? {
                return Ok(NavInputEnd::InterruptedMovement);
            }
        }
        let held = Instant::now();
        let mut last_sample = 0;
        let mut detector = MotionDetector::default();
        loop {
            if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
                return Ok(NavInputEnd::InterruptedMovement);
            }
            let elapsed = held.elapsed().as_millis() as u64;
            if elapsed.saturating_sub(last_sample) < MOTION_SAMPLE_MS {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            // 入口已核对父任务的实际目标；此私有持键段不更换会话或输入方式。
            self.check_ether_synthesis_target(self.game_window.is_some())?;
            if let Some(window) = &self.game_window {
                od_platform::check_keyboard_target(window, true)?;
            }
            let Some(frame) = self.combat_frame(cancel)? else {
                return Ok(NavInputEnd::InterruptedMovement);
            };
            let view = self.inspect_lost_void_navigation(
                &frame,
                recognizer,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if frame.age_ms() > 1500
                || cancel.load(Ordering::Acquire)
                || started.elapsed() >= timeout
            {
                return Ok(NavInputEnd::InterruptedMovement);
            }
            let Some(view) = view else {
                return Ok(NavInputEnd::InterruptedMovement);
            };
            if view.screen != S::World || view.interact || view.battle {
                return Ok(NavInputEnd::InterruptedMovement);
            }
            if !flow.motion().recovering()
                && od_core::lost_void_navigation::select_target(
                    &view.detections,
                    &flow.request().challenge,
                    flow.memory(),
                    &flow.request().ignored,
                    None,
                )
                .map_err(anyhow::Error::msg)?
                .is_none()
            {
                return Ok(NavInputEnd::InterruptedMovement);
            }
            let sample_ms = frame.captured.saturating_duration_since(held).as_millis() as u64;
            let pixels = frame
                .image
                .buffer
                .raw_data()
                .context("背景运动截图缺少像素")?;
            let result = detector
                .observe_bgr(pixels, sample_ms)
                .map_err(anyhow::Error::msg)?;
            if frame.age_ms() > 1500
                || cancel.load(Ordering::Acquire)
                || started.elapsed() >= timeout
            {
                return Ok(NavInputEnd::InterruptedMovement);
            }
            last_sample = sample_ms;
            if sample_ms >= MOTION_CONFIRM_MS
                && (result != MotionResult::Unknown || sample_ms >= MOTION_UNKNOWN_MS)
            {
                return Ok(NavInputEnd::MotionObserved { result });
            }
        }
    }
}
