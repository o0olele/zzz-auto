//! 内部体力实例的持久应用记录；不另建控制器或开放缺少证明的消费入口。
use crate::charge_worker::{ChargeWorker, ChargeWorkerEnd, ChargeWorkerSnapshot};
use anyhow::{Context, Result, ensure};
use od_core::{
    RunRecord, RunSlot, RunState,
    charge_application::{ChargeAppOutcome, ChargeAppProgress, ChargeAppSummary},
    config::{AccountConfig, ConfigStore},
    pipeline::PipelineRequest,
    run_history::{HistoryError, RunStore},
    run_plan::RunContext,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeApplicationEnd {
    Run(ChargeWorkerEnd),
    AwaitingHistorySave,
}
pub struct ChargeApplicationRun {
    worker: ChargeWorker,
    record: RunRecord,
    started: bool,
    ended: bool,
    end: Option<ChargeWorkerEnd>,
    pending_history: Option<RunRecord>,
    history_error: Option<String>,
    data_root: Option<std::path::PathBuf>,
}
pub(crate) fn progress_from_snapshot(snapshot: &ChargeWorkerSnapshot) -> Result<ChargeAppProgress> {
    let mut p = ChargeAppProgress::new(snapshot.request.seed.context.clone())
        .map_err(anyhow::Error::msg)?;
    p.stage = format!("{:?}", snapshot.stage);
    p.summary = snapshot
        .schedule
        .as_ref()
        .map(ChargeAppSummary::from_schedule)
        .transpose()
        .map_err(anyhow::Error::msg)?;
    p.resources = snapshot.observed.as_ref().and_then(|o| o.resources);
    p.active_ms = snapshot.active_ms;
    p.inputs_released = snapshot.inputs_released;
    Ok(p)
}
fn record_progress(
    record: &RunRecord,
    progress: ChargeAppProgress,
    message: String,
) -> Result<RunRecord> {
    let mut next = record.clone();
    next.state = progress.state;
    next.finished_at = next.state.is_terminal().then(chrono::Utc::now);
    next.charge = Some(Box::new(progress));
    next.message = message;
    ensure!(next.matches_account_context(), "体力应用记录与冻结账号不符");
    Ok(next)
}
impl ChargeApplicationRun {
    pub fn new(worker: ChargeWorker, config: &AccountConfig) -> Result<Self> {
        let snapshot = worker.snapshot();
        ensure!(
            worker.history_end().is_none()
                && snapshot.active_ms == 0
                && snapshot.schedule.is_none(),
            "体力应用记录必须先于本次执行建立"
        );
        let source = &snapshot.request.seed.context;
        ensure!(
            source.config_revision == config.revision,
            "体力历史启动修订已变化"
        );
        let account = config
            .instances
            .iter()
            .find(|a| a.id == source.instance_id)
            .context("体力历史账号不存在")?;
        let group = account
            .groups
            .iter()
            .find(|g| g.id == source.group_id)
            .context("体力历史应用组不存在")?;
        let context = RunContext {
            instance_id: source.instance_id,
            instance_name: account.name.clone(),
            group_id: group.id.clone(),
            group_name: Some(group.display_name().into()),
            config_revision: source.config_revision,
            app_ids: vec!["charge_plan".into()],
            rerun_completed: false,
            standalone: true,
            game_day: Some(source.game_day.clone()),
        };
        let record = RunSlot::default()
            .begin_charge(context, progress_from_snapshot(&snapshot)?)
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            worker,
            record,
            started: false,
            ended: false,
            end: None,
            pending_history: None,
            history_error: None,
            data_root: None,
        })
    }
    pub fn record(&self) -> &RunRecord {
        &self.record
    }
    pub fn control(&self) -> PipelineRequest {
        self.worker.history_control().requester()
    }
    pub fn awaiting_history_save(&self) -> bool {
        self.pending_history.is_some()
    }
    pub fn history_error(&self) -> Option<&str> {
        self.history_error.as_deref()
    }
    fn save_record(&mut self, history: &RunStore) -> Result<ChargeApplicationEnd> {
        if let Err(error) = history.save(&self.record) {
            self.pending_history = Some(self.record.clone());
            self.history_error = Some(error.to_string());
            return Ok(ChargeApplicationEnd::AwaitingHistorySave);
        }
        Ok(ChargeApplicationEnd::Run(
            self.end.context("体力应用缺少结果")?,
        ))
    }
    /// 补存历史从不重新执行体力流程；首次登记仍采用不能覆盖旧文件的路径。
    pub fn retry_history_save(&mut self, history: &RunStore) -> Result<()> {
        ensure!(
            self.data_root.as_deref() == Some(history.data_directory()),
            "不能向其他数据目录补存体力历史"
        );
        let record = self
            .pending_history
            .as_ref()
            .context("没有等待补存的体力历史")?;
        if self.started {
            history.save(record)?;
        } else {
            history.begin_charge_record(record)?;
            self.started = true;
        }
        self.pending_history = None;
        self.history_error = None;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.pending_history.is_none()
                && !self.ended
                && self.end == Some(ChargeWorkerEnd::Paused),
            "体力应用尚未安全暂停或仍待保存"
        );
        self.worker.resume()?;
        let mut progress = (**self.record.charge.as_ref().context("体力摘要缺失")?).clone();
        progress.state = RunState::Running;
        progress.outcome = None;
        progress.inputs_released = false;
        progress.world_returned = false;
        self.record = record_progress(&self.record, progress, "体力继续准备".into())?;
        self.end = None;
        Ok(())
    }
    pub fn retry_charge_result(&mut self, store: &ConfigStore, history: &RunStore) -> Result<()> {
        ensure!(
            history.data_directory() == store.data_directory(),
            "体力历史与配置须属于同一数据目录"
        );
        ensure!(
            self.data_root.as_deref() == Some(store.data_directory()),
            "体力结果不属于本次冻结数据目录"
        );
        ensure!(
            self.pending_history.is_none() && self.end == Some(ChargeWorkerEnd::AwaitingSave),
            "体力结果尚不能补交"
        );
        self.worker.retry_saved_result(store, |_| Ok(()))?;
        let progress = self.worker.history_progress(store)?;
        self.end = self.worker.history_end();
        self.ended = self.end.is_some_and(|e| e != ChargeWorkerEnd::Paused);
        self.record = record_progress(
            &self.record,
            progress,
            "原体力结果已补交，未重新执行".into(),
        )?;
        if let Err(error) = history.save(&self.record) {
            self.pending_history = Some(self.record.clone());
            self.history_error = Some(error.to_string());
            return Err(error.into());
        }
        Ok(())
    }
    pub fn execute(
        &mut self,
        store: &ConfigStore,
        history: &RunStore,
        mut publish: impl FnMut(RunRecord) -> Result<()>,
    ) -> Result<ChargeApplicationEnd> {
        let preparing = std::time::Instant::now();
        ensure!(
            history.data_directory() == store.data_directory(),
            "体力历史与配置须属于同一数据目录"
        );
        ensure!(
            self.data_root
                .as_deref()
                .is_none_or(|root| root == store.data_directory()),
            "体力数据目录已变化"
        );
        self.data_root = Some(store.data_directory().to_owned());
        ensure!(
            !self.ended && self.pending_history.is_none() && self.end.is_none(),
            "体力应用已结束或仍待保存"
        );
        if !self.started {
            if let Err(error) = history.begin_charge_record(&self.record) {
                if matches!(&error, HistoryError::Io(e) if e.kind() == std::io::ErrorKind::AlreadyExists)
                {
                    self.ended = true;
                    return Err(error.into());
                }
                self.pending_history = Some(self.record.clone());
                self.history_error = Some(error.to_string());
                return Ok(ChargeApplicationEnd::AwaitingHistorySave);
            }
            self.started = true;
        } else if let Err(error) = history.save(&self.record) {
            self.pending_history = Some(self.record.clone());
            self.history_error = Some(error.to_string());
            return Err(error.into());
        }
        self.worker.include_history_time(preparing.elapsed());
        let record = &mut self.record;
        let result = self.worker.execute(store, |snapshot| {
            let progress = progress_from_snapshot(&snapshot)?;
            let next = record_progress(record, progress, "体力任务执行中".into())?;
            history.save(&next)?;
            publish(next.clone())?;
            *record = next;
            Ok(())
        });
        let finishing_started = std::time::Instant::now();
        self.worker.refresh_history_control();
        let mut progress = self.worker.history_progress(store).unwrap_or_else(|_| {
            let mut p =
                progress_from_snapshot(&self.worker.snapshot()).expect("本次体力身份已验证");
            p.state = RunState::Failed;
            p.outcome = Some(ChargeAppOutcome::Failed);
            p
        });
        // 先发布收尾检查点再决定最终记录，控制请求仍可撤销尚未提交的应用成功。
        let mut finishing = progress.clone();
        finishing.state = RunState::Running;
        finishing.outcome = None;
        finishing.stage = "Finishing".into();
        let notification = publish(record_progress(
            &self.record,
            finishing,
            "体力结果保存准备".into(),
        )?);
        self.worker.refresh_history_control();
        self.worker
            .include_history_time(finishing_started.elapsed());
        let mut final_progress = self.worker.history_progress(store);
        if final_progress.is_err() && self.worker.history_control().requested().is_some() {
            // 读日志期间到达的请求也须按暂停 / 停止处理，不能变成普通失败。
            self.worker.refresh_history_control();
            final_progress = self.worker.history_progress(store);
        }
        if notification.is_ok()
            && result.is_ok()
            && let Ok(final_progress) = final_progress
        {
            progress = final_progress;
            self.end = self.worker.history_end();
        } else {
            progress.state = RunState::Failed;
            progress.outcome = Some(ChargeAppOutcome::Failed);
            self.end = Some(ChargeWorkerEnd::Failed);
        }
        if progress.state == RunState::Succeeded
            && self
                .worker
                .history_control()
                .finish_if_uninterrupted()
                .is_err()
        {
            self.worker.refresh_history_control();
            progress = self.worker.history_progress(store)?;
            self.end = self.worker.history_end();
        }
        self.ended = self
            .end
            .is_some_and(|e| !matches!(e, ChargeWorkerEnd::Paused | ChargeWorkerEnd::AwaitingSave));
        let message = match result.and(notification) {
            Ok(()) => format!(
                "体力{}，已核实完成 {} 次",
                progress.outcome.map_or("已暂停", ChargeAppOutcome::label),
                progress.summary.as_ref().map_or(0, |s| s.successful_runs)
            ),
            Err(error) => format!("体力运行未完成：{error}"),
        };
        self.record = record_progress(&self.record, progress, message)?;
        self.save_record(history)
    }
}
