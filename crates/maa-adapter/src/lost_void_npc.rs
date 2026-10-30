//! NPC 返回后的固定移动，输入前重新识别世界页面，持有时长可恢复。
use crate::suibian_adventure::{Reader, words};
use crate::{NativeSession, RawRecognitionControl, RecognitionImage};
use anyhow::{Context, Result, ensure};
use od_core::{
    InputProbeAction,
    lost_void_npc::{NpcMoveAction, NpcMoveFlow, NpcMoveView},
};
use serde_json::json;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpcInputEnd {
    Held(u32),
    NotPosted,
    Changed,
    TimedOut,
}
impl NativeSession {
    pub fn inspect_lost_void_npc(
        &self,
        frame: &RecognitionImage,
        need_icons: bool,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<NpcMoveView>> {
        self.require_lost_void_scene()?;
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let reader = Reader {
            session: self,
            frame,
            cancel,
            started,
            timeout,
            pipeline: od_core::lost_void_scene::pipeline(),
        };
        let modal = words(&reader.read("LostVoid.ModalConfirm", None)?)?;
        if modal
            .iter()
            .any(|w| ["确认", "确定", "取消"].contains(&w.text.as_str()))
        {
            return Ok(Some(NpcMoveView {
                screen: od_core::lost_void_scene::LostVoidScreen::Modal,
                other_exclamation: None,
            }));
        }
        let Some(sight) =
            self.lost_void_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?
        else {
            return Ok(None);
        };
        let mut view = NpcMoveView {
            screen: sight.observation.screen,
            other_exclamation: None,
        };
        if need_icons && view.screen == od_core::lost_void_scene::LostVoidScreen::World {
            ensure!(
                self.detector_bundle.is_some(),
                "NPC 返回移动须先载入固定迷失之地检测模型"
            );
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
                let result=self.recognize_prepared_cancellable(frame,"LegacyYoloDetect",&json!({"model":crate::yolo::LOST_VOID_MODEL,"label_list":["0000-感叹号"],"threshold":0.6,"iou":0.5}),&control);
                finished.store(true, Ordering::Release);
                result
            });
            if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
                return Ok(None);
            }
            let result = result?;
            let detections = result.detail["detail"]["filtered"]
                .as_array()
                .context("NPC 图标推理缺少完整结果")?;
            for d in detections {
                ensure!(d["label"] == "0000-感叹号", "NPC 图标模型返回了未请求类别");
            }
            view.other_exclamation = Some(!detections.is_empty());
        }
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        Ok(Some(view))
    }
    pub fn read_lost_void_npc(
        &self,
        flow: &NpcMoveFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<NpcMoveView>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let view = self.inspect_lost_void_npc(
            &frame.image,
            flow.needs_icons(),
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
    pub fn act_lost_void_npc(
        &mut self,
        flow: &NpcMoveFlow,
        action: NpcMoveAction,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<NpcInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<NpcInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(NpcInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(NpcInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(NpcInputEnd::NotPosted);
            };
            let view = self.inspect_lost_void_npc(
                &frame.image,
                flow.needs_icons(),
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(NpcInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(NpcInputEnd::TimedOut);
            }
            let Some(view) = view else {
                return Ok(NpcInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &view) {
                return Ok(NpcInputEnd::Changed);
            }
            let key = InputProbeAction::Key { key: action.key };
            if !self.combat_input(key, true, cancel)? {
                return Ok(NpcInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(u64::from(action.ms))
                && started.elapsed() < timeout
                && !cancel.load(Ordering::Acquire)
            {
                std::thread::sleep(Duration::from_millis(2));
            }
            let elapsed = held.elapsed().as_millis().min(u128::from(action.ms)) as u32;
            self.combat_input(key, false, &AtomicBool::new(false))?;
            Ok(NpcInputEnd::Held(elapsed))
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}
