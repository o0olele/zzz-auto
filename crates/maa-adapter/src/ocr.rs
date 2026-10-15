use anyhow::{Context, Result, ensure};
use od_core::ocr::{TextMatchMode, matches_text};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Instant;

use crate::{NativeSession, RecognitionImage, RecognitionMeasurement};

fn default_roi() -> [i32; 4] {
    [0, 0, 1920, 1080]
}
fn default_lcs() -> f64 {
    0.5
}
fn default_ignore_case() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OcrTextConfig {
    pub model: String,
    #[serde(default = "default_roi")]
    pub roi: [i32; 4],
    #[serde(default)]
    pub only_rec: bool,
    #[serde(default)]
    pub threshold: f64,
    #[serde(default)]
    pub words: Vec<String>,
    #[serde(default)]
    pub match_mode: TextMatchMode,
    #[serde(default = "default_lcs")]
    pub lcs_percent: f64,
    #[serde(default = "default_ignore_case")]
    pub ignore_case: bool,
}

impl OcrTextConfig {
    pub fn parse(params: &Value) -> Result<Self> {
        let config: Self = serde_json::from_value(params.clone())?;
        ensure!(
            config.model == "ppocrv5",
            "OCR 文字适配须使用固定 ppocrv5 模型"
        );
        let [x, y, w, h] = config.roi;
        ensure!(
            x >= 0
                && y >= 0
                && w > 0
                && h > 0
                && x.checked_add(w).is_some_and(|r| r <= 1920)
                && y.checked_add(h).is_some_and(|b| b <= 1080),
            "OCR ROI 超出 1920×1080"
        );
        ensure!(
            config.threshold.is_finite()
                && (0.0..=1.0).contains(&config.threshold)
                && config.lcs_percent.is_finite()
                && (0.0..=1.0).contains(&config.lcs_percent),
            "OCR 置信度和 LCS 比例须为 0..1"
        );
        ensure!(
            config.words.len() <= 64
                && config
                    .words
                    .iter()
                    .all(|word| { !word.is_empty() && word.chars().count() <= 512 }),
            "OCR 目标文字不能为空，每项最多 512 字，最多 64 项"
        );
        Ok(config)
    }

    pub fn effective_threshold(&self) -> f64 {
        // 旧 TextSystem 在检测 + 识别路径先应用 drop_score=0.5；单行直识别不经过它。
        self.threshold.max(if self.only_rec { 0.0 } else { 0.5 })
    }
}

/// 从 all 读取未经 Maa trim / 正则处理的文字；原始详情保留用于差异验收。
pub fn text_decision(native: &Value, params: &Value) -> Result<Value> {
    let config = OcrTextConfig::parse(params)?;
    let all = native["detail"]["all"]
        .as_array()
        .context("Maa OCR 缺少 all")?;
    ensure!(
        native["hit"].is_boolean() && all.len() <= 512,
        "Maa OCR 详情无效或结果过多"
    );
    ensure!(
        !config.only_rec || all.len() == 1,
        "单行 OCR 未返回唯一原始结果"
    );
    let mut filtered = Vec::new();
    let threshold = config.effective_threshold();
    for item in all {
        let text = item["text"].as_str().context("OCR 缺少文字")?;
        let score = item["score"].as_f64().context("OCR 缺少分数")?;
        let [x, y, w, h]: [i32; 4] = serde_json::from_value(item["box"].clone())?;
        ensure!(
            score.is_finite()
                && (0.0..=1.0).contains(&score)
                && text.chars().count() <= 512
                && x >= 0
                && y >= 0
                && w > 0
                && h > 0
                && x.checked_add(w).is_some_and(|r| r <= 1920)
                && y.checked_add(h).is_some_and(|b| b <= 1080),
            "OCR 文字、分数或全图坐标无效"
        );
        // SDK JSON 只有六位小数，不能从舍入分数证明相等阈值已通过。
        ensure!(
            threshold == 0.0 || (score - threshold).abs() > 0.000001,
            "OCR 分数接近阈值，SDK JSON 精度不足以判定"
        );
        if score < threshold || text.is_empty() {
            continue;
        }
        if !config.words.is_empty()
            && !config.words.iter().any(|word| {
                matches_text(
                    word,
                    text,
                    config.match_mode,
                    config.ignore_case,
                    config.lcs_percent,
                )
            })
        {
            continue;
        }
        let mut accepted = item.clone();
        accepted["center"] = json!([x + w / 2, y + h / 2]);
        filtered.push(accepted);
    }
    Ok(json!({
        "hit": !filtered.is_empty(),
        "detail": {"all": all, "best": filtered.first(), "filtered": filtered},
        "adaptation": {
            "kind": "OcrText", "config": config, "effective_threshold": threshold,
            "coordinate_space": "full_frame", "score_source": "maa_json_6_decimals",
            "legacy_algorithm_compatible": false
        },
        "native_detail": native
    }))
}

impl NativeSession {
    pub(crate) fn recognize_ocr_text(
        &self,
        image: &RecognitionImage,
        params: &Value,
    ) -> Result<RecognitionMeasurement> {
        let started = Instant::now();
        let config = OcrTextConfig::parse(params)?;
        let mut measured = self.recognize_maa_prepared(
            image,
            "OCR",
            &json!({
                "model": config.model, "roi": config.roi, "only_rec": config.only_rec,
                "threshold": 0.0, "order_by": "Vertical"
            }),
        )?;
        let details_at = Instant::now();
        measured.detail = text_decision(&measured.detail, params)?;
        measured.timing.details_us += details_at.elapsed().as_micros() as u64;
        measured.timing.total_us = started.elapsed().as_micros() as u64;
        Ok(measured)
    }
}
