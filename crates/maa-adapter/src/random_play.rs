use crate::{NativeSession, navigation::TurnReport};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub enum ShopRead {
    Themes(Vec<String>),
    Retry,
    Cancelled,
}
pub use ShopRead as RandomPlayRead;
impl NativeSession {
    pub fn prepare_random_play_step(&self, game: bool) -> Result<()> {
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "录像店控制器模式不匹配"
        );
        crate::validate_basic_daily_app_bundle(
            self.daily_bundle.as_deref().unwrap_or(&self.resource_root),
            "random_play",
        )?;
        Ok(())
    }
    pub fn random_play_turn(
        &mut self,
        turn_dx: f64,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<TurnReport> {
        self.navigation_turn_to(
            "random-play/map.png",
            0.0,
            od_core::navigation::TurnCalibration {
                pixels_per_degree: turn_dx,
            },
            timeout.min(Duration::from_secs(30)),
            cancel,
        )
    }
    pub fn random_play_themes(&self, cancel: &AtomicBool) -> Result<ShopRead> {
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(ShopRead::Cancelled);
        };
        let title = self.recognize_prepared(
            &frame.image,
            "OCR",
            &json!({
                "model":"ppocrv5","roi":[1272,14,280,70],"expected":["^经营状况$"],"threshold":0.6
            }),
        )?;
        if title.detail["hit"] != true {
            return Ok(ShopRead::Retry);
        }
        let mut readings = Vec::new();
        for roi in [
            [784, 136, 128, 88],
            [914, 136, 116, 88],
            [1030, 136, 126, 88],
        ] {
            if cancel.load(Ordering::Acquire) {
                return Ok(ShopRead::Cancelled);
            }
            let result = self.recognize_prepared(
                &frame.image,
                "OCR",
                &json!({
                    "model":"ppocrv5","roi":roi,"only_rec":true,"threshold":0.0
                }),
            )?;
            let all = result.detail["detail"]["all"]
                .as_array()
                .context("主题 OCR 缺少结果列表")?;
            readings.push(
                all.iter()
                    .filter_map(|r| r["text"].as_str())
                    .collect::<String>(),
            );
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(ShopRead::Cancelled);
        }
        Ok(ShopRead::Themes(od_core::random_play::resolve_themes(
            &readings,
        )))
    }
}
