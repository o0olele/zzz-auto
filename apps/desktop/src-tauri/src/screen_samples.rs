use crate::session::EngineSession;
use od_core::{
    CaptureMethod,
    screen_samples::{ScreenSampleBook, ScreenSampleState},
};
use od_protocol::{ApiError, Command, Payload};
use std::sync::{Arc, Mutex};

pub async fn library_page(
    data: std::path::PathBuf,
    query: od_core::screen_sample_library::ScreenSampleQuery,
) -> Result<od_core::screen_sample_library::ScreenSamplePage, ApiError> {
    tokio::task::spawn_blocking(move || {
        od_core::screen_sample_library::list(&data, &query).map_err(failed)
    })
    .await
    .map_err(failed)?
}

pub async fn open_saved(
    data: std::path::PathBuf,
    key: String,
    metadata_sha256: String,
) -> Result<od_core::screen_sample_library::ScreenSampleView, ApiError> {
    tokio::task::spawn_blocking(move || {
        od_core::screen_sample_library::open(&data, &key, &metadata_sha256).map_err(failed)
    })
    .await
    .map_err(failed)?
}

pub async fn capture(
    session: &mut EngineSession,
    cache: &Arc<Mutex<ScreenSampleBook>>,
    hwnd: &str,
    method: CaptureMethod,
) -> Result<ScreenSampleState, ApiError> {
    cache.lock().map_err(|_| failed("素材缓存不可用"))?.clear();
    session.require_recovered()?;
    let Payload::Snapshot(snapshot) = session.request(Command::Snapshot).await? else {
        return Err(failed("无法确认游戏窗口和运行状态"));
    };
    let window =
        od_core::screen_samples::capture_target(&snapshot, hwnd, method).map_err(failed)?;
    let Payload::Screenshot(screenshot) = session.request(Command::Screenshot).await? else {
        return Err(failed("引擎未返回截图"));
    };
    let cache = cache.clone();
    let session_id = session.id;
    tokio::task::spawn_blocking(move || {
        cache
            .lock()
            .map_err(|_| failed("素材缓存不可用"))?
            .capture(session_id, &window, method, screenshot)
            .map_err(failed)
    })
    .await
    .map_err(failed)?
}

pub fn failed(error: impl std::fmt::Display) -> ApiError {
    ApiError::new("screen_sample_error", error.to_string())
}

pub async fn clear_and_wait(cache: Arc<Mutex<ScreenSampleBook>>) -> Result<(), ApiError> {
    tokio::task::spawn_blocking(move || {
        cache.lock().map_err(|_| failed("素材缓存不可用"))?.clear();
        Ok(())
    })
    .await
    .map_err(failed)?
}
