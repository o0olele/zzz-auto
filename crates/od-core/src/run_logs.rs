use crate::{RunEvent, RunState, Snapshot};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};
use ts_rs::TS;
use uuid::Uuid;

pub const SESSION_LIMIT: usize = 4;
pub const EVENT_LIMIT: usize = 256;
pub const MESSAGE_LIMIT: usize = 8192;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LogRun {
    pub id: Uuid,
    pub app_id: String,
    pub state: RunState,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LogSession {
    pub id: Uuid,
    pub engine_version: String,
    #[ts(type = "string")]
    pub captured_at: DateTime<Utc>,
    #[ts(type = "number")]
    pub event_sequence: u64,
    #[ts(type = "number")]
    pub dropped_events: u64,
    pub omitted_events: u32,
    pub truncated_events: u32,
    pub run: Option<LogRun>,
    pub events: Vec<RunEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LogSessionInfo {
    pub id: Uuid,
    #[ts(type = "string")]
    pub captured_at: DateTime<Utc>,
    pub event_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopLogs {
    pub sessions: Vec<LogSessionInfo>,
    pub selected: Option<LogSession>,
    pub discarded_sessions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LogExport {
    pub path: String,
    pub session_id: Uuid,
    pub event_count: u32,
}

#[derive(Default)]
pub struct LogBook {
    sessions: VecDeque<LogSession>,
    discarded_sessions: u32,
}

fn shorten(value: &str, limit: usize) -> String {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

impl LogBook {
    pub fn observe(&mut self, id: Uuid, snapshot: &Snapshot) {
        let omitted = snapshot.events.len().saturating_sub(EVENT_LIMIT);
        let mut truncated = 0;
        let events = snapshot
            .events
            .iter()
            .skip(omitted)
            .map(|event| {
                if event.message.len() > MESSAGE_LIMIT || event.level.len() > 32 {
                    truncated += 1;
                }
                RunEvent {
                    sequence: event.sequence,
                    run_id: event.run_id,
                    at: event.at,
                    level: shorten(&event.level, 32),
                    message: shorten(&event.message, MESSAGE_LIMIT),
                }
            })
            .collect();
        let session = LogSession {
            id,
            engine_version: shorten(&snapshot.engine_version, 128),
            captured_at: Utc::now(),
            event_sequence: snapshot.event_sequence,
            dropped_events: snapshot.dropped_events,
            omitted_events: omitted.try_into().unwrap_or(u32::MAX),
            truncated_events: truncated,
            run: snapshot.run.as_ref().map(|run| LogRun {
                id: run.id,
                app_id: shorten(&run.app_id, 256),
                state: run.state,
            }),
            events,
        };
        if let Some(index) = self.sessions.iter().position(|session| session.id == id) {
            self.sessions.remove(index);
        }
        self.sessions.push_front(session);
        if self.sessions.len() > SESSION_LIMIT {
            self.sessions.pop_back();
            self.discarded_sessions = self.discarded_sessions.saturating_add(1);
        }
    }

    pub fn view(&self, selected: Option<Uuid>) -> Result<DesktopLogs> {
        let session = match selected {
            Some(id) => Some(
                self.sessions
                    .iter()
                    .find(|session| session.id == id)
                    .context("该会话日志已淘汰，请选择仍保留的会话")?,
            ),
            None => self.sessions.front(),
        };
        Ok(DesktopLogs {
            sessions: self
                .sessions
                .iter()
                .map(|session| LogSessionInfo {
                    id: session.id,
                    captured_at: session.captured_at,
                    event_count: session.events.len() as u32,
                })
                .collect(),
            selected: session.cloned(),
            discarded_sessions: self.discarded_sessions,
        })
    }
}

pub fn export(data: &Path, session: &LogSession) -> Result<LogExport> {
    ensure!(data.is_absolute(), "日志导出数据目录必须为绝对路径");
    ensure!(
        session.events.len() <= EVENT_LIMIT
            && session
                .events
                .iter()
                .all(|event| event.message.len() <= MESSAGE_LIMIT && event.level.len() <= 32),
        "日志快照超过导出限制"
    );
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "schema_version": 1, "scope": "retained_engine_events", "session": session,
    }))?;
    ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "日志快照超过 16 MiB 导出限制"
    );
    let directory = data.join("logs/exports");
    crate::config::reject_links(&directory)?;
    fs::create_dir_all(&directory)?;
    crate::config::reject_links(&directory)?;
    let path = directory.join(format!(
        "engine-{}-{}.json",
        Utc::now().format("%Y%m%d-%H%M%S"),
        Uuid::new_v4()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    let result = file.write_all(&bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = result {
        let _ = fs::remove_file(&path);
        return Err(error).context("日志导出写入失败");
    }
    Ok(LogExport {
        path: path.to_string_lossy().into_owned(),
        session_id: session.id,
        event_count: session.events.len() as u32,
    })
}
