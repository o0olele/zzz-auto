use crate::{
    NativeSession, RecognitionImage,
    navigation_vision::{
        self, Heading, MAP_BUTTON_DELTA, MAP_BUTTON_RECT, MINI_MAP_SIZE, STATIC_MAP_RECT,
    },
};
use anyhow::{Context, Result, ensure};
use od_core::navigation::{AbsoluteTurn, TurnCalibration, TurnDecision};
use serde_json::json;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    Aligned,
    Cancelled,
    TimedOut,
}

#[derive(Debug)]
pub struct TurnReport {
    pub outcome: TurnOutcome,
    pub commands: u32,
    pub last_heading: Option<f64>,
}

pub struct NavigationFrame {
    pub(crate) image: RecognitionImage,
    captured: Instant,
}

impl NavigationFrame {
    pub fn age(&self) -> Duration {
        self.captured.elapsed()
    }
}

impl NativeSession {
    pub fn navigation_frame(&self, cancel: &AtomicBool) -> Result<Option<NavigationFrame>> {
        ensure!(!self.tasker.running(), "导航截图前须等待 Pipeline 收尾");
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let captured = Instant::now();
        let status = self.wait_input_job(self.controller.post_screencap()?, cancel, &mut None)?;
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        ensure!(status.is_success(), "导航截图失败");
        let buffer = self.controller.cached_image()?;
        ensure!(
            buffer.width() == 1920
                && buffer.height() == 1080
                && buffer.channels() == 3
                && buffer.image_type() == 16,
            "导航截图须为连续 1080p BGR8"
        );
        Ok(Some(NavigationFrame {
            image: RecognitionImage {
                buffer,
                prepare_us: captured.elapsed().as_micros() as u64,
            },
            captured,
        }))
    }

    /// 模板由受校验的业务资源包提供；定位及角度识别共用同一张截图。
    pub fn navigation_heading(
        &self,
        frame: &NavigationFrame,
        map_template: &str,
        cancel: &AtomicBool,
    ) -> Result<Option<Heading>> {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let result = self.recognize_prepared(
            &frame.image,
            "TemplateMatch",
            &json!({
                "roi": MAP_BUTTON_RECT, "template": map_template, "threshold": 0.5,
                "method": 5, "green_mask": true, "order_by": "Score"
            }),
        )?;
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let bgr = frame.image.buffer.raw_data().context("导航截图没有像素")?;
        if result.detail["hit"] == true {
            let [x, y, _, _]: [i32; 4] =
                serde_json::from_value(result.detail["detail"]["best"]["box"].clone())
                    .context("地图模板没有有效的匹配框")?;
            let rect = [
                x - MAP_BUTTON_DELTA[0],
                y - MAP_BUTTON_DELTA[1],
                MINI_MAP_SIZE,
                MINI_MAP_SIZE,
            ];
            if let Some(heading) = navigation_vision::crop_heading(bgr, rect)? {
                return Ok(Some(heading));
            }
        }
        // 与参考版相同的静态兜底；玩家必须在框中心，偏移画面不能据此转向。
        navigation_vision::crop_heading(bgr, STATIC_MAP_RECT)
    }

    /// 调用方先确认大世界画面并持有运行槽。每次调用重新读取朝向；
    /// 暂停/停止的同一取消信号阻止后续截图和鼠标输入。
    pub fn navigation_turn_to(
        &mut self,
        map_template: &str,
        target: f64,
        calibration: TurnCalibration,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<TurnReport> {
        ensure!(
            !timeout.is_zero() && timeout <= Duration::from_secs(30),
            "转向超时须为 1ns..30s"
        );
        let mut turn = AbsoluteTurn::new(target, calibration).map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let mut last_heading = None;
        let mut dispatched = 0;
        let outcome = loop {
            if cancel.load(Ordering::Acquire) {
                break TurnOutcome::Cancelled;
            }
            if started.elapsed() >= timeout {
                break TurnOutcome::TimedOut;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                break TurnOutcome::Cancelled;
            };
            let heading = self.navigation_heading(&frame, map_template, cancel)?;
            if cancel.load(Ordering::Acquire) {
                break TurnOutcome::Cancelled;
            }
            if started.elapsed() >= timeout {
                break TurnOutcome::TimedOut;
            }
            // 耗时过长的截图/识别不允许带动输入，也不能用于连续确认。
            let heading = heading.filter(|_| frame.age() <= Duration::from_millis(500));
            if let Some(heading) = heading {
                last_heading = Some(heading.degrees);
                match turn.observe(heading.degrees).map_err(anyhow::Error::msg)? {
                    TurnDecision::Aligned => break TurnOutcome::Aligned,
                    TurnDecision::Observe => {}
                    TurnDecision::Move { dx, .. } => {
                        if !self.combat_turn(dx, cancel)? {
                            break TurnOutcome::Cancelled;
                        }
                        dispatched += 1;
                    }
                }
            } else {
                turn.invalidate_observation();
            }
            // 给镜头动画时间；每 10ms 检查取消和整段超时。
            let settle = Instant::now();
            while settle.elapsed() < Duration::from_millis(500)
                && started.elapsed() < timeout
                && !cancel.load(Ordering::Acquire)
            {
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        Ok(TurnReport {
            outcome,
            commands: dispatched,
            last_heading,
        })
    }
}
