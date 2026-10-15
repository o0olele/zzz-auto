use crate::NativeSession;
use anyhow::{Result, ensure};
use maa_framework::tasker::Tasker;
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver},
};

#[derive(Debug, Clone, Serialize)]
pub struct PipelineEvent {
    pub message: String,
    pub task_id: Option<i64>,
    pub node: Option<String>,
}

pub struct PipelineEvents {
    pub receiver: Receiver<PipelineEvent>,
    dropped: Arc<AtomicU64>,
}

pub(crate) struct PipelineSubscription {
    tasker: Tasker,
    task_sink: i64,
    context_sink: i64,
}

impl Drop for PipelineSubscription {
    fn drop(&mut self) {
        // 先注销原生 context，再让绑定释放回调；不能等 Tasker 析构才清理指针。
        self.tasker.remove_context_sink(self.context_sink);
        self.tasker.remove_sink(self.task_sink);
    }
}

impl PipelineEvents {
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl NativeSession {
    pub fn pipeline_events(&mut self, capacity: usize) -> Result<PipelineEvents> {
        ensure!((1..=4096).contains(&capacity), "事件队列容量须为 1..4096");
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let callback_dropped = dropped.clone();
        let callback = move |message: &str, details: &str| {
            if !(message.starts_with("Node.PipelineNode.")
                || message.starts_with("Node.Action.")
                || message.starts_with("Tasker.Task."))
            {
                return;
            }
            if details.len() > 65_536 {
                callback_dropped.fetch_add(1, Ordering::Relaxed);
                return;
            }
            let details: serde_json::Value = serde_json::from_str(details).unwrap_or_default();
            let event = PipelineEvent {
                message: message.to_owned(),
                task_id: details["task_id"].as_i64(),
                node: details["name"]
                    .as_str()
                    .filter(|name| name.len() <= 256)
                    .map(str::to_owned),
            };
            // 回调不等待所属 Tasker，不取得业务状态锁，不输出 OCR / focus / 任意详情。
            if sender.try_send(event).is_err() {
                callback_dropped.fetch_add(1, Ordering::Relaxed);
            }
        };
        let task_sink = self.tasker.add_sink(callback.clone())?;
        let context_sink = match self.tasker.add_context_sink(callback) {
            Ok(id) => id,
            Err(error) => {
                self.tasker.remove_sink(task_sink);
                return Err(error.into());
            }
        };
        self.pipeline_subscriptions.push(PipelineSubscription {
            tasker: self.tasker.clone(),
            task_sink,
            context_sink,
        });
        Ok(PipelineEvents { receiver, dropped })
    }
}
