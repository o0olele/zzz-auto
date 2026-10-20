use crate::{NativeSession, RecognitionImage};
use anyhow::{Context, Result, ensure};
use od_core::{
    intel_board::{self, CommissionCandidate, CommissionKind, IntelLedger},
    intel_board_flow::IntelFlow,
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntelBoardRead {
    Unknown,
    UnreadableProgress,
    Board {
        contribution: u16,
        commission: Option<CommissionCandidate>,
    },
    Cancelled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntelPickEnd {
    Opened,
    Changed,
    Retry,
    Cancelled,
    TimedOut,
}

impl NativeSession {
    /// 在本次委托检查点保存后调用；重新读板，绝不点击跨步骤保存的位置。
    pub fn choose_intel_commission(
        &mut self,
        flow: &IntelFlow,
        ledger: &IntelLedger,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
        game: bool,
    ) -> Result<IntelPickEnd> {
        flow.validate(ledger).map_err(anyhow::Error::msg)?;
        ensure!(
            flow.stage == "OpenCommission"
                && ledger
                    .pending
                    .as_ref()
                    .is_some_and(|p| !p.deployed && p.result.is_none()),
            "没有待打开的本次委托"
        );
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "情报板选帖控制器模式不符"
        );
        crate::validate_basic_daily_app_bundle(
            self.daily_bundle.as_deref().unwrap_or(&self.resource_root),
            "intel_board",
        )?;
        let started = Instant::now();
        let result = (|| -> Result<IntelPickEnd> {
            if cancel.load(Ordering::Acquire) {
                return Ok(IntelPickEnd::Cancelled);
            }
            if started.elapsed() >= timeout {
                return Ok(IntelPickEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    &cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(&cancel)? else {
                return Ok(IntelPickEnd::Cancelled);
            };
            let read = self.inspect_intel_board(
                &frame.image,
                "intel-board/star.png",
                &cancel,
                timeout.saturating_sub(started.elapsed()),
            );
            if cancel.load(Ordering::Acquire) {
                return Ok(IntelPickEnd::Cancelled);
            }
            if started.elapsed() >= timeout {
                return Ok(IntelPickEnd::TimedOut);
            }
            let candidate = match read? {
                IntelBoardRead::Board {
                    contribution,
                    commission,
                } => {
                    if ledger.observed_contribution != Some(contribution) {
                        return Ok(IntelPickEnd::Changed);
                    }
                    commission
                }
                IntelBoardRead::Cancelled => return Ok(IntelPickEnd::Cancelled),
                IntelBoardRead::UnreadableProgress => return Ok(IntelPickEnd::Retry),
                IntelBoardRead::Unknown => {
                    // 点击后暂停可能已经打开详情；只读检查后态，不能再次点击帖子。
                    let report = self.intel_board_step(
                        flow,
                        ledger,
                        cancel.clone(),
                        timeout
                            .saturating_sub(started.elapsed())
                            .min(Duration::from_millis(1000)),
                        game,
                    )?;
                    ensure!(report.report.released, "委托后态检查未释放输入");
                    if cancel.load(Ordering::Acquire) {
                        return Ok(IntelPickEnd::Cancelled);
                    }
                    if started.elapsed() >= timeout {
                        return Ok(IntelPickEnd::TimedOut);
                    }
                    return Ok(match report.report.end {
                        crate::PipelineEnd::Succeeded => IntelPickEnd::Opened,
                        crate::PipelineEnd::Cancelled => IntelPickEnd::Cancelled,
                        _ => IntelPickEnd::Retry,
                    });
                }
            };
            let Some(candidate) = candidate else {
                return Ok(IntelPickEnd::Changed);
            };
            if candidate.kind != ledger.pending.as_ref().unwrap().kind {
                return Ok(IntelPickEnd::Changed);
            }
            if frame.age() > Duration::from_millis(1000) {
                return Ok(IntelPickEnd::Retry);
            }
            if cancel.load(Ordering::Acquire) {
                return Ok(IntelPickEnd::Cancelled);
            }
            let [x, y, w, h] = candidate.rect;
            let action = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x: x + w / 2,
                y: y + h / 2,
            };
            if !self.combat_input(action, true, &cancel)? {
                return Ok(IntelPickEnd::Cancelled);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(100) && !cancel.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(action, false, &AtomicBool::new(false))?;
            Ok(if cancel.load(Ordering::Acquire) {
                IntelPickEnd::Cancelled
            } else if started.elapsed() >= timeout {
                IntelPickEnd::TimedOut
            } else {
                IntelPickEnd::Opened
            })
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
    pub fn intel_board_step(
        &mut self,
        flow: &IntelFlow,
        ledger: &IntelLedger,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
        game: bool,
    ) -> Result<crate::PipelineExecution> {
        flow.validate(ledger).map_err(anyhow::Error::msg)?;
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "情报板界面控制器模式不匹配"
        );
        crate::validate_basic_daily_app_bundle(
            self.daily_bundle.as_deref().unwrap_or(&self.resource_root),
            "intel_board",
        )?;
        ensure!(
            intel_board::pipeline().get(flow.entry()).is_some(),
            "此情报板步骤须由原生选队、移动、战斗或读板执行器处理"
        );
        if flow.stage == "Deploy" {
            ensure!(
                ledger.pending.as_ref().is_some_and(|p| p.deployed),
                "出战输入前须保存投递检查点"
            );
        }
        if flow.stage == "ConfirmTransport" {
            ensure!(flow.transport_attempted, "传送输入前须保存检查点");
        }
        if game {
            self.prepare_input_target(
                od_core::mail::MailConfig::recovery_inputs()[1],
                &cancel,
                &mut None,
            )?;
        }
        let overrides = if let Some(pending) = &ledger.pending {
            json!({"Intel.Kind":{"expected":[format!("^{}$",pending.kind.label())]}})
        } else {
            json!({})
        };
        self.run_pipeline_detailed(
            &flow.entry(),
            &crate::PipelineInputPlan {
                keys: [18, 27, 78].into(),
                contacts: [0].into(),
            },
            cancel,
            timeout,
            &overrides.to_string(),
            true,
        )
    }

    /// 调用方须提供已经加载并校验的星标模板。OCR、进度与本人帖子标记均
    /// 来自传入的同一帧；返回值只是只读候选，实际输入前须重新识别。
    pub fn inspect_intel_board(
        &self,
        frame: &RecognitionImage,
        star_template: &str,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<IntelBoardRead> {
        let started = Instant::now();
        let ready = || -> Result<bool> {
            if cancel.load(Ordering::Acquire) {
                return Ok(false);
            }
            ensure!(started.elapsed() < timeout, "情报板只读识别超时");
            Ok(true)
        };
        ensure!(!self.tasker.running(), "情报板识别前须等待 Pipeline 收尾");
        if !ready()? {
            return Ok(IntelBoardRead::Cancelled);
        }
        let result =
            self.recognize_prepared(frame, "OCR", &json!({"model":"ppocrv5","threshold":0.6}))?;
        if !ready()? {
            return Ok(IntelBoardRead::Cancelled);
        }
        let texts = result.detail["detail"]["filtered"]
            .as_array()
            .context("情报板 OCR 缺少结果")?;
        // 固定 OCR 可能把同一个“兑换”按钮读成“兌換”；只合并这两个等义字形。
        let has_text = |expected: &str| {
            texts.iter().any(|text| {
                text["text"].as_str().is_some_and(|text| {
                    text.replace('兌', "兑")
                        .replace('換', "换")
                        .contains(expected)
                })
            })
        };
        let exchange_at_bottom = texts.iter().any(|reading| {
            let Some(text) = reading["text"].as_str() else {
                return false;
            };
            let Ok(rect) = serde_json::from_value::<[i32; 4]>(reading["box"].clone()) else {
                return false;
            };
            let [x, y, width, height] = rect;
            intel_board::valid_rect(rect)
                && (954..1242).contains(&(x + width / 2))
                && (991..1064).contains(&(y + height / 2))
                && text.replace('兌', "兑").replace('換', "换") == "点数兑换"
        });
        if !has_text("周期内可获取") || !exchange_at_bottom {
            return Ok(IntelBoardRead::Unknown);
        }
        let result = self.recognize_prepared(
            frame,
            "OCR",
            &json!({
                "model":"ppocrv5","roi":[322,1019,191,42],"only_rec":true,"threshold":0.6
            }),
        )?;
        if !ready()? {
            return Ok(IntelBoardRead::Cancelled);
        }
        let progress = result.detail["detail"]["filtered"]
            .as_array()
            .context("情报板进度 OCR 缺少结果")?;
        let contribution = match progress.as_slice() {
            [reading] => reading["text"]
                .as_str()
                .and_then(intel_board::parse_progress),
            _ => None,
        };
        let Some(contribution) = contribution else {
            return Ok(IntelBoardRead::UnreadableProgress);
        };
        let mut candidates = Vec::new();
        for reading in texts {
            let kind = match reading["text"].as_str() {
                Some("恶名狩猎") => CommissionKind::NotoriousHunt,
                Some("专业挑战室") => CommissionKind::ExpertChallenge,
                _ => continue,
            };
            candidates.push(CommissionCandidate {
                kind,
                rect: serde_json::from_value(reading["box"].clone())
                    .context("情报板委托缺少有效坐标")?,
            });
        }
        // 规则只看星标与候选在 x 轴的重叠。固定模板只需扫描这些列的完整高度，
        // 两侧保留完整模板及 30px 扩展余量；与全帧过滤等价，避免每次耗时超过帧龄。
        ensure!(
            candidates.len() <= 128 && candidates.iter().all(|c| intel_board::valid_rect(c.rect)),
            "情报板候选坐标无效"
        );
        let rois = if star_template == "intel-board/star.png" {
            let width = image::load_from_memory(intel_board::source_images()[0].1)?.width() as i32;
            let mut columns: Vec<_> = candidates
                .iter()
                .map(|c| {
                    (
                        (c.rect[0] - 2 * width - 30).max(0),
                        (c.rect[0] + c.rect[2] + 2 * width).min(1920),
                    )
                })
                .collect();
            columns.sort_unstable();
            let mut merged: Vec<(i32, i32)> = vec![];
            for (left, right) in columns {
                if let Some(last) = merged.last_mut().filter(|last| left <= last.1) {
                    last.1 = last.1.max(right);
                } else {
                    merged.push((left, right));
                }
            }
            merged
                .into_iter()
                .map(|(left, right)| [left, 0, right - left, 1080])
                .collect::<Vec<_>>()
        } else {
            vec![[0, 0, 1920, 1080]]
        };
        let mut stars = Vec::new();
        for roi in rois {
            let result = self.recognize_prepared(
                frame,
                "TemplateMatch",
                &json!({"template":star_template,"threshold":0.8,"green_mask":true,"roi":roi}),
            )?;
            if !ready()? {
                return Ok(IntelBoardRead::Cancelled);
            }
            for item in result.detail["detail"]["filtered"]
                .as_array()
                .context("情报板星标识别缺少结果")?
            {
                stars.push(
                    serde_json::from_value(item["box"].clone())
                        .context("情报板星标缺少有效坐标")?,
                );
            }
        }
        let commission =
            intel_board::choose_commission(&candidates, &stars).map_err(anyhow::Error::msg)?;
        if !ready()? {
            return Ok(IntelBoardRead::Cancelled);
        }
        Ok(IntelBoardRead::Board {
            contribution,
            commission,
        })
    }
}
