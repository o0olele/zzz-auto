use crate::{CaptureMethod, Screenshot, Snapshot, WindowInfo};
use anyhow::{Context, Result, ensure};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};
use ts_rs::TS;
use uuid::Uuid;

pub const MAX_PNG_BYTES: usize = 10 * 1024 * 1024;

pub(crate) fn validate_png(png: &[u8]) -> Result<()> {
    ensure!(
        png.len() <= MAX_PNG_BYTES && png.starts_with(b"\x89PNG\r\n\x1a\n"),
        "素材必须是有界 PNG"
    );
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1920);
    limits.max_image_height = Some(1080);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().context("PNG 解码失败或超出 1080p 限制")?;
    ensure!(
        decoded.width() == 1920 && decoded.height() == 1080,
        "PNG 实际尺寸不是 1920×1080"
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ScreenSampleKind {
    EngagementDaily,
    EngagementConfirm,
    EngagementPreview,
    CoffeeOptions,
    CoffeeEnergy,
    CoffeeBuff,
    Other,
}
impl ScreenSampleKind {
    pub fn options() -> Value {
        json!([
            {"value":"engagement_daily","label":"活跃度 · 日常页"},
            {"value":"engagement_confirm","label":"活跃度 · 领取确认"},
            {"value":"engagement_preview","label":"活跃度 · 奖励预览"},
            {"value":"coffee_options","label":"咖啡 · 点单选项"},
            {"value":"coffee_energy","label":"咖啡 · 电量奖励"},
            {"value":"coffee_buff","label":"咖啡 · 增益确认"},
            {"value":"other","label":"其他画面"}
        ])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ScreenSampleSource {
    pub session_id: Uuid,
    pub hwnd: String,
    pub pid: u32,
    pub capture_method: CaptureMethod,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ScreenSamplePreview {
    pub id: Uuid,
    pub source: ScreenSampleSource,
    pub screenshot: Screenshot,
    pub sha256: String,
    pub bytes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ScreenSampleExport {
    pub id: Uuid,
    pub kind: ScreenSampleKind,
    pub note: String,
    pub image_path: String,
    pub metadata_path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct ScreenSampleState {
    pub preview: Option<ScreenSamplePreview>,
    pub saved: Option<ScreenSampleExport>,
}

#[derive(Default)]
pub struct ScreenSampleBook {
    state: ScreenSampleState,
    png: Vec<u8>,
}

/// 采集前核对调用者看到的目标；图片保存使用冻结的来源，不再读取游戏。
pub fn capture_target(
    snapshot: &Snapshot,
    hwnd: &str,
    method: CaptureMethod,
) -> Result<WindowInfo> {
    ensure!(
        snapshot
            .run
            .as_ref()
            .is_none_or(|run| run.state.is_terminal()),
        "已有任务运行、暂停或正在收尾，请先停止并等待完成"
    );
    ensure!(
        snapshot.record_error.is_none(),
        "运行记录尚未保存，请先重试保存"
    );
    ensure!(
        snapshot
            .run
            .as_ref()
            .is_none_or(|run| run.inputs_released()),
        "输入尚未确认释放，请先完成恢复"
    );
    let window = snapshot.window.as_ref().context("请先在总览连接游戏窗口")?;
    ensure!(
        window.hwnd == hwnd && snapshot.capture_method == Some(method),
        "游戏窗口或截图方式已变化，请重新核对后采集"
    );
    ensure!(
        window.width == 1920 && window.height == 1080,
        "素材采集要求 1920×1080 客户区"
    );
    Ok(window.clone())
}

impl ScreenSampleBook {
    pub fn state(&self) -> ScreenSampleState {
        self.state.clone()
    }
    pub fn clear(&mut self) {
        self.state = ScreenSampleState::default();
        self.png.clear();
    }

    pub fn capture(
        &mut self,
        session_id: Uuid,
        window: &WindowInfo,
        method: CaptureMethod,
        screenshot: Screenshot,
    ) -> Result<ScreenSampleState> {
        self.clear();
        ensure!(
            window.width == 1920
                && window.height == 1080
                && screenshot.width == 1920
                && screenshot.height == 1080
                && screenshot.capture_method == Some(method),
            "素材尺寸或截图方式不匹配"
        );
        ensure!(
            screenshot.color_order.len() <= 128
                && screenshot.quality.mean_luma.is_finite()
                && (0.0..=255.0).contains(&screenshot.quality.mean_luma)
                && screenshot.quality.dark_pixel_ratio.is_finite()
                && (0.0..=1.0).contains(&screenshot.quality.dark_pixel_ratio),
            "截图元信息无效"
        );
        ensure!(
            screenshot.png_base64.len() <= MAX_PNG_BYTES.div_ceil(3) * 4,
            "截图编码超过 10 MiB 限制"
        );
        let png = base64::engine::general_purpose::STANDARD
            .decode(&screenshot.png_base64)
            .context("截图编码无效")?;
        validate_png(&png)?;
        let preview = ScreenSamplePreview {
            id: Uuid::new_v4(),
            source: ScreenSampleSource {
                session_id,
                hwnd: window.hwnd.clone(),
                pid: window.pid,
                capture_method: method,
            },
            sha256: hex::encode(Sha256::digest(&png)),
            bytes: png.len() as u32,
            screenshot,
        };
        self.png = png;
        self.state.preview = Some(preview);
        Ok(self.state())
    }

    pub fn save(
        &mut self,
        data: &Path,
        id: Uuid,
        kind: ScreenSampleKind,
        note: &str,
    ) -> Result<ScreenSampleState> {
        let preview = self
            .state
            .preview
            .as_ref()
            .filter(|p| p.id == id)
            .context("预览已经变化或丢失，请重新采集")?;
        let note = note.trim();
        ensure!(
            note.chars().count() <= 240 && !note.chars().any(char::is_control),
            "备注最多 240 字且不能包含控制字符"
        );
        if let Some(saved) = &self.state.saved {
            ensure!(
                saved.kind == kind && saved.note == note,
                "这张预览已经保存；更换标注请重新采集"
            );
            return Ok(self.state());
        }
        ensure!(data.is_absolute(), "素材数据目录必须为绝对路径");
        let directory = data.join("diagnostics/screens");
        crate::config::reject_links(&directory)?;
        fs::create_dir_all(&directory)?;
        crate::config::reject_links(&directory)?;
        let destination = directory.join(format!(
            "{}-{}",
            preview.screenshot.captured_at.format("%Y%m%d-%H%M%S"),
            id
        ));
        crate::config::reject_links(&destination)?;
        ensure!(
            !destination.try_exists()?,
            "素材目录已存在，不会覆盖，请重新采集"
        );
        let staging = directory.join(format!(".{}-{}.tmp", id, Uuid::new_v4()));
        fs::create_dir(&staging)?;
        let metadata = json!({
            "schema_version":1,"scope":"user_reviewed_screen_sample","id":id,"source":preview.source,
            "annotation":{"kind":kind,"note":note,"source":"user"},"accepted":false,
            "image":{"file":"image.png","bytes":preview.bytes,"sha256":preview.sha256,"width":preview.screenshot.width,"height":preview.screenshot.height,
            "captured_at":preview.screenshot.captured_at,"elapsed_ms":preview.screenshot.elapsed_ms,"color_order":preview.screenshot.color_order,"quality":preview.screenshot.quality}
        });
        let result = (|| -> Result<()> {
            for (name, bytes) in [
                ("image.png", self.png.clone()),
                ("metadata.json", serde_json::to_vec_pretty(&metadata)?),
            ] {
                let path = staging.join(name);
                let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
            crate::config::reject_links(&directory)?;
            crate::config::reject_links(&destination)?;
            ensure!(!destination.try_exists()?, "素材目录已存在，不会覆盖");
            fs::rename(&staging, &destination)?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(staging.join("image.png"));
            let _ = fs::remove_file(staging.join("metadata.json"));
            let _ = fs::remove_dir(&staging);
            return Err(error).context("素材保存失败，可重试保存这张预览");
        }
        self.state.saved = Some(ScreenSampleExport {
            id,
            kind,
            note: note.into(),
            image_path: destination.join("image.png").to_string_lossy().into_owned(),
            metadata_path: destination
                .join("metadata.json")
                .to_string_lossy()
                .into_owned(),
            sha256: preview.sha256.clone(),
        });
        Ok(self.state())
    }
}
