//! 独立加载观察，只供已持有出口意图的父流程结合到达后态使用。
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, Word, words},
};
use anyhow::{Context, Result, ensure};
use od_core::lost_void_loading::{LoadingObservation, lore_loading, now_loading};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub(crate) struct LoadingTrace {
    id: [u8; 16],
    observation: Option<LoadingObservation>,
}
pub struct LostVoidExitTrace {
    slot: Arc<Mutex<Option<LoadingTrace>>>,
    id: [u8; 16],
}
impl LostVoidExitTrace {
    pub fn observation(&self) -> Result<Option<LoadingObservation>> {
        let slot = self
            .slot
            .lock()
            .map_err(|_| anyhow::anyhow!("加载记录不可用"))?;
        ensure!(
            slot.as_ref().is_some_and(|s| s.id == self.id),
            "加载记录不属于本次出口"
        );
        Ok(slot.as_ref().and_then(|s| s.observation.clone()))
    }
    pub fn finish(self) -> Result<Option<LoadingObservation>> {
        let observation = {
            let mut slot = self
                .slot
                .lock()
                .map_err(|_| anyhow::anyhow!("加载记录不可用"))?;
            ensure!(
                slot.as_ref().is_some_and(|s| s.id == self.id),
                "加载记录不属于本次出口"
            );
            slot.take().and_then(|s| s.observation)
        };
        Ok(observation)
    }
}
impl Drop for LostVoidExitTrace {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.slot.lock()
            && slot.as_ref().is_some_and(|s| s.id == self.id)
        {
            *slot = None;
        }
    }
}
fn gray_loading_art(frame: &RecognitionImage) -> Result<bool> {
    ensure!(
        frame.buffer.width() == 1920
            && frame.buffer.height() == 1080
            && frame.buffer.image_type() == 16,
        "加载观察要求 1080p BGR"
    );
    let pixels = frame.buffer.raw_data().context("加载观察没有像素")?;
    ensure!(pixels.len() == 1920 * 1080 * 3, "加载像素长度无效");
    let (mut total, mut visible, mut colorful) = (0u32, 0u32, 0u32);
    for y in (108..972).step_by(4) {
        for x in (192..1728).step_by(4) {
            let i = (y * 1920 + x) * 3;
            let rgb = &pixels[i..i + 3];
            let hi = u32::from(*rgb.iter().max().unwrap());
            let lo = u32::from(*rgb.iter().min().unwrap());
            total += 1;
            if hi > 20 {
                visible += 1;
                if (hi - lo) * 255 > 40 * hi {
                    colorful += 1;
                }
            }
        }
    }
    if visible * 4 < total || colorful as f64 / visible.max(1) as f64 >= 0.03 {
        return Ok(false);
    }
    let (mut n, mut sum, mut squares) = (0f64, 0f64, 0f64);
    for y in (80..900).step_by(4) {
        for x in (650..1750).step_by(4) {
            let i = (y * 1920 + x) * 3;
            let v = pixels[i..i + 3].iter().map(|v| f64::from(*v)).sum::<f64>() / 3.0;
            n += 1.0;
            sum += v;
            squares += v * v;
        }
    }
    // 在原低饱和度判断之上排除全黑和均匀灰帧，右侧必须有场景纹理。
    Ok((squares / n - (sum / n).powi(2)).max(0.0).sqrt() >= 12.0)
}
impl NativeSession {
    /// 仅在已保存出口意图的事件作用域使用；令牌结束或展开栈时自动清理。
    pub fn trace_lost_void_exit(&self, id: [u8; 16]) -> Result<LostVoidExitTrace> {
        ensure!(
            id != [0; 16] && self.inputs_released(),
            "加载记录缺少出口身份或输入收尾"
        );
        self.require_lost_void_scene()?;
        let mut slot = self
            .lost_void_loading_trace
            .lock()
            .map_err(|_| anyhow::anyhow!("加载记录不可用"))?;
        ensure!(slot.is_none(), "当前控制器已有加载记录作用域");
        *slot = Some(LoadingTrace {
            id,
            observation: None,
        });
        Ok(LostVoidExitTrace {
            slot: self.lost_void_loading_trace.clone(),
            id,
        })
    }
    pub(crate) fn observe_lost_void_exit_frame(
        &self,
        frame: &crate::navigation::NavigationFrame,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let id = {
            let slot = self
                .lost_void_loading_trace
                .lock()
                .map_err(|_| anyhow::anyhow!("加载记录不可用"))?;
            slot.as_ref()
                .filter(|s| s.observation.is_none())
                .map(|s| s.id)
        };
        let Some(id) = id else {
            return Ok(());
        };
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        let budget = Duration::from_millis(1500).saturating_sub(frame.age());
        let observation = self.lost_void_loading_sight(&frame.image, cancel, budget)?;
        if !cancel.load(Ordering::Acquire)
            && frame.age() <= Duration::from_millis(1500)
            && let Some(observation) = observation
        {
            let mut slot = self
                .lost_void_loading_trace
                .lock()
                .map_err(|_| anyhow::anyhow!("加载记录不可用"))?;
            if let Some(slot) = slot.as_mut()
                && slot.id == id
            {
                slot.observation = Some(observation);
            }
        }
        Ok(())
    }
    pub fn inspect_lost_void_loading(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LoadingObservation>> {
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let started = Instant::now();
        self.require_lost_void_scene()?;
        self.lost_void_loading_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))
    }
    pub(crate) fn lost_void_loading_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LoadingObservation>> {
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let started = Instant::now();
        let result = (|| -> Result<Option<LoadingObservation>> {
            let reader = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::lost_void_scene::pipeline(),
            };
            if reader.hit("LostVoid.WorldTab", None)? && reader.hit("LostVoid.BattleMenu", None)? {
                return Ok(None);
            }
            let read = |name: &str| words(&reader.read(name, None)?);
            let texts = |items: Vec<Word>| items.into_iter().map(|w| w.text).collect::<Vec<_>>();
            if let Some(observation) = now_loading(&texts(read("LostVoid.LoadingNow")?)) {
                return Ok(Some(observation));
            }
            let title = texts(read("LostVoid.LoadingLoreTitle")?);
            if title.len() != 1 || !title[0].starts_with(['【', '[']) {
                return Ok(None);
            }
            let mut body = read("LostVoid.LoadingLoreBody")?;
            body.sort_by_key(|w| (w.center()[1], w.rect[0]));
            let mut rows: Vec<Vec<Word>> = vec![];
            for word in body {
                if let Some(row) = rows.last_mut()
                    && (row[0].center()[1] - word.center()[1]).abs() < 14
                {
                    row.push(word);
                } else {
                    rows.push(vec![word]);
                }
            }
            let body: Vec<_> = rows
                .into_iter()
                .map(|mut row| {
                    row.sort_by_key(|w| w.rect[0]);
                    row.into_iter().map(|w| w.text).collect::<String>()
                })
                .collect();
            let logo = reader.hit("LostVoid.LoadingLogo", None)?;
            Ok(lore_loading(&title, &body, logo, gray_loading_art(frame)?))
        })();
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        result
    }
}
