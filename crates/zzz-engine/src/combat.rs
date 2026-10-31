use anyhow::{Context, Result, ensure};
use maa_adapter::{
    NativeSession,
    combat_vision::{CombatFrame, CombatObservation, CombatRecognizer},
};
use od_core::{
    RunState,
    battle_result::{BattleEndTracker, NormalBattleResult},
    combat::{CombatConfig, CombatProgress},
    combat_inputs::InputLedger,
    conditional::{Action, ButtonWay, Decision, Scheduler, Script, StateUpdate},
    pipeline::{Interrupt, PipelineControl, PipelineRequest},
};
use serde_json::json;
use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub fn script_catalog(
    bundle: &maa_adapter::combat_vision::CombatBundle,
    data: &std::path::Path,
) -> Result<od_core::combat::CombatScriptCatalog> {
    use sha2::Digest;
    let (imported, battle, dodge) = od_core::combat_scripts::catalog(data)?;
    let mut digest = sha2::Sha256::new();
    digest.update(std::fs::read(bundle.root.join("combat-manifest.json"))?);
    digest.update(imported.as_deref().unwrap_or("").as_bytes());
    Ok(od_core::combat::CombatScriptCatalog {
        revision: hex::encode(digest.finalize()),
        imported,
        battle: bundle
            .scripts
            .iter()
            .cloned()
            .chain(battle)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        dodge: bundle
            .dodge_scripts
            .iter()
            .cloned()
            .chain(dodge)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
    })
}

enum Step {
    Action(Action),
    Button(String, bool),
    ButtonConfirmed(String, bool),
    NativeInput(String, od_core::InputProbeAction, bool),
    Move(i32),
    Delay(f64),
    ClearInputs,
    ReleaseTurn,
}

struct PendingInput {
    job: maa_adapter::CombatInputJob,
    started: Instant,
    data: serde_json::Value,
    trace: Option<crate::combat_performance::PerformanceTrace>,
    finished: bool,
    refresh_agents: bool,
}
impl PendingInput {
    fn poll(&mut self, native: &mut NativeSession) -> Result<bool> {
        let result = native.poll_combat_input(&mut self.job);
        if !matches!(&result, Ok(false)) {
            self.finished = true;
            if let Some(trace) = &self.trace {
                self.data["duration_us"] = json!(self.started.elapsed().as_micros() as u64);
                self.data["sent"] = json!(result.as_ref().is_ok_and(|done| *done));
                trace.event("input", self.data.take());
            }
        }
        result
    }
}
impl Drop for PendingInput {
    fn drop(&mut self) {
        if !self.finished
            && let Some(trace) = &self.trace
        {
            trace.event("input_aborted", self.data.take());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatSegmentEnd {
    Result(NormalBattleResult),
    Interrupted(Interrupt),
}

/// 每次新截图在状态识别和动作调度前决定；等待不能沿用旧输入或状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatFrameGate {
    Proceed,
    Hold,
    /// 同次战斗的临时重读，保留已生效的自定义状态和冷却；仍禁止输入。
    Refresh,
    Yield,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatResultPolicy {
    NormalBattle,
    ExternalBoundary,
}
pub struct CombatGuardOptions {
    pub budget: Duration,
    pub result_policy: CombatResultPolicy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardedCombatSegmentEnd {
    Result(NormalBattleResult),
    Yielded,
    Interrupted(Interrupt),
}
enum InnerEnd {
    Normal(NormalBattleResult),
    Yielded,
}
struct CombatLoopOptions<'a> {
    allow_audio: bool,
    check_end: bool,
    worker: Option<&'a mut crate::combat_worker::CombatWorker>,
}

pub struct CombatRun {
    pub id: Uuid,
    pub progress: CombatProgress,
    pub message: String,
    control: PipelineControl,
    scheduler: Scheduler,
    elapsed: f64,
    sequence: Option<u64>,
    random: u64,
    parented: bool,
    end_tracker: BattleEndTracker,
    parent_deadline: Option<Instant>,
    patrol_ui_timeout: Option<u16>,
    performance: Option<crate::combat_performance::PerformanceLog>,
}
impl CombatRun {
    pub fn new(id: Uuid, config: CombatConfig, script: Script) -> Result<Self> {
        config.validate().map_err(anyhow::Error::msg)?;
        config.validate_script(&script)?;
        let random = u64::from_le_bytes(id.as_bytes()[..8].try_into()?).max(1);
        Ok(Self {
            id,
            progress: CombatProgress::new(config),
            message: "战斗准备中".into(),
            control: PipelineControl::default(),
            scheduler: Scheduler::new(script),
            elapsed: 0.0,
            sequence: None,
            random,
            parented: false,
            end_tracker: BattleEndTracker::default(),
            parent_deadline: None,
            patrol_ui_timeout: None,
            performance: None,
        })
    }
    pub fn enable_performance(
        &mut self,
        data: &std::path::Path,
        capture: impl serde::Serialize,
    ) -> Result<std::path::PathBuf> {
        ensure!(self.performance.is_none(), "战斗性能日志已经开启");
        let (log, path) = crate::combat_performance::PerformanceLog::start(
            data,
            self.id,
            json!({
                "config": self.progress.config, "script_revision": self.progress.script_revision,
                "capture_method": capture,
            }),
        )?;
        self.performance = Some(log);
        Ok(path)
    }
    /// 应用内战斗与外层流程共用请求句柄，结束当前战斗不能结束整个应用。
    pub fn new_segment(
        id: Uuid,
        config: CombatConfig,
        script: Script,
        control: PipelineControl,
    ) -> Result<Self> {
        let mut run = Self::new(id, config, script)?;
        run.parented = true;
        run.control = control;
        Ok(run)
    }
    pub fn control(&self) -> PipelineRequest {
        self.control.requester()
    }
    pub fn for_world_patrol(mut self, ui_timeout: u16) -> Self {
        self.patrol_ui_timeout = Some(ui_timeout);
        self
    }
    pub fn stopping(&self) -> bool {
        self.control.requested() == Some(Interrupt::Stop)
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(!self.parented, "应用内战斗由外层流程继续");
        ensure!(
            self.progress.state == RunState::Paused
                && self.progress.inputs_released
                && self.control.resume(),
            "战斗尚未确认暂停或已请求停止"
        );
        self.scheduler.resume();
        self.sequence = None;
        self.progress.state = RunState::Running;
        Ok(())
    }
    /// 外层确认整个任务安全暂停后恢复子战斗，覆盖结果返回与父交接之间的暂停。
    pub fn resume_segment_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            self.parented
                && self.progress.inputs_released
                && !matches!(self.progress.state, RunState::Failed | RunState::Stopped)
                && self.control.resume(),
            "子战斗尚未由父任务确认暂停、输入未释放或已经失败 / 停止"
        );
        self.progress.state = RunState::Running;
        self.sequence = None;
        self.end_tracker.reset_confirmation();
        Ok(())
    }
    /// 慢帧撤销视觉判断和动作，不能抹掉已经执行的 BUFF、连招计数及场景冷却。
    fn refresh_observation(&mut self, pending: &mut Vec<StateUpdate>) -> Result<()> {
        for update in pending.iter().filter(|s| s.name.starts_with("自定义-")) {
            self.scheduler.states.apply(update)?;
        }
        pending.clear();
        self.scheduler.refresh_observation();
        Ok(())
    }
    /// 漏识别保留已确认队伍；仅在可见槽位符合其循环顺序时继续按名字切人。
    fn observe_team(&mut self, observed: &[String]) -> bool {
        let unique: std::collections::BTreeSet<_> =
            observed.iter().filter(|name| !name.is_empty()).collect();
        if unique.is_empty()
            || unique.len() != observed.iter().filter(|name| !name.is_empty()).count()
        {
            return false;
        }
        if unique.len() == observed.len() && observed.len() >= self.progress.team.len() {
            self.progress.team = observed.to_vec();
            return true;
        }
        let Some(front) = observed.first().filter(|name| !name.is_empty()) else {
            return false;
        };
        let Some(offset) = self.progress.team.iter().position(|name| name == front) else {
            return false;
        };
        let mut ordered = self.progress.team.clone();
        ordered.rotate_left(offset);
        if observed.len() > ordered.len()
            || observed
                .iter()
                .enumerate()
                .any(|(position, name)| !name.is_empty() && name != &ordered[position])
        {
            return false;
        }
        self.progress.team = ordered;
        true
    }
    fn intervals(&mut self) -> Result<BTreeMap<String, f64>> {
        let mut result = BTreeMap::new();
        for (name, default) in [
            ("check_dodge_interval", 0.02),
            ("check_agent_interval", 0.5),
            ("check_chain_interval", 1.0),
            ("check_quick_interval", 0.5),
            ("target_lock_interval", 1.0),
            ("auto_lock_interval", 1.0),
            ("auto_turn_interval", 2.0),
        ] {
            let [lo, hi] = self.scheduler.script.interval_range(name, default)?;
            self.random ^= self.random << 13;
            self.random ^= self.random >> 7;
            self.random ^= self.random << 17;
            result.insert(
                name.into(),
                lo + (hi - lo) * (self.random >> 11) as f64 / ((1u64 << 53) as f64),
            );
        }
        Ok(result)
    }
    pub fn execute(
        &mut self,
        native: &mut NativeSession,
        recognizer: &mut CombatRecognizer,
        allow_audio: bool,
        mut publish: impl FnMut(CombatProgress),
    ) -> Result<()> {
        ensure!(!self.parented, "应用内战斗须使用执行到结算的入口");
        let started = Instant::now();
        let result = self
            .execute_inner(
                native,
                recognizer,
                allow_audio,
                &mut |progress| {
                    publish(progress);
                    Ok(())
                },
                false,
                &mut |_, _, _, _| Ok(CombatFrameGate::Proceed),
            )
            .map(|_| ());
        self.elapsed += started.elapsed().as_secs_f64();
        let released = native.release_inputs();
        self.progress.inputs_released = released.is_ok();
        self.scheduler.pause();
        self.sequence = None;
        if let Err(error) = result.and(released) {
            self.control.finish();
            self.progress.state = RunState::Failed;
            self.message = format!("战斗失败：{error:#}");
            if let Some(log) = &self.performance {
                log.trace().event(
                    "run_state",
                    json!({"progress": self.progress, "message": self.message}),
                );
            }
            publish(self.progress.clone());
            return Err(error);
        }
        match self.control.requested() {
            Some(Interrupt::Pause) if self.control.acknowledge_pause() => {
                self.progress.state = RunState::Paused;
                self.message = "战斗已暂停，输入已释放；继续后重新识别".into();
            }
            _ => {
                self.control.finish();
                self.progress.state = RunState::Stopped;
                self.message = "战斗已停止，输入已释放".into();
            }
        }
        if let Some(log) = &self.performance {
            log.trace().event(
                "run_state",
                json!({"progress": self.progress, "message": self.message}),
            );
        }
        publish(self.progress.clone());
        Ok(())
    }
    pub fn execute_segment_with_budget(
        &mut self,
        native: &mut NativeSession,
        recognizer: &mut CombatRecognizer,
        allow_audio: bool,
        budget: Duration,
        publish: impl FnMut(CombatProgress) -> Result<()>,
    ) -> Result<CombatSegmentEnd> {
        use std::sync::atomic::{AtomicBool, Ordering};
        let deadline = self.segment_deadline(budget);
        self.parent_deadline = Some(deadline);
        let finished = AtomicBool::new(false);
        let raw = recognizer.cancellation_handle();
        let control = self.control.clone();
        let result = std::thread::scope(|scope| {
            scope.spawn(|| {
                while !finished.load(Ordering::Acquire) {
                    if (control.requested().is_some() || Instant::now() >= deadline)
                        && let Some(raw) = raw.lock().expect("应用内战斗推理取消锁失效").clone()
                    {
                        let _ = raw.cancel();
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            });
            let result = self.execute_segment(native, recognizer, allow_audio, publish);
            finished.store(true, Ordering::Release);
            result
        });
        self.parent_deadline = None;
        result
    }
    pub fn execute_segment(
        &mut self,
        native: &mut NativeSession,
        recognizer: &mut CombatRecognizer,
        allow_audio: bool,
        publish: impl FnMut(CombatProgress) -> Result<()>,
    ) -> Result<CombatSegmentEnd> {
        match self.execute_segment_guarded(
            native,
            recognizer,
            allow_audio,
            CombatResultPolicy::NormalBattle,
            &mut |_, _, _, _| Ok(CombatFrameGate::Proceed),
            publish,
        )? {
            GuardedCombatSegmentEnd::Result(result) => Ok(CombatSegmentEnd::Result(result)),
            GuardedCombatSegmentEnd::Interrupted(interrupt) => {
                Ok(CombatSegmentEnd::Interrupted(interrupt))
            }
            GuardedCombatSegmentEnd::Yielded => anyhow::bail!("普通战斗出现未提供的外部交接"),
        }
    }
    /// 外部守卫负责自己的页面身份及稳定新帧证明；本方法负责调度、释放与父预算。
    pub fn execute_segment_guarded_with_budget(
        &mut self,
        native: &mut NativeSession,
        recognizer: &mut CombatRecognizer,
        allow_audio: bool,
        options: CombatGuardOptions,
        mut guard: impl FnMut(
            &NativeSession,
            &CombatRecognizer,
            &CombatFrame,
            Duration,
        ) -> Result<CombatFrameGate>,
        publish: impl FnMut(CombatProgress) -> Result<()>,
    ) -> Result<GuardedCombatSegmentEnd> {
        use std::sync::atomic::{AtomicBool, Ordering};
        let deadline = self.segment_deadline(options.budget);
        self.parent_deadline = Some(deadline);
        let finished = AtomicBool::new(false);
        let raw = recognizer.cancellation_handle();
        let control = self.control.clone();
        let result = std::thread::scope(|scope| {
            scope.spawn(|| {
                while !finished.load(Ordering::Acquire) {
                    if (control.requested().is_some() || Instant::now() >= deadline)
                        && let Some(raw) = raw.lock().expect("应用内战斗推理取消锁失效").clone()
                    {
                        let _ = raw.cancel();
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            });
            let result = self.execute_segment_guarded(
                native,
                recognizer,
                allow_audio,
                options.result_policy,
                &mut guard,
                publish,
            );
            finished.store(true, Ordering::Release);
            result
        });
        self.parent_deadline = None;
        result
    }
    fn segment_deadline(&self, budget: Duration) -> Instant {
        let config_remaining = Duration::from_secs_f64(
            (f64::from(self.progress.config.timeout_ms) / 1000.0 - self.elapsed).max(0.0),
        );
        Instant::now() + budget.min(config_remaining)
    }
    fn execute_segment_guarded(
        &mut self,
        native: &mut NativeSession,
        recognizer: &mut CombatRecognizer,
        allow_audio: bool,
        policy: CombatResultPolicy,
        guard: &mut impl FnMut(
            &NativeSession,
            &CombatRecognizer,
            &CombatFrame,
            Duration,
        ) -> Result<CombatFrameGate>,
        mut publish: impl FnMut(CombatProgress) -> Result<()>,
    ) -> Result<GuardedCombatSegmentEnd> {
        ensure!(
            policy == CombatResultPolicy::NormalBattle || self.patrol_ui_timeout.is_none(),
            "世界巡逻与外部页面边界不能混用"
        );
        ensure!(
            self.parented && !self.progress.state.is_terminal() && self.progress.inputs_released,
            "战斗交接缺少可继续状态或输入释放证明"
        );
        self.scheduler.resume();
        self.sequence = None;
        self.end_tracker.reset_confirmation();
        self.progress.state = RunState::Running;
        let started = Instant::now();
        let initial_elapsed = self.elapsed;
        let result = self.execute_inner(
            native,
            recognizer,
            allow_audio,
            &mut publish,
            policy == CombatResultPolicy::NormalBattle,
            guard,
        );
        self.elapsed += started.elapsed().as_secs_f64();
        let released = native.release_inputs();
        self.progress.inputs_released = released.is_ok();
        self.progress.in_battle = false;
        self.scheduler.pause();
        self.sequence = None;
        let result = result.and_then(|result| released.map(|_| result));
        let mut end = match result {
            Err(error) => {
                self.progress.state = RunState::Failed;
                self.message = format!("应用内战斗失败：{error:#}");
                // 错误不能被一次失败的状态发布吞掉，外层仍负责持久化并收尾。
                let _ = publish(self.progress.clone());
                return Err(error);
            }
            Ok(result) => match self.control.requested() {
                Some(interrupt) => {
                    self.progress.state = match interrupt {
                        Interrupt::Pause => RunState::Paused,
                        Interrupt::Stop => RunState::Stopped,
                    };
                    self.message = "应用内战斗已中断，输入已释放".into();
                    GuardedCombatSegmentEnd::Interrupted(interrupt)
                }
                None => {
                    let result = result.context("应用内战斗没有结算或中断证据")?;
                    match result {
                        InnerEnd::Normal(result) => {
                            self.progress.state = match result {
                                NormalBattleResult::Completed => RunState::Succeeded,
                                NormalBattleResult::Retreat => RunState::Failed,
                            };
                            self.message = "战斗结算已确认，输入已释放，交回应用流程".into();
                            GuardedCombatSegmentEnd::Result(result)
                        }
                        InnerEnd::Yielded => {
                            self.progress.state = RunState::Running;
                            self.message = "战斗输入已释放，交回父流程核对局内页面".into();
                            GuardedCombatSegmentEnd::Yielded
                        }
                    }
                }
            },
        };
        let saved = publish(self.progress.clone());
        self.elapsed = initial_elapsed + started.elapsed().as_secs_f64();
        if let Err(error) = saved {
            self.progress.state = RunState::Failed;
            self.message = format!("战斗交接状态保存失败：{error:#}");
            return Err(error);
        }
        // 发布检查点期间也可能收到暂停或停止；外层仍须在实际交接前检查请求。
        if let Some(interrupt) = self.control.requested() {
            self.progress.state = match interrupt {
                Interrupt::Pause => RunState::Paused,
                Interrupt::Stop => RunState::Stopped,
            };
            self.message = "应用内战斗已中断，输入已释放".into();
            end = GuardedCombatSegmentEnd::Interrupted(interrupt);
        }
        if self.control.requested().is_none()
            && (self
                .parent_deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
                || self.elapsed * 1000.0 >= f64::from(self.progress.config.timeout_ms))
        {
            self.progress.state = RunState::Failed;
            self.message = "战斗交接保存期间总运行预算已耗尽".into();
            anyhow::bail!("战斗交接保存期间总运行预算已耗尽");
        }
        Ok(end)
    }
    fn execute_inner(
        &mut self,
        native: &mut NativeSession,
        recognizer: &mut CombatRecognizer,
        allow_audio: bool,
        publish: &mut impl FnMut(CombatProgress) -> Result<()>,
        check_end: bool,
        guard: &mut impl FnMut(
            &NativeSession,
            &CombatRecognizer,
            &CombatFrame,
            Duration,
        ) -> Result<CombatFrameGate>,
    ) -> Result<Option<InnerEnd>> {
        if !self.parented && !check_end && self.patrol_ui_timeout.is_none() {
            recognizer.reset();
            let worker = crate::combat_worker::CombatWorker::new(
                native.combat_reader(recognizer)?,
                recognizer.clone(),
            )?;
            self.execute_with_worker(native, recognizer, allow_audio, publish, guard, worker)
        } else {
            self.execute_loop(
                native,
                recognizer,
                publish,
                guard,
                CombatLoopOptions {
                    allow_audio,
                    check_end,
                    worker: None,
                },
            )
        }
    }
    fn execute_with_worker(
        &mut self,
        native: &mut NativeSession,
        recognizer: &mut CombatRecognizer,
        allow_audio: bool,
        publish: &mut impl FnMut(CombatProgress) -> Result<()>,
        guard: &mut impl FnMut(
            &NativeSession,
            &CombatRecognizer,
            &CombatFrame,
            Duration,
        ) -> Result<CombatFrameGate>,
        mut worker: crate::combat_worker::CombatWorker,
    ) -> Result<Option<InnerEnd>> {
        worker.set_trace(self.performance.as_ref().map(|log| log.trace()));
        let result = self.execute_loop(
            native,
            recognizer,
            publish,
            guard,
            CombatLoopOptions {
                allow_audio,
                check_end: false,
                worker: Some(&mut worker),
            },
        );
        // 输入先释放，再等待在途识别；慢推理不能拖长持有按键。
        let released = native.release_inputs();
        let capture = native.finish_combat_capture();
        let state = worker.finish();
        released?;
        capture?;
        *recognizer = state?;
        result
    }
    fn execute_loop(
        &mut self,
        native: &mut NativeSession,
        recognizer: &mut CombatRecognizer,
        publish: &mut impl FnMut(CombatProgress) -> Result<()>,
        guard: &mut impl FnMut(
            &NativeSession,
            &CombatRecognizer,
            &CombatFrame,
            Duration,
        ) -> Result<CombatFrameGate>,
        options: CombatLoopOptions<'_>,
    ) -> Result<Option<InnerEnd>> {
        let CombatLoopOptions {
            allow_audio,
            check_end,
            mut worker,
        } = options;
        let trace = self.performance.as_ref().map(|log| log.trace());
        if let Some(trace) = &trace {
            trace.event(
                "loop_start",
                json!({"asynchronous": worker.is_some(), "elapsed_seconds": self.elapsed}),
            );
        }
        native.check_combat_input_mode(self.progress.config.mode)?;
        recognizer.reset();
        self.progress.team.clear();
        for prefix in [
            "前台-",
            "后台-1-",
            "后台-2-",
            "快速支援-",
            "连携技-1-",
            "连携技-2-",
            "切换角色-",
        ] {
            let names: Vec<_> = recognizer
                .bundle
                .spec
                .agents
                .iter()
                .map(|agent| format!("{prefix}{}", agent.name))
                .chain((prefix.starts_with("连携技-")).then(|| format!("{prefix}邦布")))
                .collect();
            self.scheduler.states.add_mutex(&names);
            let kinds: std::collections::BTreeSet<_> = recognizer
                .bundle
                .spec
                .agents
                .iter()
                .map(|agent| format!("{prefix}{}", agent.kind))
                .collect();
            self.scheduler
                .states
                .add_mutex(&kinds.into_iter().collect::<Vec<_>>());
        }
        for agent in &recognizer.bundle.spec.agents {
            let name = &agent.name;
            self.scheduler.states.add_exclusions(
                format!("前台-{name}"),
                [
                    format!("后台-{name}"),
                    format!("后台-1-{name}"),
                    format!("后台-2-{name}"),
                ],
            );
            self.scheduler
                .states
                .add_exclusions(format!("后台-{name}"), [format!("前台-{name}")]);
            for position in 1..=2 {
                self.scheduler.states.add_exclusions(
                    format!("后台-{position}-{name}"),
                    [
                        format!("前台-{name}"),
                        format!("后台-{}-{name}", 3 - position),
                    ],
                );
            }
        }
        let recognition_dodge_only = self.scheduler.script.states.iter().all(|name| {
            name.starts_with("闪避识别-")
                || name.starts_with("自定义-")
                || name.starts_with("按键-")
                || name == "按键可用-普通攻击"
        });
        let flash_enabled = self
            .scheduler
            .script
            .states
            .iter()
            .any(|name| name == "闪避识别-红光" || name == "闪避识别-黄光");
        let cancel = self.control.cancellation();
        let delay = Instant::now();
        while delay.elapsed() < Duration::from_millis(self.progress.config.start_delay_ms as u64) {
            if self.control.requested().is_some() {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut audio = if self.progress.config.audio {
            ensure!(allow_audio, "离线回放不能启动真实输出回录");
            let template = od_core::audio_dodge::read_wav(&std::fs::read(
                recognizer.bundle.root.join("audio/template_1.wav"),
            )?)?;
            Some((
                od_platform::audio::AudioLoopback::start()?,
                od_core::audio_dodge::AudioDodge::new(
                    &template,
                    self.progress.config.audio_threshold_milli as f64 / 1000.0,
                )?,
            ))
        } else {
            None
        };
        let started = Instant::now();
        let mut ledger = InputLedger::new(self.progress.config.mode.is_gamepad());
        let mut queue = VecDeque::new();
        let mut until = None;
        let mut next_frame = 0.0;
        let mut recognition_seconds = 0.0_f64;
        let mut capture_seconds = 0.0_f64;
        let mut last_fresh = None;
        let mut refresh_pending = false;
        let mut last_publish = 0.0;
        let mut last_lock = 0.0;
        let mut last_turn = 0.0;
        let mut last_audio = -1.0;
        let mut last_ultimate = -2.0;
        let mut flash_dispatch_until = 0.0;
        let mut pending_states = Vec::new();
        let mut observation = CombatObservation::default();
        let mut team_ready = false;
        let mut patrol_visible_frames = 0_u8;
        let mut patrol_last_hud = Instant::now();
        let mut latest_source = None;
        let mut pending_input: Option<PendingInput> = None;
        let mut capture_origin = None;
        let mut release_deadline = None;
        let mut loop_performance = trace
            .as_ref()
            .map(crate::combat_performance::LoopPerformance::new);
        self.progress.inputs_released = false;
        publish(self.progress.clone())?;
        loop {
            if let Some(performance) = &mut loop_performance {
                performance.iteration();
            }
            let _loop_span = trace
                .as_ref()
                .map(|trace| trace.span("slow_loop", json!({}), 50_000));
            if self.control.requested().is_some() {
                return Ok(None);
            }
            ensure!(
                self.parent_deadline
                    .is_none_or(|deadline| Instant::now() < deadline),
                "应用总运行预算已耗尽"
            );
            let mut now = self.elapsed + started.elapsed().as_secs_f64();
            if let Some(input) = &mut pending_input
                && input.poll(native)?
            {
                if input.refresh_agents {
                    recognizer.refresh_agents();
                    if let Some(background) = worker.as_mut() {
                        background.refresh_agents();
                    }
                    team_ready = false;
                }
                pending_input = None;
            }
            ensure!(
                now * 1000.0 < self.progress.config.timeout_ms as f64,
                "战斗运行达到超时上限"
            );
            // 已排入的定时松键优先于可能阻塞的截图和识别。
            let release_due = matches!(queue.front(), Some(Step::Delay(_)))
                && matches!(
                    queue.get(1),
                    Some(
                        Step::Button(_, false) | Step::NativeInput(_, _, false) | Step::ReleaseTurn
                    )
                )
                || (matches!(
                    queue.front(),
                    Some(
                        Step::Button(_, false) | Step::NativeInput(_, _, false) | Step::ReleaseTurn
                    )
                ) && until.is_none_or(|deadline| {
                    let blocking = if worker.is_some() {
                        (capture_seconds + 0.005).max(0.02)
                    } else {
                        recognition_seconds.max(0.005)
                    };
                    deadline - now <= blocking
                }));
            let asynchronous = worker.is_some();
            let mut completed = None;
            if let Some(background) = worker.as_mut() {
                let mut early = Vec::new();
                completed = background.poll(&mut early)?;
                for flash in early {
                    if let Some(trace) = &trace {
                        trace.event("flash_consumed", json!({
                            "frame_id": trace.stamp(flash.captured),
                            "frame_age_us": flash.captured.elapsed().as_micros() as u64,
                            "accepted_age": flash.captured.elapsed().as_millis() <= u128::from(self.progress.config.max_frame_age_ms),
                        }));
                    }
                    if flash.captured.elapsed().as_millis()
                        <= u128::from(self.progress.config.max_frame_age_ms)
                    {
                        latest_source = Some(flash.captured);
                        pending_states.extend(flash.states);
                        last_fresh = Some(flash.at);
                        refresh_pending = false;
                        self.progress.in_battle = true;
                        flash_dispatch_until =
                            self.elapsed + started.elapsed().as_secs_f64() + 0.02;
                    }
                }
                let capture_poll_started = Instant::now();
                if let Some(frame) = native.poll_combat_frame(&cancel)? {
                    let (frame_at, epoch) =
                        capture_origin.take().context("战斗截图缺少提交代际")?;
                    capture_seconds = frame.recognition_image().prepare_us as f64 / 1_000_000.0;
                    if let Some(trace) = &trace {
                        trace.event(
                            "capture",
                            json!({
                                "frame_id": trace.stamp(frame.captured),
                                "capture_us": frame.recognition_image().prepare_us,
                                "main_thread_us": capture_poll_started.elapsed().as_micros() as u64,
                                "timing": frame.capture_timing, "asynchronous": true,
                                "accepted_epoch": epoch == background.epoch(),
                            }),
                        );
                    }
                    if epoch == background.epoch() {
                        background.capture(frame, frame_at);
                    }
                }
                // 先消费已完成结果，避免新截图再次延后闪光事件的调度。
                let input_ready = pending_input.is_some()
                    || (until.is_none()
                        && matches!(
                            queue.front(),
                            Some(
                                Step::Button(_, _)
                                    | Step::ButtonConfirmed(_, _)
                                    | Step::NativeInput(_, _, _)
                                    | Step::Move(_)
                                    | Step::ClearInputs
                                    | Step::ReleaseTurn
                            )
                        ));
                let prioritize_flash = now < flash_dispatch_until && until.is_none();
                let may_capture = !release_due && !input_ready && !prioritize_flash;
                if completed.is_none()
                    && !native.combat_capture_pending()
                    && now >= next_frame
                    && let Some(performance) = &mut loop_performance
                {
                    if release_due {
                        performance.defer("timed_release");
                    } else if input_ready {
                        performance.defer("input_ready");
                    } else if prioritize_flash {
                        performance.defer("flash_dispatch");
                    }
                }
                if completed.is_none()
                    && now >= next_frame
                    && !native.combat_capture_pending()
                    && may_capture
                {
                    if !native.start_combat_frame(&cancel)? {
                        return Ok(None);
                    }
                    capture_origin = Some((now, background.epoch()));
                    next_frame = now + self.progress.config.frame_interval_ms as f64 / 1000.0;
                }
                if background.needs_submission() {
                    let mut intervals = self.intervals()?;
                    intervals.insert("flash_enabled".into(), f64::from(flash_enabled));
                    background.submit(intervals, recognition_dodge_only)?;
                }
                now = self.elapsed + started.elapsed().as_secs_f64();
                if self.control.requested().is_some() {
                    return Ok(None);
                }
                ensure!(
                    self.parent_deadline
                        .is_none_or(|deadline| Instant::now() < deadline),
                    "应用总运行预算已耗尽"
                );
                ensure!(
                    now * 1000.0 < self.progress.config.timeout_ms as f64,
                    "战斗运行达到超时上限"
                );
            }
            if completed.is_some() || (!asynchronous && now >= next_frame && !release_due) {
                let recognition_started = Instant::now();
                let (mut frame, frame_at, prefetched) = if let Some(result) = completed {
                    (result.frame, result.at, Some(result.observation))
                } else {
                    let Some(frame) = native.combat_frame(&cancel)? else {
                        return Ok(None);
                    };
                    (frame, now, None)
                };
                latest_source = Some(frame.captured);
                self.progress.frames += 1;
                if self.control.requested().is_some() {
                    return Ok(None);
                }
                ensure!(
                    self.parent_deadline
                        .is_none_or(|deadline| Instant::now() < deadline),
                    "应用总运行预算已耗尽"
                );
                ensure!(
                    (self.elapsed + started.elapsed().as_secs_f64()) * 1000.0
                        < f64::from(self.progress.config.timeout_ms),
                    "战斗运行达到超时上限"
                );
                let remaining = self
                    .parent_deadline
                    .map(|deadline| deadline.saturating_duration_since(Instant::now()))
                    .unwrap_or(Duration::from_millis(u64::from(
                        self.progress.config.timeout_ms,
                    )));
                let gate = guard(native, recognizer, &frame, remaining)?;
                if self.control.requested().is_some() {
                    return Ok(None);
                }
                ensure!(
                    self.parent_deadline
                        .is_none_or(|deadline| Instant::now() < deadline),
                    "应用总运行预算已耗尽"
                );
                ensure!(
                    (self.elapsed + started.elapsed().as_secs_f64()) * 1000.0
                        < f64::from(self.progress.config.timeout_ms),
                    "战斗运行达到超时上限"
                );
                if gate != CombatFrameGate::Proceed {
                    native.release_inputs()?;
                    ledger.clear();
                    pending_input = None;
                    queue.clear();
                    until = None;
                    if gate == CombatFrameGate::Refresh {
                        self.refresh_observation(&mut pending_states)?;
                    }
                    pending_states.clear();
                    if let Some((capture, detector)) = &mut audio {
                        let _ = capture.drain()?;
                        detector.reset();
                    }
                    if gate == CombatFrameGate::Refresh {
                        refresh_pending = true;
                    } else {
                        self.scheduler.pause();
                        self.scheduler.resume();
                        refresh_pending = false;
                    }
                    self.sequence = None;
                    team_ready = false;
                    last_fresh = None;
                    self.progress.in_battle = false;
                    self.progress.inputs_released = true;
                    self.progress.frame_age_ms = frame.age_ms();
                    if gate == CombatFrameGate::Yield {
                        ensure!(frame.age_ms() <= 1500, "战斗交接守卫返回过期截图");
                        return Ok(Some(InnerEnd::Yielded));
                    }
                    recognition_seconds = recognition_started.elapsed().as_secs_f64();
                    now = self.elapsed + started.elapsed().as_secs_f64();
                    next_frame = now + self.progress.config.frame_interval_ms as f64 / 1000.0;
                    if now - last_publish >= 0.1 {
                        publish(self.progress.clone())?;
                        last_publish = now;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }
                self.progress.inputs_released = false;
                if self.patrol_ui_timeout.is_some() {
                    if frame.age_ms() <= self.progress.config.max_frame_age_ms
                        && native.patrol_world_in_combat(&frame)?
                    {
                        native.release_inputs()?;
                        ledger.clear();
                        pending_input = None;
                        queue.clear();
                        until = None;
                        self.scheduler.pause();
                        self.scheduler.resume();
                        self.sequence = None;
                        patrol_visible_frames += 1;
                        if patrol_visible_frames >= 2 {
                            return Ok(Some(InnerEnd::Normal(NormalBattleResult::Completed)));
                        }
                        next_frame = now + 0.2;
                        continue;
                    }
                    patrol_visible_frames = 0;
                    recognizer.align_patrol_frame(native, &mut frame, &cancel)?;
                }
                let mut intervals = self.intervals()?;
                intervals.insert("flash_enabled".into(), f64::from(flash_enabled));
                let fresh = if let Some(observation) = prefetched {
                    observation
                } else {
                    recognizer.observe(
                        native,
                        &frame,
                        frame_at,
                        &intervals,
                        recognition_dodge_only,
                        &cancel,
                    )?
                };
                if self.control.requested().is_some() {
                    return Ok(None);
                }
                ensure!(
                    self.parent_deadline
                        .is_none_or(|deadline| Instant::now() < deadline),
                    "应用总运行预算已耗尽"
                );
                let has_chain = fresh.states.iter().any(|state| {
                    !state.clear
                        && (state.name == "连携技-准备"
                            || state.name.starts_with("连携技-1-")
                            || state.name.starts_with("连携技-2-"))
                });
                if let Some(seconds) = self.patrol_ui_timeout {
                    if fresh.in_battle || has_chain {
                        patrol_last_hud = Instant::now();
                    }
                    ensure!(
                        patrol_last_hud.elapsed() < Duration::from_secs(u64::from(seconds)),
                        "锄大地战斗界面消失，已停止输入"
                    );
                }
                let end = if check_end && !fresh.in_battle && !has_chain {
                    // 结算识别可能耗时，先释放战斗输入并丢弃尚未执行的动作。
                    native.release_inputs()?;
                    ledger.clear();
                    pending_input = None;
                    queue.clear();
                    until = None;
                    pending_states.clear();
                    self.scheduler.pause();
                    self.scheduler.resume();
                    self.sequence = None;
                    recognizer.normal_battle_result(native, &frame, &cancel)?
                } else {
                    None
                };
                if self.control.requested().is_some() {
                    return Ok(None);
                }
                if check_end {
                    ensure!(
                        self.parent_deadline
                            .is_none_or(|deadline| Instant::now() < deadline),
                        "应用总运行预算已耗尽"
                    );
                    ensure!(
                        (self.elapsed + started.elapsed().as_secs_f64()) * 1000.0
                            < f64::from(self.progress.config.timeout_ms),
                        "战斗运行达到超时上限"
                    );
                }
                self.progress.frame_age_ms = frame.age_ms();
                if check_end
                    && let Some(end) = self.end_tracker.observe(
                        fresh.in_battle,
                        end,
                        self.progress.frame_age_ms <= self.progress.config.max_frame_age_ms,
                    )
                {
                    return Ok(Some(InnerEnd::Normal(end)));
                }
                now = self.elapsed + started.elapsed().as_secs_f64();
                recognition_seconds = recognition_started.elapsed().as_secs_f64();
                if !asynchronous {
                    next_frame = frame_at + self.progress.config.frame_interval_ms as f64 / 1000.0;
                }
                if frame.age_ms() > self.progress.config.max_frame_age_ms {
                    if let Some(trace) = &trace {
                        trace.event("stale_frame", json!({"frame_id": trace.stamp(frame.captured), "frame_age_us": frame.captured.elapsed().as_micros() as u64}));
                    }
                    self.progress.stale_frames += 1;
                    native.release_inputs()?;
                    ledger.clear();
                    pending_input = None;
                    queue.clear();
                    until = None;
                    self.refresh_observation(&mut pending_states)?;
                    refresh_pending = true;
                    self.sequence = None;
                    team_ready = false;
                    last_fresh = None;
                    self.progress.in_battle = false;
                } else {
                    if self.progress.config.auto_ultimate
                        && now - last_ultimate >= 2.0
                        && fresh
                            .states
                            .iter()
                            .any(|s| s.name == "按键可用-终结技" && !s.clear)
                    {
                        pending_states.push(StateUpdate {
                            name: "自定义-终结技被强制释放".into(),
                            at: frame_at,
                            value: None,
                            add: None,
                            time_add: None,
                            clear: false,
                        });
                        last_ultimate = now;
                    }
                    pending_states.extend(fresh.states.iter().cloned());
                    self.progress.in_battle = fresh.in_battle;
                    if !fresh.team.is_empty() {
                        team_ready = self.observe_team(&fresh.team);
                    }
                    last_fresh = (fresh.in_battle
                        || fresh.states.iter().any(|s| {
                            (s.name == "连携技-准备"
                                || s.name.starts_with("连携技-1-")
                                || s.name.starts_with("连携技-2-"))
                                && !s.clear
                        }))
                    .then_some(frame_at);
                    if last_fresh.is_some() {
                        refresh_pending = false;
                    }
                    observation = fresh;
                }
            }
            if now - last_audio >= 0.02
                && let Some((capture, detector)) = &mut audio
            {
                let _span = trace
                    .as_ref()
                    .map(|trace| trace.span("audio", json!({}), 0));
                last_audio = now;
                let (samples, dropped) = capture.drain()?;
                if dropped > 0 {
                    detector.reset();
                }
                detector.append(&samples)?;
                let (correlation, hit) = detector.check(now)?;
                self.progress.audio_correlation = Some(correlation);
                if hit && self.progress.in_battle {
                    pending_states.push(StateUpdate {
                        name: "闪避识别-声音".into(),
                        at: now,
                        value: None,
                        add: None,
                        time_add: None,
                        clear: false,
                    });
                }
            }
            let fresh = last_fresh.is_some_and(|at| {
                (now - at) * 1000.0 <= self.progress.config.max_frame_age_ms as f64
            });
            if !fresh {
                if self.sequence.is_some() || !queue.is_empty() || !ledger.is_empty() {
                    native.release_inputs()?;
                    ledger.clear();
                    pending_input = None;
                    queue.clear();
                    until = None;
                }
                if !refresh_pending {
                    self.refresh_observation(&mut pending_states)?;
                    refresh_pending = true;
                }
                self.sequence = None;
                team_ready = false;
                pending_states.clear();
            } else {
                let scheduler_started = Instant::now();
                // 原生确认后的逻辑状态必须先提交，避免实际已切人却被抢占丢掉队伍更新。
                let decision = if matches!(queue.front(), Some(Step::ButtonConfirmed(_, _)))
                    && pending_input.is_none()
                {
                    Decision::Idle
                } else {
                    self.scheduler
                        .tick(now, &std::mem::take(&mut pending_states))?
                };
                if let Some(trace) = &trace {
                    let decision_name = match &decision {
                        Decision::Start(_) => "start",
                        Decision::Replace(_) => "replace",
                        Decision::Interrupt => "interrupt",
                        Decision::Idle => "idle",
                    };
                    if decision_name != "idle" || scheduler_started.elapsed().as_millis() >= 5 {
                        trace.event("decision", json!({
                            "decision": decision_name,
                            "scheduler_us": scheduler_started.elapsed().as_micros() as u64,
                            "latest_source_frame_id": latest_source.map(|at| trace.stamp(at)),
                            "source_age_us": latest_source.map(|at| at.elapsed().as_micros() as u64),
                        }));
                    }
                }
                let replacing = matches!(decision, Decision::Replace(_));
                match decision {
                    Decision::Start(execution) | Decision::Replace(execution) => {
                        if replacing || pending_input.is_some() {
                            if replacing {
                                self.progress.preemptions += 1;
                            }
                            native.release_inputs()?;
                            ledger.clear();
                            pending_input = None;
                        }
                        queue.clear();
                        until = None;
                        self.progress.handler = if execution.label.is_empty() {
                            execution.source
                        } else {
                            execution.label
                        };
                        self.sequence = Some(execution.generation);
                        queue.extend(execution.actions.into_iter().map(Step::Action));
                    }
                    Decision::Interrupt => {
                        native.release_inputs()?;
                        ledger.clear();
                        pending_input = None;
                        queue.clear();
                        until = None;
                        self.sequence = None;
                        self.progress.preemptions += 1;
                    }
                    Decision::Idle => {}
                }
                let elapsed_deadline = until.filter(|deadline| now >= *deadline);
                if elapsed_deadline.is_some() {
                    release_deadline = elapsed_deadline;
                    until = None;
                }
                if queue.is_empty() && until.is_none() && pending_input.is_none() {
                    if let Some(generation) = self.sequence.take() {
                        self.scheduler.complete(generation)?;
                    }
                    if self.progress.in_battle {
                        let intervals = self.intervals()?;
                        if intervals["auto_lock_interval"] > 0.0
                            && now - last_lock >= intervals["auto_lock_interval"]
                        {
                            queue.push_back(Step::Action(Action::Button {
                                name: "锁定敌人".into(),
                                way: ButtonWay::Tap,
                                press: None,
                            }));
                            last_lock = now;
                        }
                        if intervals["auto_turn_interval"] > 0.0
                            && now - last_turn >= intervals["auto_turn_interval"]
                        {
                            queue.push_back(Step::Action(Action::Turn { distance: 100.0 }));
                            last_turn = now;
                        }
                    }
                }
                if pending_input.is_none()
                    && until.is_none()
                    && let Some(step) = queue.pop_front()
                {
                    let confirmed = matches!(&step, Step::ButtonConfirmed(_, _));
                    match step {
                        Step::NativeInput(name, action, pressed) => {
                            let input_started = Instant::now();
                            let input_at = self.elapsed + started.elapsed().as_secs_f64();
                            let Some(job) = native.start_combat_input(action, pressed, &cancel)?
                            else {
                                return Ok(None);
                            };
                            pending_input = Some(PendingInput {
                                job,
                                started: input_started,
                                trace: trace.clone(),
                                finished: false,
                                refresh_agents: pressed
                                    && (name.starts_with("切换角色-")
                                        || name.starts_with("连携技-")),
                                data: json!({
                                    "button": name, "pressed": pressed, "handler": self.progress.handler,
                                    "started_us": trace.as_ref().map(|trace| trace.stamp(input_started)),
                                    "latest_source_frame_id": trace.as_ref().and_then(|trace| latest_source.map(|at| trace.stamp(at))),
                                    "source_age_us": latest_source.map(|at| input_started.saturating_duration_since(at).as_micros() as u64),
                                    "release_start_lateness_us": if pressed { None } else { release_deadline.map(|deadline: f64| ((input_at - deadline).max(0.0) * 1_000_000.0) as u64) },
                                }),
                            });
                        }
                        Step::Move(dx) => {
                            let input_started = Instant::now();
                            let Some(job) = native.start_combat_turn(dx, &cancel)? else {
                                return Ok(None);
                            };
                            pending_input = Some(PendingInput {
                                job,
                                started: input_started,
                                trace: trace.clone(),
                                finished: false,
                                refresh_agents: false,
                                data: json!({"button":"转向", "pressed":true, "started_us": trace.as_ref().map(|trace| trace.stamp(input_started))}),
                            });
                        }
                        Step::Delay(seconds) => {
                            until = Some(now + seconds);
                            release_deadline = None;
                        }
                        Step::ClearInputs => {
                            native.release_inputs()?;
                            ledger.clear();
                            pending_input = None;
                        }
                        Step::ReleaseTurn => {
                            for (action, down) in
                                ledger.set("__turn", Vec::new(), false).into_iter().rev()
                            {
                                if asynchronous {
                                    queue.push_front(Step::NativeInput(
                                        "__turn".into(),
                                        action,
                                        down,
                                    ));
                                } else if !native.combat_input(action, down, &cancel)? {
                                    return Ok(None);
                                }
                            }
                        }
                        Step::Button(name, pressed) | Step::ButtonConfirmed(name, pressed) => {
                            if asynchronous && !confirmed {
                                let actions = ledger.set(
                                    &name,
                                    self.progress
                                        .config
                                        .binding(&name)
                                        .map_err(anyhow::Error::msg)?,
                                    pressed,
                                );
                                queue.push_front(Step::ButtonConfirmed(name.clone(), pressed));
                                for (action, down) in actions.into_iter().rev() {
                                    queue.push_front(Step::NativeInput(name.clone(), action, down));
                                }
                            } else {
                                if !confirmed {
                                    for (action, down) in ledger.set(
                                        &name,
                                        self.progress
                                            .config
                                            .binding(&name)
                                            .map_err(anyhow::Error::msg)?,
                                        pressed,
                                    ) {
                                        let input_started = Instant::now();
                                        let input_at =
                                            self.elapsed + started.elapsed().as_secs_f64();
                                        let result = native.combat_input(action, down, &cancel);
                                        if let Some(trace) = &trace {
                                            trace.event("input", json!({
                                        "button": name, "pressed": down, "handler": self.progress.handler,
                                        "started_us": trace.stamp(input_started),
                                        "duration_us": input_started.elapsed().as_micros() as u64,
                                        "sent": result.as_ref().is_ok_and(|sent| *sent),
                                        "latest_source_frame_id": latest_source.map(|at| trace.stamp(at)),
                                        "source_age_us": latest_source.map(|at| input_started.saturating_duration_since(at).as_micros() as u64),
                                        "release_start_lateness_us": if down { None } else { elapsed_deadline.map(|deadline: f64| ((input_at - deadline).max(0.0) * 1_000_000.0) as u64) },
                                    }));
                                        }
                                        if !result? {
                                            return Ok(None);
                                        }
                                    }
                                }
                                pending_states.push(StateUpdate {
                                    name: format!(
                                        "按键-{name}-{}",
                                        if pressed { "按下" } else { "松开" }
                                    ),
                                    at: self.elapsed + started.elapsed().as_secs_f64(),
                                    value: None,
                                    add: None,
                                    time_add: None,
                                    clear: false,
                                });
                                if pressed {
                                    pending_states.push(StateUpdate {
                                        name: format!("按键-{name}"),
                                        at: now,
                                        value: None,
                                        add: None,
                                        time_add: None,
                                        clear: false,
                                    });
                                    if name.starts_with("切换角色-") || name.starts_with("连携技-")
                                    {
                                        recognizer.refresh_agents();
                                        if let Some(background) = worker.as_mut() {
                                            background.refresh_agents();
                                        }
                                    }
                                    let team = &mut self.progress.team;
                                    if team.len() > 1 {
                                        if name == "切换角色-下一个" {
                                            team.rotate_left(1);
                                        } else if name == "切换角色-上一个" {
                                            team.rotate_right(1);
                                        } else if name == "连携技-左" || name == "连携技-右"
                                        {
                                            let index = usize::from(name == "连携技-右");
                                            if let Some(Some(target)) = observation.chain.get(index)
                                                && let Some(position) =
                                                    team.iter().position(|n| n == target)
                                            {
                                                team.rotate_left(position);
                                            }
                                        }
                                        if name.starts_with("切换角色-")
                                            || name.starts_with("连携技-")
                                        {
                                            for (position, name) in team.iter().enumerate() {
                                                if let Some(agent) = recognizer
                                                    .bundle
                                                    .spec
                                                    .agents
                                                    .iter()
                                                    .find(|a| &a.name == name)
                                                {
                                                    let prefix = if position == 0 {
                                                        "前台-".into()
                                                    } else {
                                                        format!("后台-{position}-")
                                                    };
                                                    for suffix in [&agent.name, &agent.kind] {
                                                        pending_states.push(StateUpdate {
                                                            name: format!("{prefix}{suffix}"),
                                                            at: now,
                                                            value: None,
                                                            add: None,
                                                            time_add: None,
                                                            clear: false,
                                                        });
                                                    }
                                                    if position == 0 {
                                                        pending_states.push(StateUpdate {
                                                            name: format!(
                                                                "切换角色-{}",
                                                                agent.name
                                                            ),
                                                            at: now,
                                                            value: None,
                                                            add: None,
                                                            time_add: None,
                                                            clear: false,
                                                        });
                                                        pending_states.push(StateUpdate {
                                                            name: format!(
                                                                "切换角色-{}",
                                                                agent.kind
                                                            ),
                                                            at: now,
                                                            value: None,
                                                            add: None,
                                                            time_add: None,
                                                            clear: false,
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Step::Action(action) => {
                            self.progress.actions += 1;
                            self.progress.action =
                                format!("{action:?}").chars().take(512).collect();
                            match action {
                                Action::Wait { seconds } => until = Some(now + seconds),
                                Action::Button { name, way, press } => {
                                    let switching =
                                        name.starts_with("切换角色-") && way != ButtonWay::Up;
                                    let mut steps = Vec::new();
                                    if way == ButtonWay::Up {
                                        steps.push(Step::Button(name.clone(), false));
                                    } else {
                                        steps.push(Step::Button(name.clone(), true));
                                        if way == ButtonWay::Tap || press.is_some() {
                                            steps.push(Step::Delay(press.unwrap_or(
                                                self.progress.config.tap_ms as f64 / 1000.0,
                                            )));
                                            steps.push(Step::Button(name.clone(), false));
                                        }
                                    }
                                    for step in steps.into_iter().rev() {
                                        queue.push_front(step);
                                    }
                                    if switching {
                                        queue.push_front(Step::ClearInputs);
                                    }
                                }
                                Action::SetState {
                                    names,
                                    seconds,
                                    seconds_add,
                                    value,
                                    add,
                                } => {
                                    for name in names {
                                        pending_states.push(StateUpdate {
                                            name,
                                            at: now + seconds,
                                            value,
                                            add,
                                            time_add: Some(seconds_add),
                                            clear: false,
                                        });
                                    }
                                }
                                Action::ClearState { names } => {
                                    for name in names {
                                        pending_states.push(StateUpdate {
                                            name,
                                            at: now,
                                            value: None,
                                            add: None,
                                            time_add: None,
                                            clear: true,
                                        });
                                    }
                                }
                                Action::SwitchAgent { name } => {
                                    let index = team_ready
                                        .then(|| self.progress.team.iter().position(|n| n == &name))
                                        .flatten();
                                    if let Some(index) = index {
                                        if index > 0 {
                                            queue.push_front(Step::Action(Action::Button {
                                                name: if index == 1 {
                                                    "切换角色-下一个"
                                                } else {
                                                    "切换角色-上一个"
                                                }
                                                .into(),
                                                way: ButtonWay::Tap,
                                                press: None,
                                            }));
                                        }
                                    } else {
                                        // 目标身份或方向尚不可靠，不能继续队列里的攻击动作。
                                        native.release_inputs()?;
                                        ledger.clear();
                                        pending_input = None;
                                        queue.clear();
                                        until = None;
                                        self.refresh_observation(&mut pending_states)?;
                                        self.sequence = None;
                                        last_fresh = None;
                                        refresh_pending = true;
                                        team_ready = false;
                                        recognizer.refresh_agents();
                                        if let Some(background) = worker.as_mut() {
                                            background.refresh_agents();
                                        }
                                        next_frame = now;
                                        self.message =
                                            format!("切换目标 {name} 信息暂不完整，重新识别队伍");
                                    }
                                }
                                Action::QuickAssist => {
                                    if let Some(name) = &observation.quick_assist {
                                        queue.push_front(Step::Action(Action::SwitchAgent {
                                            name: name.clone(),
                                        }));
                                    }
                                }
                                Action::Turn { distance } => {
                                    let distance = distance
                                        * self.progress.config.turn_sensitivity_milli as f64
                                        / 1000.0;
                                    ensure!(
                                        distance.is_finite() && distance.abs() <= 10000.0,
                                        "转向位移超出范围"
                                    );
                                    if self.progress.config.mode.is_gamepad() {
                                        let action = od_core::InputProbeAction::Stick {
                                            stick: od_core::InputSide::Right,
                                            x: if distance < 0.0 {
                                                -self.progress.config.gamepad_turn_speed
                                            } else {
                                                self.progress.config.gamepad_turn_speed
                                            },
                                            y: 0,
                                        };
                                        if distance != 0.0 {
                                            let mut inputs = Vec::new();
                                            for (action, down) in
                                                ledger.set("__turn", vec![action], true)
                                            {
                                                if asynchronous {
                                                    inputs.push(Step::NativeInput(
                                                        "__turn".into(),
                                                        action,
                                                        down,
                                                    ));
                                                } else if !native
                                                    .combat_input(action, down, &cancel)?
                                                {
                                                    return Ok(None);
                                                }
                                            }
                                            queue.push_front(Step::ReleaseTurn);
                                            queue.push_front(Step::Delay(distance.abs() / 1000.0));
                                            for input in inputs.into_iter().rev() {
                                                queue.push_front(input);
                                            }
                                        }
                                    } else if distance.round() as i32 != 0 {
                                        if asynchronous {
                                            queue.push_front(Step::Move(distance.round() as i32));
                                        } else {
                                            native.combat_turn(distance.round() as i32, &cancel)?;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if now - last_publish >= 0.1 {
                let _span = trace
                    .as_ref()
                    .map(|trace| trace.span("publish", json!({}), 0));
                self.progress.states = self
                    .scheduler
                    .states
                    .values()
                    .iter()
                    .take(128)
                    .map(|(n, v)| (n.clone(), v.at))
                    .collect();
                publish(self.progress.clone())?;
                last_publish = now;
            }
            let sleep_us = if asynchronous { 1000 } else { 2000 };
            let _sleep_span = trace
                .as_ref()
                .map(|trace| trace.span("slow_sleep", json!({"requested_us": sleep_us}), 5_000));
            let sleep_started = Instant::now();
            std::thread::sleep(Duration::from_micros(sleep_us));
            if let Some(performance) = &mut loop_performance {
                performance.sleep(sleep_started.elapsed().as_micros() as u64);
            }
        }
    }
}

#[cfg(test)]
#[path = "combat_io_tests.rs"]
mod io_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use od_core::conditional::FileTemplates;

    struct SlowReader {
        calls: std::sync::atomic::AtomicUsize,
        busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl maa_adapter::CombatVision for SlowReader {
        fn match_template(
            &self,
            _: &CombatFrame,
            _: [i32; 4],
            _: &maa_adapter::CombatTemplate,
        ) -> Result<Option<f64>> {
            if self.calls.fetch_add(1, std::sync::atomic::Ordering::AcqRel) > 0 {
                self.busy.store(true, std::sync::atomic::Ordering::Release);
                std::thread::sleep(Duration::from_millis(300));
                self.busy.store(false, std::sync::atomic::Ordering::Release);
            }
            Ok(Some(0.99))
        }
        fn recognize_prepared_cancellable(
            &self,
            _: &maa_adapter::RecognitionImage,
            _: &str,
            _: &serde_json::Value,
            _: &maa_adapter::RawRecognitionControl,
        ) -> Result<maa_adapter::RecognitionMeasurement> {
            Ok(maa_adapter::RecognitionMeasurement {
                detail: serde_json::json!({"adaptation":{"class_index":1}}),
                native_status: "fixture-flash".into(),
                timing: maa_adapter::RecognitionTiming {
                    session_setup_us: 0,
                    prepare_us: 0,
                    enqueue_us: 0,
                    completion_us: 0,
                    details_us: 0,
                    total_us: 0,
                },
            })
        }
    }
    type TimedKeys = std::sync::Arc<std::sync::Mutex<Vec<(bool, Instant, bool)>>>;
    struct TimedController {
        png: Vec<u8>,
        busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
        keys: TimedKeys,
    }
    impl maa_adapter::CustomControllerCallback for TimedController {
        fn connect(&self) -> bool {
            true
        }
        fn request_uuid(&self) -> Option<String> {
            Some("combat-timer-during-inference".into())
        }
        fn screencap(&self) -> Option<Vec<u8>> {
            Some(self.png.clone())
        }
        fn key_down(&self, key: i32) -> bool {
            if key == 69 || key == 16 {
                self.keys.lock().unwrap().push((
                    true,
                    Instant::now(),
                    self.busy.load(std::sync::atomic::Ordering::Acquire),
                ));
            }
            true
        }
        fn key_up(&self, key: i32) -> bool {
            if key == 69 || key == 16 {
                self.keys.lock().unwrap().push((
                    false,
                    Instant::now(),
                    self.busy.load(std::sync::atomic::Ordering::Acquire),
                ));
            }
            true
        }
    }

    #[test]
    #[ignore = "需要固定 Maa SDK、战斗资源及固定截图；只使用模拟控制器"]
    fn slow_recognition_does_not_delay_timed_release_or_stop_cleanup() -> Result<()> {
        use base64::Engine;
        use std::sync::{Arc, Mutex, atomic::AtomicBool};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()?;
        maa_adapter::load_sdk(
            &root.join("sdk/maa/bin"),
            &root.join("artifacts/combat-timer-logs"),
        )?;
        let png = {
            let fixture = NativeSession::for_fixture(
                &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
                &root.join("resources/generated/combat"),
                maa_adapter::InferenceDevice::Cpu,
            )?;
            base64::engine::general_purpose::STANDARD.decode(fixture.screenshot()?.png_base64)?
        };
        for way in [ButtonWay::Tap, ButtonWay::Down] {
            let busy = Arc::new(AtomicBool::new(false));
            let keys: TimedKeys = Arc::new(Mutex::new(Vec::new()));
            let mut native = NativeSession::for_custom_controller(
                TimedController {
                    png: png.clone(),
                    busy: busy.clone(),
                    keys: keys.clone(),
                },
                &root.join("resources/generated/combat"),
                od_core::InputMode::Foreground,
            )?;
            let mut recognizer =
                native.load_combat_bundle(&root.join("resources/generated/combat"))?;
            let mut script = Script::parse(
                "timer",
                "timer.yml",
                r#"
auto_lock_interval: -1
auto_turn_interval: -1
scenes:
  - handlers:
      - states: "[按键可用-普通攻击]"
        operations:
          - op_name: 等待秒数
            seconds: 0.04
          - op_name: 按键-特殊攻击
          - op_name: 等待秒数
            seconds: 1
"#,
                &FileTemplates {
                    root: root.join("resources/generated/combat/scripts"),
                },
            )?;
            script.scenes[0].handlers[0].actions[1] = Action::Button {
                name: "特殊攻击".into(),
                way,
                press: None,
            };
            let mut run = CombatRun::new(
                Uuid::new_v4(),
                CombatConfig {
                    start_delay_ms: 0,
                    timeout_ms: 3000,
                    ..Default::default()
                },
                script,
            )?;
            run.enable_performance(&root.join("artifacts/combat-performance-timers"), "fixture")?;
            let control = run.control();
            let worker = crate::combat_worker::CombatWorker::new(
                SlowReader {
                    calls: 0.into(),
                    busy: busy.clone(),
                },
                recognizer.clone(),
            )?;
            run.execute_with_worker(
                &mut native,
                &mut recognizer,
                false,
                &mut |_| {
                    let events = keys.lock().unwrap();
                    if (way == ButtonWay::Down && events.iter().any(|e| e.0))
                        || (way == ButtonWay::Tap && events.iter().any(|e| !e.0))
                    {
                        control.stop();
                    }
                    Ok(())
                },
                &mut |_, _, _, _| Ok(CombatFrameGate::Proceed),
                worker,
            )?;
            let events = keys.lock().unwrap();
            let down = events.iter().find(|e| e.0).context("未发送按下")?;
            let up = events.iter().find(|e| !e.0).context("未释放按键")?;
            let hold_ms = (up.1 - down.1).as_millis();
            eprintln!(
                "{way:?}: 持有 {hold_ms}ms，按下/释放时识别在途 {}/{}",
                down.2, up.2
            );
            assert!(down.2 && up.2, "按下和释放均须发生在慢识别尚未结束时");
            if way == ButtonWay::Tap {
                assert!(hold_ms >= 20, "计时点按不能提前松开");
                assert!(hold_ms < 100, "20ms 点按不能被 300ms 识别拖住");
            }
            assert!(
                !busy.load(std::sync::atomic::Ordering::Acquire),
                "返回前必须等待工作线程结束"
            );
        }
        Ok(())
    }

    #[test]
    #[ignore = "需要固定 Maa SDK、战斗资源及固定截图；只使用模拟控制器"]
    fn flash_is_dispatched_before_slow_agent_recognition_finishes() -> Result<()> {
        use base64::Engine;
        use std::sync::{Arc, Mutex, atomic::AtomicBool};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()?;
        maa_adapter::load_sdk(
            &root.join("sdk/maa/bin"),
            &root.join("artifacts/combat-timer-logs"),
        )?;
        let png = {
            let fixture = NativeSession::for_fixture(
                &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
                &root.join("resources/generated/combat"),
                maa_adapter::InferenceDevice::Cpu,
            )?;
            base64::engine::general_purpose::STANDARD.decode(fixture.screenshot()?.png_base64)?
        };
        let busy = Arc::new(AtomicBool::new(false));
        let keys: TimedKeys = Arc::new(Mutex::new(Vec::new()));
        let mut native = NativeSession::for_custom_controller(
            TimedController {
                png,
                busy: busy.clone(),
                keys: keys.clone(),
            },
            &root.join("resources/generated/combat"),
            od_core::InputMode::Foreground,
        )?;
        let mut recognizer = native.load_combat_bundle(&root.join("resources/generated/combat"))?;
        let script = Script::parse(
            "early-flash",
            "early-flash.yml",
            r#"
auto_lock_interval: -1
auto_turn_interval: -1
scenes:
  - triggers: [闪避识别-红光]
    priority: 99
    handlers:
      - states: "[闪避识别-红光]"
        operations:
          - op_name: 按键-闪避
  - handlers:
      - states: "[前台-露西]"
        operations:
          - op_name: 等待秒数
            seconds: 1
"#,
            &FileTemplates {
                root: root.join("resources/generated/combat/scripts"),
            },
        )?;
        let mut run = CombatRun::new(
            Uuid::new_v4(),
            CombatConfig {
                start_delay_ms: 0,
                timeout_ms: 3000,
                ..Default::default()
            },
            script,
        )?;
        let control = run.control();
        let worker = crate::combat_worker::CombatWorker::new(
            SlowReader {
                calls: 0.into(),
                busy: busy.clone(),
            },
            recognizer.clone(),
        )?;
        run.execute_with_worker(
            &mut native,
            &mut recognizer,
            false,
            &mut |progress| {
                if keys.lock().unwrap().iter().any(|e| !e.0) {
                    assert_eq!(progress.frames, 0, "首个完整识别尚未结束时应已响应闪光");
                    control.stop();
                }
                Ok(())
            },
            &mut |_, _, _, _| Ok(CombatFrameGate::Proceed),
            worker,
        )?;
        let events = keys.lock().unwrap();
        assert_eq!(
            events.iter().filter(|e| e.0).count(),
            1,
            "同一闪光不能被完整结果重复触发"
        );
        assert!(events.iter().all(|e| e.2), "闪避输入不能等慢头像识别结束");
        Ok(())
    }

    #[test]
    #[ignore = "需要固定 Maa SDK、战斗资源及固定截图；只使用模拟控制器"]
    fn worker_drops_old_epoch_and_keeps_only_latest_pending_frame() -> Result<()> {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()?;
        maa_adapter::load_sdk(
            &root.join("sdk/maa/bin"),
            &root.join("artifacts/combat-timer-logs"),
        )?;
        let bundle = maa_adapter::combat_vision::CombatBundle::load(
            &root.join("resources/generated/combat"),
        )?;
        let native = NativeSession::for_fixture(
            &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
            &bundle.root,
            maa_adapter::InferenceDevice::Cpu,
        )?;
        let busy = Arc::new(AtomicBool::new(false));
        let mut worker = crate::combat_worker::CombatWorker::new(
            SlowReader {
                calls: 1.into(),
                busy: busy.clone(),
            },
            CombatRecognizer::new(bundle),
        )?;
        let cancel = AtomicBool::new(false);
        let intervals = BTreeMap::from([("flash_enabled".into(), 0.0)]);
        worker.capture(native.combat_frame(&cancel)?.unwrap(), 0.0);
        worker.submit(intervals.clone(), true)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while !busy.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(busy.load(Ordering::Acquire));
        worker.refresh_agents();
        worker.capture(native.combat_frame(&cancel)?.unwrap(), 1.0);
        worker.capture(native.combat_frame(&cancel)?.unwrap(), 2.0);
        let observed = loop {
            ensure!(Instant::now() < deadline, "等待新代际识别超时");
            if let Some(frame) = worker.poll(&mut Vec::new())? {
                break frame;
            }
            worker.submit(intervals.clone(), true)?;
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(observed.at, 2.0, "旧代际与被覆盖的待识别帧均不能回放");
        worker.capture(native.combat_frame(&cancel)?.unwrap(), 3.0);
        worker.submit(BTreeMap::from([("flash_enabled".into(), 1.0)]), true)?;
        let mut early = Vec::new();
        let completed = loop {
            ensure!(Instant::now() < deadline, "等待闪光结果超时");
            if let Some(frame) = worker.poll(&mut early)? {
                break frame;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(early.len(), 1, "一帧闪光只提前投递一次");
        assert!(early[0].states.iter().any(|s| s.name == "闪避识别-红光"));
        assert!(
            !completed
                .observation
                .states
                .iter()
                .any(|s| s.name == "闪避识别-红光"),
            "完整结果不能再次触发已经投递的闪光"
        );
        worker.finish()?;
        Ok(())
    }

    fn run() -> Result<CombatRun> {
        let script = Script::parse(
            "team-test",
            "team-test.yml",
            r#"
scenes:
  - handlers:
      - states: ""
        operations:
          - op_name: 等待秒数
            seconds: 0.02
"#,
            &FileTemplates {
                root: std::path::PathBuf::new(),
            },
        )?;
        CombatRun::new(Uuid::new_v4(), CombatConfig::default(), script)
    }
    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn partial_portraits_preserve_known_quick_assist_members() -> Result<()> {
        let mut run = run()?;
        assert!(run.observe_team(&names(&["露西", "叶瞬光", "照"])));
        assert!(run.observe_team(&names(&["露西", "", ""])));
        assert_eq!(run.progress.team, names(&["露西", "叶瞬光", "照"]));
        // 兼容只报告前台的旧观察，不能将已有三人队伍覆盖成单人。
        assert!(run.observe_team(&names(&["露西"])));
        assert_eq!(run.progress.team, names(&["露西", "叶瞬光", "照"]));
        assert!(run.observe_team(&names(&["照", "", "叶瞬光"])));
        assert_eq!(run.progress.team, names(&["照", "露西", "叶瞬光"]));
        Ok(())
    }

    #[test]
    fn unknown_or_conflicting_slots_do_not_authorize_a_named_switch() -> Result<()> {
        let mut run = run()?;
        assert!(!run.observe_team(&names(&["露西", "", ""])));
        assert!(run.progress.team.is_empty());
        assert!(run.observe_team(&names(&["露西", "叶瞬光", "照"])));
        for observed in [
            names(&["", "叶瞬光", "照"]),
            names(&["露西", "照", ""]),
            names(&["露西", "露西", "照"]),
            names(&["千夏", "", ""]),
        ] {
            assert!(!run.observe_team(&observed));
            assert_eq!(run.progress.team, names(&["露西", "叶瞬光", "照"]));
        }
        Ok(())
    }
}
