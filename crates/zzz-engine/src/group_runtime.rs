use super::*;

enum GroupAppWork {
    Patrol(Box<PatrolWork>),
    Temple(Box<TempleWork>),
    Intel(Box<IntelWork>),
    Mail(MailWork),
    Coffee(CoffeeWork),
    Daily(DailyWork),
}
struct TempleWork {
    run: crate::suibian_run::TempleRun,
    window: WindowInfo,
    capture: CaptureMethod,
    fixture: bool,
}
struct IntelWork {
    run: crate::intel_board::IntelRun,
    window: WindowInfo,
    capture: CaptureMethod,
    fixture: bool,
}
struct PatrolWork {
    run: crate::world_patrol::PatrolRun,
    window: WindowInfo,
    capture: CaptureMethod,
    fixture: bool,
}

struct DailyWork {
    run: Box<crate::basic_daily::DailyRun>,
    window: WindowInfo,
    capture: CaptureMethod,
    fixture: bool,
    prepared: bool,
    events: Option<maa_adapter::PipelineEvents>,
    dropped: u64,
}

pub(super) struct GroupWork {
    id: Uuid,
    plan: GroupPlan,
    window: WindowInfo,
    capture: CaptureMethod,
    fixture: bool,
    checked: bool,
    current: Option<GroupAppWork>,
    intel: Option<crate::intel_board::PreparedIntel>,
    patrol: Option<crate::intel_board::PreparedIntel>,
}

impl GroupWork {
    pub(super) fn id(&self) -> Uuid {
        self.id
    }

    fn prepare(
        &mut self,
        record: &RunRecord,
    ) -> anyhow::Result<od_core::pipeline::PipelineRequest> {
        let app = record
            .group
            .as_ref()
            .and_then(|g| g.current_index.map(|i| g.steps[i].app_id.as_str()))
            .ok_or_else(|| anyhow::anyhow!("缺少当前分项"))?;
        let (work, control) = match app {
            "world_patrol" => {
                let progress = record
                    .daily
                    .iter()
                    .find(|p| p.app_id == app)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("缺少锄大地检查点"))?;
                let run = crate::world_patrol::PatrolRun::new(
                    record.id,
                    progress,
                    self.patrol
                        .take()
                        .ok_or_else(|| anyhow::anyhow!("缺少锄大地预检脚本"))?,
                    self.plan.game_day.clone(),
                )?;
                let control = run.control();
                (
                    GroupAppWork::Patrol(Box::new(PatrolWork {
                        run,
                        window: self.window.clone(),
                        capture: self.capture,
                        fixture: self.fixture,
                    })),
                    control,
                )
            }
            "suibian_temple" => {
                let progress = record
                    .daily
                    .iter()
                    .find(|p| p.app_id == app)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("缺少随便观检查点"))?;
                let run = crate::suibian_run::TempleRun::new(record.id, progress)?;
                let control = run.control();
                (
                    GroupAppWork::Temple(Box::new(TempleWork {
                        run,
                        window: self.window.clone(),
                        capture: self.capture,
                        fixture: self.fixture,
                    })),
                    control,
                )
            }
            "intel_board" => {
                let progress = record
                    .daily
                    .iter()
                    .find(|p| p.app_id == app)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("缺少情报板检查点"))?;
                let run = crate::intel_board::IntelRun::new(
                    record.id,
                    progress,
                    self.intel
                        .take()
                        .ok_or_else(|| anyhow::anyhow!("缺少预检战斗脚本"))?,
                    self.plan.predefined_teams.clone(),
                )?;
                let control = run.control();
                (
                    GroupAppWork::Intel(Box::new(IntelWork {
                        run,
                        window: self.window.clone(),
                        capture: self.capture,
                        fixture: self.fixture,
                    })),
                    control,
                )
            }
            "email" => {
                let run = Box::new(MailRun::with_id(record.id, self.plan.mail_config)?);
                let control = run.control();
                (
                    GroupAppWork::Mail(MailWork {
                        fixture: self.fixture,
                        run,
                        window: self.window.clone(),
                        capture: self.capture,
                        prepared: false,
                        events: None,
                        dropped: 0,
                    }),
                    control,
                )
            }
            "coffee" => {
                let run = Box::new(CoffeeRun::from_world(record.id, self.plan.coffee_config)?);
                let control = run.control();
                (
                    GroupAppWork::Coffee(CoffeeWork {
                        fixture: self.fixture,
                        run,
                        window: self.window.clone(),
                        capture: self.capture,
                        prepared: false,
                        events: None,
                        dropped: 0,
                    }),
                    control,
                )
            }
            app if od_core::basic_daily::APP_IDS.contains(&app) => {
                let mut run = Box::new(crate::basic_daily::DailyRun::new(
                    record.id,
                    app,
                    self.plan.daily_config,
                )?);
                run.progress = record
                    .daily
                    .iter()
                    .find(|p| p.app_id == app)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("缺少日常分项检查点"))?;
                let control = run.control();
                (
                    GroupAppWork::Daily(DailyWork {
                        run,
                        window: self.window.clone(),
                        capture: self.capture,
                        fixture: self.fixture,
                        prepared: false,
                        events: None,
                        dropped: 0,
                    }),
                    control,
                )
            }
            _ => anyhow::bail!("分项没有执行器"),
        };
        self.current = Some(work);
        Ok(control)
    }

    pub(super) fn control(&mut self, resume: bool) {
        match &mut self.current {
            Some(GroupAppWork::Patrol(work)) => {
                if resume
                    && work.run.interrupt_requested() != Some(od_core::pipeline::Interrupt::Stop)
                {
                    if let Err(error) = work.run.resume() {
                        work.run.progress.state = MailState::Failed;
                        work.run.message = error.to_string();
                    }
                } else {
                    work.run.control().stop();
                }
            }
            Some(GroupAppWork::Temple(work)) => {
                if resume
                    && work.run.interrupt_requested() != Some(od_core::pipeline::Interrupt::Stop)
                {
                    if let Err(error) = work.run.resume() {
                        work.run.progress.state = MailState::Failed;
                        work.run.message = error.to_string();
                    }
                } else {
                    work.run.control().stop();
                }
            }
            Some(GroupAppWork::Intel(work)) => {
                if resume
                    && work.run.interrupt_requested() != Some(od_core::pipeline::Interrupt::Stop)
                {
                    if let Err(error) = work.run.resume() {
                        work.run.progress.state = MailState::Failed;
                        work.run.message = error.to_string();
                    }
                } else {
                    work.run.control().stop();
                }
            }
            Some(GroupAppWork::Mail(work)) => {
                if resume
                    && work.run.interrupt_requested() != Some(od_core::pipeline::Interrupt::Stop)
                {
                    if let Err(error) = work.run.resume() {
                        work.run.state = MailState::Failed;
                        work.run.message = error.to_string();
                    }
                } else {
                    work.run.control().stop();
                }
            }
            Some(GroupAppWork::Coffee(work)) => {
                if resume
                    && work.run.interrupt_requested() != Some(od_core::pipeline::Interrupt::Stop)
                {
                    if let Err(error) = work.run.resume() {
                        work.run.progress.state = CoffeeState::Failed;
                        work.run.message = error.to_string();
                    }
                } else {
                    work.run.control().stop();
                }
            }
            Some(GroupAppWork::Daily(work)) => {
                if resume
                    && work.run.interrupt_requested() != Some(od_core::pipeline::Interrupt::Stop)
                {
                    if let Err(error) = work.run.resume() {
                        work.run.progress.state = MailState::Failed;
                        work.run.message = error.to_string();
                    }
                } else {
                    work.run.control().stop();
                }
            }
            None => {}
        }
    }
}

impl Engine {
    pub(super) fn start_group_work(
        &self,
        state: &mut State,
        plan: GroupPlan,
        window: WindowInfo,
    ) -> Result<Payload, ApiError> {
        let intel = if plan.items.iter().any(|i| {
            i.app_id == "intel_board" && i.decision == od_core::run_plan::PlanDecision::Run
        }) {
            Some(
                crate::intel_board::PreparedIntel::load(
                    plan.app_combat
                        .clone()
                        .ok_or_else(|| ApiError::new("invalid_combat", "缺少情报板冻结战斗设置"))?,
                    &self.combat_root,
                    &self.combat_data,
                )
                .map_err(native_error)?,
            )
        } else {
            None
        };
        let patrol = if plan.items.iter().any(|i| {
            i.app_id == "world_patrol" && i.decision == od_core::run_plan::PlanDecision::Run
        }) {
            Some(
                crate::intel_board::PreparedIntel::load(
                    plan.world_patrol_combat
                        .clone()
                        .ok_or_else(|| ApiError::new("invalid_combat", "缺少锄大地冻结战斗设置"))?,
                    &self.combat_root,
                    &self.combat_data,
                )
                .map_err(native_error)?,
            )
        } else {
            None
        };
        let record = state
            .slot
            .begin_group_plan(plan.mail_config, &plan)
            .map_err(|e| ApiError::new("invalid_group", e))?;
        let mut work = Box::new(GroupWork {
            id: record.id,
            plan,
            window,
            capture: state.capture_method.unwrap_or_default(),
            fixture: self.fixture,
            checked: false,
            current: None,
            intel,
            patrol,
        });
        let control = work.prepare(&record).map_err(native_error)?;
        if let Err(error) = self.history.save(&record) {
            let message = format!("启动记录保存失败，未投递游戏动作：{error}");
            let failed = fail_current(state, message.clone());
            state.failed_save = failed;
            state.record_error = Some(message.clone());
            return Err(ApiError::new("history_error", message));
        }
        state
            .slot
            .confirm_group_saved(&record)
            .map_err(|e| ApiError::new("history_error", e))?;
        if self.sender.try_send(Work::Group(work)).is_err() {
            if let Some(failed) = fail_current(state, "工作线程不可用，未投递游戏动作".into())
                && let Err(error) = self.history.save(&failed)
            {
                state.failed_save = Some(failed);
                state.record_error = Some(error.to_string());
            }
            return Err(ApiError::new(
                "worker_unavailable",
                "工作线程无法接收应用组",
            ));
        }
        state.pipeline_control = Some(control);
        state.event(
            Some(record.id),
            "info",
            "开始应用组试运行，按保存顺序执行首批日常".into(),
        );
        Ok(Payload::Run(record))
    }
}

fn fail_current(state: &mut State, message: String) -> Option<RunRecord> {
    let record = state.slot.current()?.clone();
    let app = record
        .group
        .as_ref()?
        .current_index
        .map(|i| record.group.as_ref().unwrap().steps[i].app_id.as_str());
    let result = match app {
        Some("email") => {
            let mut p = record.mail?;
            p.state = MailState::Failed;
            p.inputs_released = true;
            state.slot.update_mail(record.id, p, message)
        }
        Some("coffee") => {
            let mut p = record.coffee?;
            p.state = CoffeeState::Failed;
            p.inputs_released = true;
            state.slot.update_coffee(record.id, p, message)
        }
        Some(app) if od_core::basic_daily::APP_IDS.contains(&app) => {
            let mut p = record.daily.iter().find(|p| p.app_id == app)?.clone();
            p.state = MailState::Failed;
            p.inputs_released = true;
            state.slot.update_daily(record.id, p, message)
        }
        _ => state.slot.finish(record.id, RunState::Failed, message),
    };
    result.ok()
}

pub(super) fn run_group_work(
    mut work: Box<GroupWork>,
    paths: &EnginePaths,
    shared: &Arc<Mutex<State>>,
    native: &mut Option<NativeSession>,
) -> Option<Box<GroupWork>> {
    if !work.checked {
        let preflight = (|| -> anyhow::Result<()> {
            for item in &work.plan.items {
                if item.decision != od_core::run_plan::PlanDecision::Run {
                    continue;
                }
                match item.app_id.as_str() {
                    "world_patrol" => {
                        maa_adapter::world_patrol::validate_bundle(
                            &paths.resources.join("generated/world-patrol"),
                        )?;
                    }
                    "email" => {
                        maa_adapter::validate_mail_bundle(&paths.resources.join("generated/mail"))?;
                    }
                    "coffee" => {
                        let bundle = maa_adapter::validate_coffee_run_bundle(
                            &paths.resources.join("generated/coffee-run"),
                        )?;
                        maa_adapter::validate_coffee_approach_bundle(&bundle)?;
                        maa_adapter::validate_map_bundle(
                            &paths.resources.join("generated/map-preview"),
                        )?;
                    }
                    "suibian_temple" => {
                        maa_adapter::suibian_temple::validate_full_temple_bundle(
                            &paths.resources.join("generated/suibian-temple"),
                        )?;
                    }
                    app if od_core::basic_daily::APP_IDS.contains(&app) => {
                        maa_adapter::validate_basic_daily_app_bundle(
                            &paths.resources.join("generated/basic-daily"),
                            app,
                        )?;
                    }
                    _ => anyhow::bail!("分项没有执行器"),
                }
            }
            Ok(())
        })();
        if let Err(error) = preflight {
            let mut state = shared.lock().expect("运行状态锁失效");
            if let Some(record) = fail_current(
                &mut state,
                format!("整组资源预检失败，未投递输入：{error:#}"),
            ) {
                save_run(paths, &mut state, &record);
            }
            state.pipeline_control = None;
            return None;
        }
        work.checked = true;
    }
    loop {
        if let Some(current) = work.current.take() {
            work.current = match current {
                GroupAppWork::Patrol(app) => {
                    run_patrol_work(app, paths, shared, native).map(GroupAppWork::Patrol)
                }
                GroupAppWork::Temple(app) => {
                    run_temple_work(app, paths, shared, native).map(GroupAppWork::Temple)
                }
                GroupAppWork::Intel(app) => {
                    run_intel_work(app, paths, shared, native).map(GroupAppWork::Intel)
                }
                GroupAppWork::Mail(app) => {
                    run_mail_work(app, paths, shared, native).map(GroupAppWork::Mail)
                }
                GroupAppWork::Coffee(app) => {
                    run_coffee_work(app, paths, shared, native).map(GroupAppWork::Coffee)
                }
                GroupAppWork::Daily(app) => {
                    run_daily_work(app, paths, shared, native).map(GroupAppWork::Daily)
                }
            };
            if work.current.is_some() {
                return Some(work);
            }
        }
        let mut state = shared.lock().expect("运行状态锁失效");
        let record = state.slot.current()?.clone();
        if record.id != work.id {
            return None;
        }
        if record.state.is_terminal() {
            save_run(paths, &mut state, &record);
            state.pipeline_control = None;
            return None;
        }
        if state.record_error.is_some()
            || matches!(record.state, RunState::Paused | RunState::Pausing)
        {
            let message = if state.record_error.is_some() {
                "分项结果未保存，重试保存后才能继续下一项".into()
            } else {
                "应用组已在分项交接处暂停".into()
            };
            if let Ok(record) = state.slot.pause_group_boundary(message) {
                save_run(paths, &mut state, &record);
            }
            return Some(work);
        }
        let next = state.slot.advance_group(&work.plan);
        let record = match next {
            Ok(record) => record,
            Err(error) => {
                if let Some(record) = fail_current(&mut state, error.into()) {
                    save_run(paths, &mut state, &record);
                }
                return None;
            }
        };
        save_run(paths, &mut state, &record);
        if state.record_error.is_some() {
            if let Some(record) =
                fail_current(&mut state, "分项启动检查点未保存，未投递输入".into())
            {
                save_run(paths, &mut state, &record);
            }
            return None;
        }
        match work.prepare(&record) {
            Ok(control) => state.pipeline_control = Some(control),
            Err(error) => {
                if let Some(record) = fail_current(&mut state, error.to_string()) {
                    save_run(paths, &mut state, &record);
                }
                return None;
            }
        }
    }
}

fn run_intel_work(
    mut work: Box<IntelWork>,
    paths: &EnginePaths,
    shared: &Arc<Mutex<State>>,
    native: &mut Option<NativeSession>,
) -> Option<Box<IntelWork>> {
    let id = work.run.id;
    let result = if work.run.progress.state.is_terminal() {
        Ok(())
    } else {
        work.run.execute(
            native,
            crate::intel_board::IntelSessionOptions {
                window: &work.window,
                capture: work.capture,
                resources: &paths.resources,
                fixture: work.fixture,
            },
            |progress| {
                let mut state = shared.lock().expect("运行状态锁失效");
                let record = state
                    .slot
                    .update_daily(id, progress, "情报板执行中".into())
                    .map_err(anyhow::Error::msg)?;
                save_run(paths, &mut state, &record);
                anyhow::ensure!(
                    state.record_error.is_none(),
                    "情报板检查点保存失败，未投递后续输入"
                );
                Ok(())
            },
        )
    };
    if let Err(error) = result {
        work.run.message = format!("情报板失败：{error:#}");
    }
    let mut state = shared.lock().expect("运行状态锁失效");
    match state
        .slot
        .update_daily(id, work.run.progress.clone(), work.run.message.clone())
    {
        Ok(record) => {
            save_run(paths, &mut state, &record);
            state.event(
                Some(id),
                if record.state == RunState::Failed {
                    "error"
                } else {
                    "info"
                },
                record.message,
            );
        }
        Err(error) => {
            state.event(
                Some(id),
                "error",
                format!("情报板结果未能写入运行槽：{error}"),
            );
            // 发布失败不能让外层误认为分项已结束并推进下一个应用。
            if let Some(mut previous) = state
                .slot
                .current()
                .and_then(|r| r.daily.iter().find(|p| p.app_id == "intel_board"))
                .cloned()
            {
                previous.state = MailState::Failed;
                previous.inputs_released = work.run.progress.inputs_released;
                if let Ok(record) =
                    state
                        .slot
                        .update_daily(id, previous, format!("情报板检查点无效：{error}"))
                {
                    save_run(paths, &mut state, &record);
                }
            }
        }
    }
    if work.run.progress.state == MailState::Paused {
        Some(work)
    } else {
        state.pipeline_control = None;
        None
    }
}

fn run_patrol_work(
    mut work: Box<PatrolWork>,
    paths: &EnginePaths,
    shared: &Arc<Mutex<State>>,
    native: &mut Option<NativeSession>,
) -> Option<Box<PatrolWork>> {
    let id = work.run.id;
    let result = if work.run.progress.state.is_terminal() {
        Ok(())
    } else {
        work.run.execute(
            native,
            crate::intel_board::IntelSessionOptions {
                window: &work.window,
                capture: work.capture,
                resources: &paths.resources,
                fixture: work.fixture,
            },
            |progress| {
                let mut state = shared.lock().expect("运行状态锁失效");
                let record = state
                    .slot
                    .update_daily(id, progress, "锄大地执行中".into())
                    .map_err(anyhow::Error::msg)?;
                save_run(paths, &mut state, &record);
                anyhow::ensure!(
                    state.record_error.is_none(),
                    "锄大地检查点保存失败，未投递后续输入"
                );
                Ok(())
            },
        )
    };
    if let Err(error) = result {
        work.run.message = format!("锄大地失败：{error:#}");
    }
    let mut state = shared.lock().expect("运行状态锁失效");
    match state
        .slot
        .update_daily(id, work.run.progress.clone(), work.run.message.clone())
    {
        Ok(record) => {
            save_run(paths, &mut state, &record);
            state.event(
                Some(id),
                if record.state == RunState::Failed {
                    "error"
                } else {
                    "info"
                },
                record.message,
            );
        }
        Err(error) => {
            state.event(
                Some(id),
                "error",
                format!("锄大地结果未能写入运行槽：{error}"),
            );
            // 发布失败不能让外层误认为分项已结束并推进下一个应用。
            if let Some(mut previous) = state
                .slot
                .current()
                .and_then(|r| r.daily.iter().find(|p| p.app_id == "world_patrol"))
                .cloned()
            {
                previous.state = MailState::Failed;
                previous.inputs_released = work.run.progress.inputs_released;
                if let Ok(record) =
                    state
                        .slot
                        .update_daily(id, previous, format!("锄大地检查点无效：{error}"))
                {
                    save_run(paths, &mut state, &record);
                }
            }
        }
    }
    if work.run.progress.state == MailState::Paused {
        Some(work)
    } else {
        state.pipeline_control = None;
        None
    }
}

fn run_temple_work(
    mut work: Box<TempleWork>,
    paths: &EnginePaths,
    shared: &Arc<Mutex<State>>,
    native: &mut Option<NativeSession>,
) -> Option<Box<TempleWork>> {
    let id = work.run.id;
    if !work.run.progress.state.is_terminal() {
        let result = work.run.execute(
            native,
            crate::suibian_run::TempleSessionOptions {
                window: &work.window,
                capture: work.capture,
                resources: &paths.resources,
                fixture: work.fixture,
            },
            |progress| {
                let mut state = shared.lock().expect("运行状态锁失效");
                let record = state
                    .slot
                    .update_daily(id, progress, "随便观执行中".into())
                    .map_err(anyhow::Error::msg)?;
                save_run(paths, &mut state, &record);
                anyhow::ensure!(
                    state.record_error.is_none(),
                    "随便观检查点保存失败，未投递后续输入"
                );
                Ok(())
            },
        );
        if let Err(error) = result {
            work.run.message = format!("随便观失败：{error:#}");
        }
    }
    let mut state = shared.lock().expect("运行状态锁失效");
    match state
        .slot
        .update_daily(id, work.run.progress.clone(), work.run.message.clone())
    {
        Ok(record) => {
            save_run(paths, &mut state, &record);
            state.event(
                Some(id),
                if record.state == RunState::Failed {
                    "error"
                } else {
                    "info"
                },
                record.message,
            );
        }
        Err(error) => {
            state.event(
                Some(id),
                "error",
                format!("随便观结果未能写入运行槽：{error}"),
            );
            if let Some(mut previous) = state
                .slot
                .current()
                .and_then(|r| r.daily.iter().find(|p| p.app_id == "suibian_temple"))
                .cloned()
            {
                previous.state = MailState::Failed;
                previous.inputs_released = work.run.progress.inputs_released;
                if let Ok(record) =
                    state
                        .slot
                        .update_daily(id, previous, format!("随便观检查点无效：{error}"))
                {
                    save_run(paths, &mut state, &record);
                }
            }
        }
    }
    if work.run.progress.state == MailState::Paused {
        Some(work)
    } else {
        state.pipeline_control = None;
        None
    }
}

fn run_daily_work(
    mut work: DailyWork,
    paths: &EnginePaths,
    shared: &Arc<Mutex<State>>,
    native: &mut Option<NativeSession>,
) -> Option<DailyWork> {
    let result = (|| -> anyhow::Result<()> {
        if work.run.progress.state.is_terminal() {
            return Ok(());
        }
        if !work.prepared && work.run.interrupt_requested().is_some() {
            if let Some(controller) = native.as_mut() {
                controller.release_inputs()?;
            }
            work.run.acknowledge_idle();
            return Ok(());
        }
        if !work.prepared {
            let bundle = maa_adapter::validate_basic_daily_app_bundle(
                &paths.resources.join("generated/basic-daily"),
                &work.run.progress.app_id,
            )?;
            if work.fixture {
                native
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("离线日常控制器未准备"))?
                    .load_basic_daily_bundle(&bundle)?;
            } else {
                if let Some(controller) = native.as_mut() {
                    controller.release_inputs()?;
                }
                native.take();
                let current = od_platform::parse_game_window(&work.window.hwnd)?;
                anyhow::ensure!(
                    current.pid == work.window.pid
                        && current.width == 1920
                        && current.height == 1080,
                    "游戏窗口身份或尺寸已变化，请重连"
                );
                let mut controller = NativeSession::for_game(
                    &current,
                    &paths.resources,
                    work.capture,
                    InputMode::Foreground,
                    InferenceDevice::Cpu,
                )?;
                controller.load_basic_daily_bundle(&bundle)?;
                *native = Some(controller);
            }
            work.events = Some(
                native
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("日常控制器未连接"))?
                    .pipeline_events(EVENT_CAPACITY)?,
            );
            work.prepared = true;
        }
        let id = work.run.id;
        let events = work.events.as_ref();
        let publish = |progress| -> anyhow::Result<()> {
            let mut state = shared.lock().expect("运行状态锁失效");
            if let Some(events) = events {
                for event in events.receiver.try_iter() {
                    state.event(
                        Some(id),
                        "debug",
                        format!("{} {}", event.message, event.node.as_deref().unwrap_or("")),
                    );
                }
            }
            let record = state
                .slot
                .update_daily(id, progress, "日常流程执行中".into())
                .map_err(anyhow::Error::msg)?;
            save_run(paths, &mut state, &record);
            anyhow::ensure!(
                state.record_error.is_none(),
                "日常检查点未保存，未投递后续输入"
            );
            Ok(())
        };
        work.run.execute(
            native
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("日常控制器未连接"))?,
            !work.fixture,
            publish,
        )?;
        if work.run.progress.stage == "Save" && work.run.progress.state == MailState::Running {
            let mut state = shared.lock().expect("运行状态锁失效");
            if work.run.interrupt_requested().is_some() {
                work.run.acknowledge_idle();
                return Ok(());
            }
            let active = state
                .slot
                .current()
                .ok_or_else(|| anyhow::anyhow!("编队运行已不存在"))?;
            anyhow::ensure!(
                active.id == id && active.state == RunState::Running,
                "编队运行身份或状态已变化，未回写配置"
            );
            let context = active
                .context
                .clone()
                .ok_or_else(|| anyhow::anyhow!("编队运行缺少账号"))?;
            // 先保存完成扫描、返回及释放输入的检查点，失败时不能改写账号。
            let record = state
                .slot
                .update_daily(
                    id,
                    work.run.progress.clone(),
                    "编队扫描已完成，准备保存可靠结果".into(),
                )
                .map_err(anyhow::Error::msg)?;
            save_run(paths, &mut state, &record);
            anyhow::ensure!(
                state.record_error.is_none(),
                "编队结果检查点未保存，未回写配置"
            );
            let scan = work
                .run
                .progress
                .team_check
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("缺少编队扫描结果"))?;
            let revision = od_core::team_check::save_members(
                &ConfigStore::new(&paths.data)?,
                context.instance_id,
                context.config_revision,
                scan,
            )?;
            work.run.finish_team_save(revision)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        work.run.progress.state = MailState::Failed;
        work.run.message = format!("日常流程失败：{error:#}");
        work.run.progress.inputs_released = native
            .as_mut()
            .is_none_or(|controller| controller.release_inputs().is_ok());
    }
    let mut state = shared.lock().expect("运行状态锁失效");
    if let Some(events) = &work.events {
        for event in events.receiver.try_iter() {
            state.event(
                Some(work.run.id),
                "debug",
                format!("{} {}", event.message, event.node.as_deref().unwrap_or("")),
            );
        }
        let dropped = events.dropped();
        state.dropped += dropped.saturating_sub(work.dropped);
        work.dropped = dropped;
    }
    if let Ok(record) = state.slot.update_daily(
        work.run.id,
        work.run.progress.clone(),
        work.run.message.clone(),
    ) {
        save_run(paths, &mut state, &record);
        state.event(
            Some(record.id),
            if record.state == RunState::Failed {
                "error"
            } else {
                "info"
            },
            record.message,
        );
    }
    if work.run.progress.state == MailState::Paused {
        Some(work)
    } else {
        state.pipeline_control = None;
        None
    }
}
