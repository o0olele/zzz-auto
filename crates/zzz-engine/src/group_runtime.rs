use super::*;

enum GroupAppWork {
    Mail(MailWork),
    Coffee(CoffeeWork),
}

pub(super) struct GroupWork {
    id: Uuid,
    plan: GroupPlan,
    window: WindowInfo,
    capture: CaptureMethod,
    fixture: bool,
    checked: bool,
    current: Option<GroupAppWork>,
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
            _ => anyhow::bail!("分项没有执行器"),
        };
        self.current = Some(work);
        Ok(control)
    }

    pub(super) fn control(&mut self, resume: bool) {
        match &mut self.current {
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
            "开始应用组试运行，按保存顺序执行邮件和咖啡".into(),
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
                GroupAppWork::Mail(app) => {
                    run_mail_work(app, paths, shared, native).map(GroupAppWork::Mail)
                }
                GroupAppWork::Coffee(app) => {
                    run_coffee_work(app, paths, shared, native).map(GroupAppWork::Coffee)
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
