use crate::{RunRecord, RunState, Snapshot, run_plan::RunContext};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RunNotificationMode {
    Off,
    #[default]
    Errors,
    All,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RunNotificationSettings {
    pub mode: RunNotificationMode,
}

impl RunNotificationMode {
    pub fn options() -> serde_json::Value {
        serde_json::json!([{ "value": "off", "label": "关闭" }, { "value": "errors", "label": "仅异常" }, { "value": "all", "label": "全部结果" }])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RunNoticeLevel {
    Success,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RunNotice {
    pub id: Uuid,
    pub session_id: Uuid,
    pub run_id: Uuid,
    pub instance_id: u32,
    pub instance_name: String,
    pub group_name: String,
    #[ts(type = "string")]
    pub at: DateTime<Utc>,
    pub level: RunNoticeLevel,
    pub title: String,
    pub message: String,
    pub read: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct RunNotifications {
    pub revision: u32,
    pub items: Vec<RunNotice>,
    pub dropped: u32,
}

struct WatchedRun {
    session: Uuid,
    id: Uuid,
    context: RunContext,
    mode: RunNotificationMode,
    save_warning: bool,
}

#[derive(Default)]
pub struct RunNotificationBook {
    armed: VecDeque<(Uuid, Uuid)>,
    watch: Option<WatchedRun>,
    items: VecDeque<RunNotice>,
    revision: u32,
    dropped: u32,
}

impl RunNotificationBook {
    pub const CAPACITY: usize = 32;
    pub fn snapshot(&self) -> RunNotifications {
        RunNotifications {
            revision: self.revision,
            items: self.items.iter().rev().cloned().collect(),
            dropped: self.dropped,
        }
    }
    pub fn arm(
        &mut self,
        session: Uuid,
        run: &RunRecord,
        settings: RunNotificationSettings,
        now: DateTime<Utc>,
    ) {
        if self.armed.contains(&(session, run.id))
            || run.state != RunState::Running
            || run.group.is_none()
            || !run.matches_account_context()
            || run
                .context
                .as_ref()
                .is_none_or(|context| context.validate().is_err())
        {
            return;
        }
        self.disconnected(now);
        if self.armed.len() == 128 {
            self.armed.pop_front();
        }
        self.armed.push_back((session, run.id));
        if settings.mode != RunNotificationMode::Off {
            self.watch = Some(WatchedRun {
                session,
                id: run.id,
                context: run.context.clone().expect("已检查账号上下文"),
                mode: settings.mode,
                save_warning: false,
            });
        }
    }
    pub fn observe(&mut self, session: Uuid, snapshot: &Snapshot, now: DateTime<Utc>) {
        let Some(mut watch) = self.watch.take() else {
            return;
        };
        let Some(run) = snapshot.run.as_ref().filter(|run| {
            run.id == watch.id
                && session == watch.session
                && run.context.as_ref() == Some(&watch.context)
        }) else {
            self.push(
                &watch,
                RunNoticeLevel::Warning,
                "运行结果未确认",
                "会话或任务已经变化，请检查运行记录及输入状态。",
                now,
            );
            return;
        };
        if snapshot.record_error.is_some() {
            if !watch.save_warning {
                self.push(
                    &watch,
                    RunNoticeLevel::Warning,
                    "运行记录尚未保存",
                    "请在运行页重试保存记录；当前结果尚未确认保存，输入状态请一并核对。",
                    now,
                );
                watch.save_warning = true;
            }
            self.watch = Some(watch);
            return;
        }
        watch.save_warning = false;
        if !run.state.is_terminal() {
            self.watch = Some(watch);
            return;
        }
        let (level, title, message) = if !run.inputs_released() {
            (
                RunNoticeLevel::Error,
                "输入尚未确认释放",
                "任务已结束，但输入尚未确认释放。请先检查并恢复遗留输入。",
            )
        } else {
            match run.state {
                RunState::Succeeded if run.completed_group_successfully() => (
                    RunNoticeLevel::Success,
                    if watch.context.standalone {
                        "独立应用已完成"
                    } else {
                        "应用组已完成"
                    },
                    if watch.context.standalone {
                        "独立应用已正常完成，运行记录已保存，输入已释放。"
                    } else {
                        "应用组已正常完成，运行记录已保存，输入已释放。"
                    },
                ),
                RunState::Failed => (
                    RunNoticeLevel::Error,
                    if watch.context.standalone {
                        "独立应用运行失败"
                    } else {
                        "应用组运行失败"
                    },
                    "运行记录已保存，输入已释放。请查看运行记录中的失败原因。",
                ),
                RunState::Stopped => (
                    RunNoticeLevel::Info,
                    if watch.context.standalone {
                        "独立应用已停止"
                    } else {
                        "应用组已停止"
                    },
                    "任务已主动停止，运行记录已保存，输入已释放。",
                ),
                _ => (
                    RunNoticeLevel::Warning,
                    "运行结果需要检查",
                    "任务已结束，但完整成功条件尚未确认。请查看运行记录。",
                ),
            }
        };
        self.push(&watch, level, title, message, now);
    }
    pub fn disconnected(&mut self, now: DateTime<Utc>) {
        if let Some(watch) = self.watch.take() {
            self.push(
                &watch,
                RunNoticeLevel::Warning,
                "运行连接中断",
                "本次运行的最终结果未确认，请检查游戏、输入状态和运行记录。重连不会自动继续任务。",
                now,
            );
        }
    }
    pub fn closing(&mut self) {
        self.watch = None;
    }
    pub fn update(&mut self, ids: &[Uuid], remove: bool) -> Result<RunNotifications, &'static str> {
        let selected: HashSet<_> = ids.iter().copied().collect();
        if ids.len() > Self::CAPACITY || selected.len() != ids.len() {
            return Err("提醒数量超过限制或存在重复标识，请刷新后重试");
        }
        let mut changed = false;
        if remove {
            let count = self.items.len();
            self.items.retain(|item| !selected.contains(&item.id));
            changed = self.items.len() != count;
        } else {
            for item in &mut self.items {
                if selected.contains(&item.id) && !item.read {
                    item.read = true;
                    changed = true;
                }
            }
        }
        if changed {
            self.revision = self.revision.saturating_add(1);
        }
        Ok(self.snapshot())
    }
    fn push(
        &mut self,
        watch: &WatchedRun,
        level: RunNoticeLevel,
        title: &str,
        message: &str,
        now: DateTime<Utc>,
    ) {
        if watch.mode == RunNotificationMode::Off
            || (watch.mode == RunNotificationMode::Errors
                && matches!(level, RunNoticeLevel::Success | RunNoticeLevel::Info))
        {
            return;
        }
        if self.items.len() == Self::CAPACITY {
            self.items.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.items.push_back(RunNotice {
            id: Uuid::new_v4(),
            session_id: watch.session,
            run_id: watch.id,
            instance_id: watch.context.instance_id,
            instance_name: watch.context.instance_name.chars().take(80).collect(),
            group_name: watch
                .context
                .group_name
                .as_deref()
                .unwrap_or(&watch.context.group_id)
                .chars()
                .take(80)
                .collect(),
            at: now,
            level,
            title: title.into(),
            message: message.into(),
            read: false,
        });
        self.revision = self.revision.saturating_add(1);
    }
}
