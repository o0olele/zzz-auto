use crate::{
    NativeSession, RawRecognitionControl,
    combat_vision::{CombatFrame, CombatRecognizer},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    InputMode, InputProbeAction, MouseButton,
    challenge_move::{self, ChallengeMove, MoveAction, MoveObservation, MoveStage},
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveEnd {
    Completed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Copy)]
pub struct MoveOptions {
    pub game: bool,
    pub timeout: Duration,
    pub turn_sensitivity_milli: u32,
    pub max_frame_age_ms: u32,
}
impl Default for MoveOptions {
    fn default() -> Self {
        Self {
            game: false,
            timeout: Duration::from_secs(120),
            turn_sensitivity_milli: 1000,
            max_frame_age_ms: 1000,
        }
    }
}

impl NativeSession {
    pub fn inspect_challenge_move(
        &self,
        recognizer: &CombatRecognizer,
        frame: &CombatFrame,
        progress: &ChallengeMove,
        cancel: &AtomicBool,
        timeout: Duration,
        max_frame_age_ms: u32,
    ) -> Result<Option<MoveObservation>> {
        progress.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            (100..=2000).contains(&max_frame_age_ms),
            "战前画面有效期须为 100..2000ms"
        );
        let started = Instant::now();
        let ready = || -> Result<bool> {
            if cancel.load(Ordering::Acquire) {
                return Ok(false);
            }
            ensure!(started.elapsed() < timeout, "战前画面识别超时");
            Ok(true)
        };
        if !ready()? {
            return Ok(None);
        }
        if frame.age_ms() > max_frame_age_ms {
            return Ok(Some(MoveObservation::default()));
        }
        let attack = recognizer
            .matched(
                self,
                frame,
                "battle/btn_normal_attack",
                recognizer.area("按键-普通攻击")?,
                0.7,
            )?
            .is_some();
        let mut o = MoveObservation {
            attack,
            ..Default::default()
        };
        if !ready()? {
            return Ok(None);
        }
        o.interact = recognizer
            .matched(
                self,
                frame,
                "battle/btn_interact",
                recognizer.area("按键-交互")?,
                0.7,
            )?
            .is_some();
        if !ready()? {
            return Ok(None);
        }
        o.boss = recognizer
            .matched(self, frame, "battle/boss_life", [1620, 16, 1720, 106], 0.7)?
            .is_some();
        if !ready()? {
            return Ok(None);
        }
        if !progress.expert && !o.attack && !o.interact && !o.boss && progress.interaction_attempted
        {
            let result = self.recognize_prepared(
                &frame.image,
                "OCR",
                &json!({"model":"ppocrv5","threshold":0.6,"expected":[".*选择.*"]}),
            )?;
            if !ready()? {
                return Ok(None);
            }
            for item in result.detail["detail"]["filtered"]
                .as_array()
                .context("鸣徽文字识别缺少结果")?
            {
                let rect: [i32; 4] =
                    serde_json::from_value(item["box"].clone()).context("鸣徽按钮坐标无效")?;
                ensure!(od_core::intel_board::valid_rect(rect), "鸣徽按钮超出画面");
                o.choices.push(rect);
            }
        }
        if !progress.expert
            && !o.interact
            && !o.boss
            && o.choices.is_empty()
            && (o.attack || progress.stage == MoveStage::Advance)
        {
            ensure!(
                self.detector_bundle.is_some(),
                "战前移动须先预检并载入固定距离模型"
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
                let result=self.recognize_prepared_cancellable(&frame.image,"LegacyYoloDetect",
                    &json!({"model":crate::yolo::LOST_VOID_MODEL,"label_list":["0001-距离"],"threshold":0.6}),&control);
                finished.store(true, Ordering::Release);
                result
            });
            if !ready()? {
                return Ok(None);
            }
            let result = result?;
            let detections = result.detail["detail"]["filtered"]
                .as_array()
                .context("距离模型缺少结果")?;
            let mut candidates = Vec::new();
            for item in detections {
                ensure!(item["label"] == "0001-距离", "距离模型返回了未请求的类别");
                let [x, _, w, _]: [i32; 4] =
                    serde_json::from_value(item["box"].clone()).context("距离提示坐标无效")?;
                let center = f64::from(x) + f64::from(w) / 2.0;
                if challenge_move::turn_distance(center).is_some() {
                    candidates.push((item["score"].as_f64().context("距离分数无效")?, center));
                }
            }
            candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
            if let Some(&(score, x)) = candidates.first() {
                ensure!(
                    !candidates
                        .iter()
                        .skip(1)
                        .any(|&(s, other)| (s - score).abs() < 1e-9 && (other - x).abs() > 10.0),
                    "距离提示同分且方向有歧义"
                );
                o.target_x = Some(x);
            }
            let result = self.recognize_prepared(
                &frame.image,
                "OCR",
                &json!({"model":"ppocrv5","roi":[432,132,1208,710],"threshold":0.6}),
            )?;
            if !ready()? {
                return Ok(None);
            }
            let mut distances = Vec::new();
            for item in result.detail["detail"]["filtered"]
                .as_array()
                .context("距离 OCR 缺少结果")?
            {
                if let Some(distance) = item["text"]
                    .as_str()
                    .and_then(challenge_move::parse_distance)
                {
                    let [x, _, w, _]: [i32; 4] =
                        serde_json::from_value(item["box"].clone()).context("距离文字坐标无效")?;
                    let score = progress
                        .last_distance
                        .map_or((f64::from(x + w / 2) - 960.0).abs(), |last| {
                            (last - distance).abs()
                        });
                    distances.push((score, distance));
                }
            }
            distances.sort_by(|a, b| a.0.total_cmp(&b.0));
            if let Some(&(score, distance)) = distances.first() {
                ensure!(
                    !distances
                        .iter()
                        .skip(1)
                        .any(|&(s, d)| (s - score).abs() < 1e-9 && (d - distance).abs() > 0.1),
                    "距离文字候选有歧义"
                );
                o.distance = Some(distance);
            }
        }
        if !ready()? {
            return Ok(None);
        }
        o.fresh = frame.age_ms() <= max_frame_age_ms;
        Ok(Some(o))
    }

    pub fn execute_challenge_move(
        &mut self,
        recognizer: &CombatRecognizer,
        progress: &mut ChallengeMove,
        options: MoveOptions,
        cancel: Arc<AtomicBool>,
        mut publish: impl FnMut(ChallengeMove) -> Result<()>,
    ) -> Result<MoveEnd> {
        let MoveOptions {
            game,
            timeout,
            turn_sensitivity_milli,
            max_frame_age_ms,
        } = options;
        ensure!(
            self.game_window.is_some() == game && self.input_mode == Some(InputMode::Foreground),
            "战前界面操作须使用前台控制器"
        );
        ensure!(
            (1..=100000).contains(&turn_sensitivity_milli),
            "战前转向灵敏度无效"
        );
        ensure!(
            (100..=2000).contains(&max_frame_age_ms) && timeout <= Duration::from_secs(600),
            "战前超时或画面有效期无效"
        );
        progress.validate().map_err(anyhow::Error::msg)?;
        if progress.pending.is_some() {
            progress.resume().map_err(anyhow::Error::msg)?;
        }
        let started = Instant::now();
        let result = (|| -> Result<MoveEnd> {
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Ok(MoveEnd::Cancelled);
                }
                if started.elapsed() >= timeout {
                    return Ok(MoveEnd::TimedOut);
                }
                if progress.stage == MoveStage::Done {
                    publish(progress.clone())?;
                    return Ok(if cancel.load(Ordering::Acquire) {
                        MoveEnd::Cancelled
                    } else {
                        MoveEnd::Completed
                    });
                }
                let Some(frame) = self.combat_frame(&cancel)? else {
                    return Ok(MoveEnd::Cancelled);
                };
                let Some(observation) = self.inspect_challenge_move(
                    recognizer,
                    &frame,
                    progress,
                    &cancel,
                    timeout.saturating_sub(started.elapsed()),
                    max_frame_age_ms,
                )?
                else {
                    return Ok(MoveEnd::Cancelled);
                };
                let action = progress.next(&observation).map_err(anyhow::Error::msg)?;
                if let Some(action) = action {
                    publish(progress.clone())?;
                    if cancel.load(Ordering::Acquire) {
                        return Ok(MoveEnd::Cancelled);
                    }
                    if started.elapsed() >= timeout {
                        return Ok(MoveEnd::TimedOut);
                    }
                    let mut settle = 500;
                    match &action {
                        MoveAction::Forward { ms, .. } => {
                            let (complete, held) =
                                self.challenge_hold(87, *ms, &cancel, started, timeout)?;
                            if progress.expert {
                                progress
                                    .record_expert_hold(held)
                                    .map_err(anyhow::Error::msg)?;
                            } else if complete {
                                progress.acknowledge().map_err(anyhow::Error::msg)?;
                            }
                            if !complete {
                                publish(progress.clone())?;
                                return Ok(if cancel.load(Ordering::Acquire) {
                                    MoveEnd::Cancelled
                                } else {
                                    MoveEnd::TimedOut
                                });
                            }
                        }
                        MoveAction::Search { key } => {
                            let (first, _) =
                                self.challenge_hold(87, 1000, &cancel, started, timeout)?;
                            let complete = first
                                && self.challenge_hold(*key, 500, &cancel, started, timeout)?.0;
                            if !complete {
                                return Ok(if cancel.load(Ordering::Acquire) {
                                    MoveEnd::Cancelled
                                } else {
                                    MoveEnd::TimedOut
                                });
                            }
                            progress.acknowledge().map_err(anyhow::Error::msg)?;
                        }
                        MoveAction::Turn { dx } => {
                            let scaled =
                                (i64::from(*dx) * i64::from(turn_sensitivity_milli) / 1000) as i32;
                            if !self.combat_turn(
                                if scaled == 0 { dx.signum() } else { scaled },
                                &cancel,
                            )? {
                                return Ok(MoveEnd::Cancelled);
                            }
                            progress.acknowledge().map_err(anyhow::Error::msg)?;
                        }
                        MoveAction::Interact => {
                            let (complete, _) =
                                self.challenge_hold(70, 200, &cancel, started, timeout)?;
                            if !complete {
                                return Ok(if cancel.load(Ordering::Acquire) {
                                    MoveEnd::Cancelled
                                } else {
                                    MoveEnd::TimedOut
                                });
                            }
                            progress.acknowledge().map_err(anyhow::Error::msg)?;
                            settle = 2000;
                        }
                        MoveAction::Buff { rect: [x, y, w, h] } => {
                            let action = InputProbeAction::MouseButton {
                                button: MouseButton::Left,
                                x: x + w / 2,
                                y: y + h / 2,
                            };
                            if !self.combat_input(action, true, &cancel)? {
                                return Ok(MoveEnd::Cancelled);
                            }
                            self.combat_input(action, false, &AtomicBool::new(false))?;
                            progress.acknowledge().map_err(anyhow::Error::msg)?;
                            settle = 1000;
                        }
                    }
                    publish(progress.clone())?;
                    let idle = Instant::now();
                    while idle.elapsed() < Duration::from_millis(settle)
                        && started.elapsed() < timeout
                        && !cancel.load(Ordering::Acquire)
                    {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                } else if progress.stage != MoveStage::Done {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
    fn challenge_hold(
        &mut self,
        key: i32,
        ms: u32,
        cancel: &AtomicBool,
        started: Instant,
        timeout: Duration,
    ) -> Result<(bool, u32)> {
        let action = InputProbeAction::Key { key };
        if !self.combat_input(action, true, cancel)? {
            return Ok((false, 0));
        }
        let held = Instant::now();
        while held.elapsed() < Duration::from_millis(u64::from(ms))
            && started.elapsed() < timeout
            && !cancel.load(Ordering::Acquire)
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        let elapsed = held.elapsed().as_millis().min(u128::from(ms)) as u32;
        self.combat_input(action, false, &AtomicBool::new(false))?;
        Ok((elapsed >= ms, elapsed))
    }
}
