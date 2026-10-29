//! 统一原生线程中的体力分项，暂停和补存保留同一个执行器。
use super::*;
use crate::{
    charge_entry::PreparedCharge,
    charge_worker::{ChargeWorker, ChargeWorkerEnd},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    charge_application::{ChargeAppOutcome, ChargeAppProgress},
    pipeline::{Interrupt, PipelineControl},
};

pub(super) struct ChargeWork {
    id: Uuid,
    prepared: Option<PreparedCharge>,
    control: PipelineControl,
    window: WindowInfo,
    capture: CaptureMethod,
    fixture: bool,
    pub(super) retry_save: bool,
    error: Option<String>,
}
impl ChargeWork {
    pub(super) fn new(
        id: Uuid,
        prepared: PreparedCharge,
        control: PipelineControl,
        window: WindowInfo,
        capture: CaptureMethod,
        fixture: bool,
    ) -> Self {
        Self {
            id,
            prepared: Some(prepared),
            control,
            window,
            capture,
            fixture,
            retry_save: false,
            error: None,
        }
    }
    pub(super) fn verify(&self) -> Result<()> {
        self.prepared.as_ref().context("体力资源尚未冻结")?.verify()
    }
    pub(super) fn control(&mut self, resume: bool, worker: &mut Option<ChargeWorker>) {
        if !resume || self.control.requested() == Some(Interrupt::Stop) {
            self.control.stop();
            return;
        }
        let result = if let Some(run) = worker {
            run.resume()
        } else {
            ensure_resume(&self.control)
        };
        if let Err(e) = result {
            self.error = Some(format!("体力继续失败：{e:#}"));
        }
    }
    fn prepare(
        &mut self,
        paths: &EnginePaths,
        native: &mut Option<NativeSession>,
        worker: &mut Option<ChargeWorker>,
    ) -> Result<()> {
        if worker.is_some() {
            return Ok(());
        }
        if self.fixture {
            native.as_mut().context("离线体力控制器缺失")?.quiesce()?;
            native
                .as_ref()
                .unwrap()
                .check_combat_input_mode(InputMode::Foreground)?;
        } else {
            let handoff = NativeSession::handoff_game(
                native,
                maa_adapter::GameSessionConfig {
                    window: &self.window,
                    resources: &paths.resources,
                    capture: self.capture,
                    input: InputMode::Foreground,
                    device: InferenceDevice::Cpu,
                },
                &self.control.cancellation(),
            )?;
            if handoff == maa_adapter::SessionHandoff::Cancelled {
                return Ok(());
            }
        }
        if self.control.requested().is_some() {
            return Ok(());
        }
        *worker = Some(self.prepared.take().context("体力冻结计划缺失")?.create(
            self.id,
            native.take().context("体力会话缺失")?,
            self.control.clone(),
        )?);
        Ok(())
    }
}
fn ensure_resume(control: &PipelineControl) -> Result<()> {
    ensure!(control.resume(), "体力尚未暂停或已经停止");
    Ok(())
}
/// 编队在体力之后执行时，只接纳本运行日志及去重回执对应的次数修订。
pub(super) fn revision_after_charge(record: &RunRecord, store: &ConfigStore) -> Result<u32> {
    let context = record.context.as_ref().context("应用组缺少配置身份")?;
    let config = store.read()?;
    let Some(charge) = &record.charge else {
        return Ok(context.config_revision);
    };
    let Some(receipt) = config
        .charge_commits
        .iter()
        .find(|r| r.context.run_id == record.id)
    else {
        return Ok(context.config_revision);
    };
    ensure!(
        charge.inputs_released
            && charge.counts_committed
            && charge.matches_record(record)
            && charge.config_revision_after == Some(receipt.saved_revision),
        "体力次数尚未完成交接"
    );
    let journal = store
        .charge_checkpoint(record.id)?
        .context("体力次数缺少原日志")?;
    charge
        .verify_schedule(&journal)
        .map_err(anyhow::Error::msg)?;
    ensure!(
        receipt.context == charge.context
            && receipt.records as usize == journal.records().len()
            && receipt.prefix_sha256
                == journal
                    .prefix_fingerprint(journal.records().len())
                    .map_err(anyhow::Error::msg)?
            && receipt.saved_revision == config.revision,
        "体力提交回执与当前应用组修订不符"
    );
    Ok(receipt.saved_revision)
}
fn before_worker(work: &ChargeWork, state: &State) -> Result<ChargeAppProgress> {
    let mut p = (**state
        .slot
        .current()
        .and_then(|r| r.charge.as_ref())
        .context("体力启动检查点缺失")?)
    .clone();
    p.inputs_released = true;
    p.state = match work.control.requested() {
        Some(Interrupt::Pause) if work.control.acknowledge_pause() => RunState::Paused,
        Some(_) => RunState::Stopped,
        None => RunState::Failed,
    };
    p.outcome = match p.state {
        RunState::Paused => None,
        RunState::Stopped => Some(ChargeAppOutcome::Stopped),
        _ => Some(ChargeAppOutcome::Failed),
    };
    Ok(p)
}

pub(super) fn run_charge_work(
    mut work: Box<ChargeWork>,
    paths: &EnginePaths,
    shared: &Arc<Mutex<State>>,
    native: &mut Option<NativeSession>,
    worker: &mut Option<ChargeWorker>,
) -> Option<Box<ChargeWork>> {
    let store = match ConfigStore::new(&paths.data) {
        Ok(s) => s,
        Err(e) => {
            work.error = Some(format!("体力配置不可用：{e}"));
            return fail_prepared(work, paths, shared);
        }
    };
    let result = (|| -> Result<()> {
        ensure!(
            work.error.is_none(),
            "{}",
            work.error.as_deref().unwrap_or_default()
        );
        work.prepare(paths, native, worker)?;
        let Some(run) = worker.as_mut() else {
            return Ok(());
        };
        if run.history_end() == Some(ChargeWorkerEnd::AwaitingSave) {
            if work.retry_save {
                work.retry_save = false;
                run.retry_saved_result(&store, |_| Ok(()))?;
            }
            return Ok(());
        }
        run.execute(&store, |snapshot| {
            let progress = crate::charge_application::progress_from_snapshot(&snapshot)?;
            let mut state = shared.lock().expect("运行状态锁失效");
            let record = state
                .slot
                .update_charge(work.id, progress, "体力执行中".into())
                .map_err(anyhow::Error::msg)?;
            save_run(paths, &mut state, &record);
            ensure!(
                state.record_error.is_none(),
                "体力检查点保存失败，已停止后续输入"
            );
            Ok(())
        })?;
        Ok(())
    })();
    if let Err(e) = &result {
        work.error = Some(format!("体力未完成：{e:#}"));
    }
    let verifying = std::time::Instant::now();
    if let Some(run) = worker.as_mut() {
        run.refresh_history_control();
    }
    // 控制命令也持有此锁：最终摘要和分项交接之间不再接受旧分项的控制请求。
    let mut state = shared.lock().expect("运行状态锁失效");
    let mut progress = if let Some(run) = worker.as_mut() {
        let mut p = run.history_progress(&store);
        run.include_history_time(verifying.elapsed());
        run.refresh_history_control();
        if p.is_err() || run.history_end() != Some(ChargeWorkerEnd::Completed) {
            run.history_progress(&store)
        } else {
            // 复用已核验的摘要时，也须带上本次结果核验耗时。
            if let Ok(progress) = &mut p {
                progress.active_ms = run.snapshot().active_ms;
            }
            p
        }
    } else {
        before_worker(&work, &state)
    };
    if let Ok(p) = &progress
        && p.state == RunState::Succeeded
        && work.control.finish_if_uninterrupted().is_err()
        && let Some(run) = worker.as_mut()
    {
        run.refresh_history_control();
        progress = run.history_progress(&store);
    }
    let mut p = match progress {
        Ok(p) => p,
        Err(e) => {
            work.error = Some(format!("体力结果校验失败：{e:#}"));
            let mut p = (**state.slot.current()?.charge.as_ref()?).clone();
            p.inputs_released = worker.as_ref().is_none_or(|r| r.snapshot().inputs_released);
            p.state = RunState::Failed;
            p.outcome = Some(ChargeAppOutcome::Failed);
            p
        }
    };
    let awaiting = worker
        .as_ref()
        .is_some_and(|r| r.history_end() == Some(ChargeWorkerEnd::AwaitingSave));
    if work.error.is_some() && !awaiting {
        p.state = RunState::Failed;
        p.outcome = Some(ChargeAppOutcome::Failed);
    }
    if awaiting {
        work.control.pause();
        work.control.acknowledge_pause();
    }
    let keep = awaiting || p.state == RunState::Paused;
    if !keep && let Some(run) = worker.as_mut() {
        match run.take_released_session() {
            Ok(session) => *native = Some(session),
            Err(e) => {
                p.state = RunState::Failed;
                p.outcome = Some(ChargeAppOutcome::Failed);
                p.inputs_released = false;
                work.error = Some(format!("体力会话交回失败：{e:#}"));
            }
        }
    }
    let message = work.error.clone().unwrap_or_else(|| {
        format!(
            "体力{}，已核实完成 {} 次",
            p.outcome.map_or("已暂停", ChargeAppOutcome::label),
            p.summary.as_ref().map_or(0, |s| s.successful_runs)
        )
    });
    let record = state.slot.update_charge(work.id, p, message);
    match record {
        Ok(record) => {
            save_run(paths, &mut state, &record);
            if awaiting {
                state.record_error = Some(
                    work.error
                        .clone()
                        .unwrap_or_else(|| "体力次数待提交，请重试保存；补交后再继续".into()),
                );
            } else if !work.retry_save && result.is_ok() && state.failed_save.is_none() {
                state.record_error = None;
            }
            state.event(Some(work.id), "info", record.message);
        }
        Err(e) => {
            if let Some(record) = fail_current(&mut state, format!("体力检查点无效：{e}")) {
                save_run(paths, &mut state, &record);
            }
            state.pipeline_control = None;
            return None;
        }
    }
    if keep {
        work.error = None;
        Some(work)
    } else {
        *worker = None;
        state.pipeline_control = None;
        None
    }
}
fn fail_prepared(
    work: Box<ChargeWork>,
    paths: &EnginePaths,
    shared: &Arc<Mutex<State>>,
) -> Option<Box<ChargeWork>> {
    let mut state = shared.lock().expect("运行状态锁失效");
    if let Some(record) = fail_current(
        &mut state,
        work.error.clone().unwrap_or_else(|| "体力准备失败".into()),
    ) {
        save_run(paths, &mut state, &record);
    }
    state.pipeline_control = None;
    None
}
