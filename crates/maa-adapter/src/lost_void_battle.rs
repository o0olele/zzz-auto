//! 同一战斗截图的页面守卫。深读必须在前一轮释放输入之后执行。
use crate::{
    NativeSession, RawRecognitionControl,
    combat_vision::{CombatFrame, CombatRecognizer},
    lost_void_battle_hud::LostVoidBattleHudReader,
    suibian_adventure::{Reader, words},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    lost_void_battle::{BattlePage, BattleView, ExitProbe},
    lost_void_navigation::NavDetection,
    lost_void_scene::LostVoidScreen as S,
};
use serde_json::json;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub enum BattleRead {
    /// 本帧只要求释放，不能把等待当作没有目标的探测结果。
    Release,
    Unavailable,
    View(BattleView),
}
pub struct LostVoidBattleWatcher {
    hud: LostVoidBattleHudReader,
    max_age_ms: u32,
    boss: bool,
    probe_sequence: u64,
    last_probe: Option<Instant>,
    positive_probe: bool,
}
impl LostVoidBattleWatcher {
    pub fn new(
        recognizer: &CombatRecognizer,
        ids: &[String],
        region: &str,
        max_age_ms: u32,
        last_sequence: u64,
    ) -> Result<Self> {
        ensure!(
            (1..=1000).contains(&max_age_ms) && od_core::lost_void::REGIONS.contains(&region),
            "战斗守卫区域或帧龄无效"
        );
        Ok(Self {
            hud: LostVoidBattleHudReader::new(recognizer, ids)?,
            max_age_ms,
            boss: matches!(region, "战斗-道中危机" | "战斗-终结之役"),
            probe_sequence: last_sequence,
            last_probe: None,
            positive_probe: false,
        })
    }
    fn due(&self) -> bool {
        self.last_probe.is_none_or(|t| {
            t.elapsed() >= Duration::from_millis(if self.positive_probe { 100 } else { 800 })
        })
    }
}
impl NativeSession {
    pub fn require_lost_void_battle_roots(
        &self,
        scene: &std::path::Path,
        combat: &std::path::Path,
        detector: &std::path::Path,
    ) -> Result<()> {
        self.require_lost_void_preparation_roots(scene, None, combat)?;
        ensure!(
            self.detector_bundle.as_deref() == Some(detector),
            "战斗脱战模型与冻结路径不一致"
        );
        Ok(())
    }

    pub fn inspect_lost_void_battle(
        &self,
        frame: &CombatFrame,
        recognizer: &CombatRecognizer,
        watcher: &mut LostVoidBattleWatcher,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<BattleRead> {
        let started = Instant::now();
        let remaining = || timeout.saturating_sub(started.elapsed());
        if cancel.load(Ordering::Acquire) || timeout.is_zero() || frame.age_ms() > 1500 {
            return Ok(BattleRead::Unavailable);
        }
        ensure!(
            self.combat_bundle.as_deref() == Some(recognizer.bundle.root.as_path()),
            "战斗守卫识别器与当前冻结资源不一致"
        );
        let result = (|| -> Result<BattleRead> {
            let cached =
                self.verify_lost_void_battle_hud(frame, &watcher.hud, cancel, watcher.max_age_ms)?;
            let page_read = cached.is_none();
            let probe_due = watcher.due();
            if page_read && !self.inputs_released() {
                return Ok(BattleRead::Release);
            }
            let reader = Reader {
                session: self,
                frame: &frame.image,
                cancel,
                started,
                timeout,
                pipeline: od_core::lost_void_scene::pipeline(),
            };
            // 确认弹窗可覆盖仍可见的 HUD；这里只读，绝不点击。
            let modal = words(&reader.read("LostVoid.ModalConfirm", None)?)?
                .iter()
                .any(|w| matches!(w.text.as_str(), "确认" | "確認"));
            if modal && !self.inputs_released() {
                return Ok(BattleRead::Release);
            }
            let mut view = BattleView::default();
            if page_read || modal {
                let Some(scene) =
                    self.inspect_lost_void_scene(&frame.image, cancel, remaining())?
                else {
                    return Ok(BattleRead::Unavailable);
                };
                view.page = match scene.screen {
                    S::World => BattlePage::World,
                    S::CommonChoice
                    | S::Event
                    | S::Gear
                    | S::GearConfirm
                    | S::Store
                    | S::Lottery
                    | S::RouteChange
                    | S::BattleResult
                    | S::BattleFail => BattlePage::Content(scene.screen),
                    S::Modal => BattlePage::Modal,
                    _ => BattlePage::Unknown,
                };
                if modal || matches!(view.page, BattlePage::Content(_) | BattlePage::Modal) {
                    if modal {
                        view.page = BattlePage::Modal;
                    }
                    return Ok(BattleRead::View(view));
                }
            }
            let matched = |key: &str, area: &str| -> Result<bool> {
                Ok(recognizer
                    .matched(self, frame, key, recognizer.area(area)?, 0.7)?
                    .is_some())
            };
            view.attack = matched("battle/btn_normal_attack", "按键-普通攻击")?;
            view.interact = matched("battle/btn_interact", "按键-交互")?;
            view.dead = matched("battle/agent_dead", "代理人阵亡")?;
            view.chain = crate::combat_contours::contours(
                frame.image.buffer.raw_data().context("战斗守卫没有像素")?,
                recognizer.area("连携条")?,
                [15.0, 183.0, 237.0],
                [10.0, 10.0, 10.0],
                false,
            )? > 0;
            // 快速边界也先松键，再在下一帧读取完整上下文。
            if (view.interact || view.dead || !(view.attack || view.chain))
                && !self.inputs_released()
            {
                return Ok(BattleRead::Release);
            }
            view.members = if cached.is_some() {
                cached
            } else {
                self.inspect_lost_void_battle_hud(frame, &mut watcher.hud, cancel, remaining())?
            };
            if view.members.is_some()
                && matches!(view.page, BattlePage::Unknown | BattlePage::World)
            {
                view.page = BattlePage::CombatHud;
            }
            if probe_due {
                let hints = words(&reader.read("LostVoid.NavHint", None)?)?;
                view.next_region_hint = hints.iter().any(|w| {
                    matches!(
                        w.text.trim_end_matches(['!', '！', '。']),
                        "前往下一个区域" | "前往下一個區域"
                    )
                });
                if !watcher.boss && matches!(view.page, BattlePage::CombatHud | BattlePage::World) {
                    let control = RawRecognitionControl::default();
                    let finished = AtomicBool::new(false);
                    let detected = std::thread::scope(|scope| {
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
                        let result = self.recognize_prepared_cancellable(&frame.image,
                            "LegacyYoloDetect", &json!({"model":crate::yolo::LOST_VOID_MODEL,"threshold":0.9,"iou":0.5}), &control);
                        finished.store(true, Ordering::Release);
                        result
                    })?;
                    let detections = detected.detail["detail"]["filtered"]
                        .as_array()
                        .context("战斗脱战模型没有完整结果")?
                        .iter()
                        .map(|d| -> Result<NavDetection> {
                            Ok(NavDetection {
                                label: d["label"].as_str().context("脱战模型没有类别")?.into(),
                                rect: serde_json::from_value(d["box"].clone())?,
                                score: d["score"].as_f64().context("脱战模型没有置信度")?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    // 模型阈值允许等于 0.9，但本次交接只采用严格大于 0.9。
                    let detections: Vec<_> =
                        detections.into_iter().filter(|d| d.score > 0.9).collect();
                    let sequence = watcher
                        .probe_sequence
                        .checked_add(1)
                        .context("脱战探测序号溢出")?;
                    let probe = ExitProbe {
                        sequence,
                        detections,
                    };
                    probe.validate().map_err(anyhow::Error::msg)?;
                    if frame.age_ms() <= 1500
                        && !cancel.load(Ordering::Acquire)
                        && !remaining().is_zero()
                    {
                        watcher.probe_sequence = sequence;
                        watcher.positive_probe = !probe.detections.is_empty();
                        view.exit_probe = Some(probe);
                    }
                }
                watcher.last_probe = Some(Instant::now());
            }
            if view.page == BattlePage::CombatHud && frame.age_ms() > watcher.max_age_ms {
                return Ok(BattleRead::Unavailable);
            }
            view.validate().map_err(anyhow::Error::msg)?;
            Ok(BattleRead::View(view))
        })();
        if cancel.load(Ordering::Acquire) || remaining().is_zero() || frame.age_ms() > 1500 {
            return Ok(BattleRead::Unavailable);
        }
        result
    }
}
