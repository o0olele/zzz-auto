//! 完整随便观运行；所有子流程共用父控制器、运行槽和剩余活动预算。
use crate::suibian_temple::HostingEnd;
use anyhow::{Context, Result, ensure};
use maa_adapter::{GameSessionConfig, InferenceDevice, NativeSession, PipelineEnd, SessionHandoff};
use od_core::{
    CaptureMethod, InputMode, WindowInfo,
    basic_daily::DailyProgress,
    mail::MailState,
    pipeline::{Interrupt, PipelineControl, PipelineRequest},
    suibian_temple_flow::{TempleOperation, TemplePhase},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub struct TempleSessionOptions<'a> {
    pub window: &'a WindowInfo,
    pub capture: CaptureMethod,
    pub resources: &'a Path,
    pub fixture: bool,
}

pub struct TempleRun {
    pub id: Uuid,
    pub progress: DailyProgress,
    pub message: String,
    control: PipelineControl,
    steps: u32,
}
impl TempleRun {
    pub fn new(id: Uuid, progress: DailyProgress) -> Result<Self> {
        ensure!(progress.app_id == "suibian_temple", "不是随便观运行");
        progress
            .temple
            .as_ref()
            .context("缺少随便观冻结设置与检查点")?
            .validate()
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            id,
            progress,
            message: "随便观准备中".into(),
            control: PipelineControl::default(),
            steps: 0,
        })
    }
    pub fn control(&self) -> PipelineRequest {
        self.control.requester()
    }
    pub fn interrupt_requested(&self) -> Option<Interrupt> {
        self.control.requested()
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.progress.state == MailState::Paused && self.progress.inputs_released,
            "随便观尚未安全暂停"
        );
        let mut next = self.progress.clone();
        let flow = next.temple.as_mut().unwrap();
        flow.resume_navigation();
        if flow.current == Some(TempleOperation::AutoManage) {
            flow.hosting
                .as_mut()
                .unwrap()
                .resume()
                .map_err(anyhow::Error::msg)?;
        }
        if matches!(
            flow.current,
            Some(
                TempleOperation::AdventureClaimOnly
                    | TempleOperation::AdventureClaimAndDispatch
                    | TempleOperation::AdventureDispatchOnly
            )
        ) {
            flow.adventures
                .last_mut()
                .unwrap()
                .resume()
                .map_err(anyhow::Error::msg)?;
        }
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        next.stage = flow.stage();
        next.state = MailState::Running;
        self.progress = next;
        Ok(())
    }
    fn sync(&mut self) {
        self.progress.stage = self.progress.temple.as_ref().unwrap().stage();
    }
    fn interrupted(&mut self) -> bool {
        match self.control.requested() {
            Some(Interrupt::Stop) => {
                self.control.finish();
                self.progress.state = MailState::Stopped;
                self.message = "随便观已停止，保留已提交操作与结果".into();
                true
            }
            Some(Interrupt::Pause) => {
                if !self.control.acknowledge_pause() {
                    return self.interrupted();
                }
                self.progress.state = MailState::Paused;
                self.message = "随便观已暂停，输入已释放".into();
                true
            }
            None => false,
        }
    }
    fn publish(
        &mut self,
        started: Instant,
        initial: u64,
        publish: &mut impl FnMut(DailyProgress) -> Result<()>,
    ) -> Result<()> {
        self.sync();
        self.progress.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        publish(self.progress.clone())
    }
    pub fn execute(
        &mut self,
        slot: &mut Option<NativeSession>,
        options: TempleSessionOptions<'_>,
        mut publish: impl FnMut(DailyProgress) -> Result<()>,
    ) -> Result<()> {
        ensure!(
            self.progress.state == MailState::Running
                || (self.progress.state == MailState::Paused
                    && self.control.requested() == Some(Interrupt::Stop)),
            "随便观不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.progress.active_ms;
        let budget = u64::from(self.progress.timeout_ms());
        let remaining = || {
            Duration::from_millis(
                budget.saturating_sub(initial.saturating_add(started.elapsed().as_millis() as u64)),
            )
        };
        let result = (|| -> Result<()> {
            if let Some(session) = slot.as_mut() {
                session.quiesce()?;
            }
            self.progress.inputs_released = true;
            if self.interrupted() {
                return Ok(());
            }
            let bundle = maa_adapter::suibian_temple::validate_full_temple_bundle(
                &options.resources.join("generated/suibian-temple"),
            )?;
            if !options.fixture
                && NativeSession::handoff_game(
                    slot,
                    GameSessionConfig {
                        window: options.window,
                        resources: options.resources,
                        capture: options.capture,
                        input: InputMode::Foreground,
                        device: InferenceDevice::Cpu,
                    },
                    &self.control.cancellation(),
                )? == SessionHandoff::Cancelled
            {
                self.interrupted();
                return Ok(());
            }
            let session = slot.as_mut().context("随便观控制器未准备")?;
            session.load_suibian_temple_bundle(&bundle)?;
            if !options.fixture {
                session.prepare_game_mail(
                    self.control.cancellation(),
                    self.progress.config.start_delay_ms,
                )?;
            }
            loop {
                if self.interrupted() {
                    return Ok(());
                }
                if remaining().is_zero() {
                    self.progress.state = MailState::TimedOut;
                    self.message = "随便观达到总活动时间上限".into();
                    self.control.finish();
                    return Ok(());
                }
                ensure!(self.steps < 16384, "随便观总步骤达到上限");
                self.steps += 1;
                let phase = self.progress.temple.as_ref().unwrap().phase;
                match phase {
                    TemplePhase::Done => {
                        self.progress.completion_proven = true;
                        self.progress.returned_world = true;
                        self.sync();
                        ensure!(self.progress.completed(), "随便观完成证明不足");
                        self.progress.state = if self.control.finish() == Some(Interrupt::Stop) {
                            MailState::Stopped
                        } else {
                            MailState::Succeeded
                        };
                        self.message = "随便观流程完成，已返回大世界并释放输入".into();
                        return Ok(());
                    }
                    TemplePhase::Ready => {
                        self.progress
                            .temple
                            .as_mut()
                            .unwrap()
                            .begin_next()
                            .map_err(anyhow::Error::msg)?;
                        self.publish(started, initial, &mut publish)?;
                    }
                    TemplePhase::Child => {
                        let op = self.progress.temple.as_ref().unwrap().current.unwrap();
                        let control = self.control.clone();
                        // 子流程每次发布先更新并校验外层，再持久保存；失败不交给后继。
                        macro_rules! child {
                            ($module:ident, $run:ident, $field:ident) => {{
                                let flow = self
                                    .progress
                                    .temple
                                    .as_ref()
                                    .unwrap()
                                    .$field
                                    .as_ref()
                                    .unwrap();
                                let mut child =
                                    crate::$module::$run::new((**flow).clone(), control.clone())?;
                                child.execute_with_budget(
                                    session,
                                    !options.fixture,
                                    remaining(),
                                    |snap| {
                                        let mut next = self.progress.clone();
                                        next.temple.as_mut().unwrap().$field =
                                            Some(Box::new(snap.flow));
                                        next.inputs_released = snap.inputs_released;
                                        next.active_ms = initial
                                            .saturating_add(started.elapsed().as_millis() as u64);
                                        self.progress
                                            .temple
                                            .as_ref()
                                            .unwrap()
                                            .check_update(next.temple.as_ref().unwrap())
                                            .map_err(anyhow::Error::msg)?;
                                        publish(next.clone())?;
                                        self.progress = next;
                                        Ok(())
                                    },
                                )?
                            }};
                        }
                        // 单独生成各子流程的保存闭包，保留各自完整且不同的证明结构。
                        let end = match op {
                            TempleOperation::AutoManage => {
                                let mut child = crate::suibian_temple::HostingRun::new(
                                    self.progress.timeout_ms(),
                                    control,
                                )?;
                                child.snapshot.progress = self
                                    .progress
                                    .temple
                                    .as_ref()
                                    .unwrap()
                                    .hosting
                                    .clone()
                                    .unwrap();
                                child.execute_with_budget(
                                    session,
                                    !options.fixture,
                                    remaining(),
                                    |snap| {
                                        let mut next = self.progress.clone();
                                        next.temple.as_mut().unwrap().hosting = Some(snap.progress);
                                        next.inputs_released = snap.inputs_released;
                                        next.active_ms = initial
                                            .saturating_add(started.elapsed().as_millis() as u64);
                                        self.progress
                                            .temple
                                            .as_ref()
                                            .unwrap()
                                            .check_update(next.temple.as_ref().unwrap())
                                            .map_err(anyhow::Error::msg)?;
                                        publish(next.clone())?;
                                        self.progress = next;
                                        Ok(())
                                    },
                                )?
                            }
                            TempleOperation::AdventureClaimOnly
                            | TempleOperation::AdventureClaimAndDispatch
                            | TempleOperation::AdventureDispatchOnly => {
                                let mut child = crate::suibian_adventure::AdventureRun::new(
                                    self.progress
                                        .temple
                                        .as_ref()
                                        .unwrap()
                                        .adventures
                                        .last()
                                        .unwrap()
                                        .clone(),
                                    control,
                                )?;
                                child.execute_with_budget(
                                    session,
                                    !options.fixture,
                                    remaining(),
                                    |snap| {
                                        let mut next = self.progress.clone();
                                        *next
                                            .temple
                                            .as_mut()
                                            .unwrap()
                                            .adventures
                                            .last_mut()
                                            .unwrap() = snap.flow;
                                        next.inputs_released = snap.inputs_released;
                                        next.active_ms = initial
                                            .saturating_add(started.elapsed().as_millis() as u64);
                                        self.progress
                                            .temple
                                            .as_ref()
                                            .unwrap()
                                            .check_update(next.temple.as_ref().unwrap())
                                            .map_err(anyhow::Error::msg)?;
                                        publish(next.clone())?;
                                        self.progress = next;
                                        Ok(())
                                    },
                                )?
                            }
                            TempleOperation::YumChaSin => child!(suibian_yum, YumRun, yum),
                            _ => self.run_simple_child(
                                op,
                                session,
                                !options.fixture,
                                remaining(),
                                started,
                                initial,
                                &mut publish,
                            )?,
                        };
                        if end != HostingEnd::Completed {
                            if !self.interrupted() {
                                self.progress.state = if end == HostingEnd::TimedOut {
                                    MailState::TimedOut
                                } else {
                                    MailState::Failed
                                };
                                self.message = format!("随便观子流程结束：{end:?}");
                                self.control.finish();
                            }
                            return Ok(());
                        }
                        self.progress
                            .temple
                            .as_mut()
                            .unwrap()
                            .finish_child(op, true, self.progress.inputs_released)
                            .map_err(anyhow::Error::msg)?;
                        self.publish(started, initial, &mut publish)?;
                    }
                    TemplePhase::Entering | TemplePhase::Returning => {
                        self.sync();
                        let stage = self.progress.stage.clone();
                        if stage == "ConfirmTransport"
                            && !self.progress.temple.as_ref().unwrap().transport_attempted
                        {
                            self.progress
                                .temple
                                .as_mut()
                                .unwrap()
                                .mark_transport()
                                .map_err(anyhow::Error::msg)?;
                        }
                        self.publish(started, initial, &mut publish)?;
                        if stage == "ConfirmTransport"
                            && (self.control.requested().is_some() || remaining().is_zero())
                        {
                            self.progress
                                .temple
                                .as_mut()
                                .unwrap()
                                .transport_not_sent()
                                .map_err(anyhow::Error::msg)?;
                            self.publish(started, initial, &mut publish)?;
                        }
                        if self.interrupted() {
                            return Ok(());
                        }
                        if remaining().is_zero() {
                            continue;
                        }
                        let step = session.temple_navigation_step(
                            &stage,
                            self.control.cancellation(),
                            remaining().min(Duration::from_secs(60)),
                            !options.fixture,
                        )?;
                        ensure!(step.report.released, "随便观导航尚未释放输入");
                        if stage == "ConfirmTransport" && step.task_id.is_none() {
                            self.progress
                                .temple
                                .as_mut()
                                .unwrap()
                                .transport_not_sent()
                                .map_err(anyhow::Error::msg)?;
                        }
                        self.progress.inputs_released = true;
                        match step.report.end {
                            PipelineEnd::Succeeded => self
                                .progress
                                .temple
                                .as_mut()
                                .unwrap()
                                .observe_navigation(
                                    &step
                                        .nodes
                                        .iter()
                                        .rev()
                                        .find(|n| n.completed)
                                        .context("随便观缺少导航完成节点")?
                                        .name,
                                )
                                .map_err(anyhow::Error::msg)?,
                            PipelineEnd::Cancelled => {
                                ensure!(self.control.requested().is_some(), "随便观收到未登记取消");
                            }
                            PipelineEnd::TimedOut => {
                                anyhow::bail!("随便观入口或返回步骤超时");
                            }
                            PipelineEnd::Failed => anyhow::bail!("随便观入口或返回步骤失败"),
                        }
                        self.publish(started, initial, &mut publish)?;
                    }
                }
            }
        })();
        let released = slot.as_mut().map_or(Ok(()), NativeSession::quiesce);
        self.progress.inputs_released = released.is_ok();
        self.sync();
        self.progress.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        let result = result.and(released);
        if let Err(error) = &result {
            self.control.finish();
            self.progress.state = if self.progress.active_ms >= budget {
                MailState::TimedOut
            } else {
                MailState::Failed
            };
            self.message = format!("随便观执行失败：{error:#}");
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn run_simple_child(
        &mut self,
        op: TempleOperation,
        session: &mut NativeSession,
        game: bool,
        remaining: Duration,
        started: Instant,
        initial: u64,
        publish: &mut impl FnMut(DailyProgress) -> Result<()>,
    ) -> Result<HostingEnd> {
        macro_rules! run {
            ($module:ident, $run:ident, $field:ident) => {{
                let mut child = crate::$module::$run::new(
                    self.progress
                        .temple
                        .as_ref()
                        .unwrap()
                        .$field
                        .clone()
                        .unwrap(),
                    self.control.clone(),
                )?;
                child.execute_with_budget(session, game, remaining, |snap| {
                    let mut next = self.progress.clone();
                    next.temple.as_mut().unwrap().$field = Some(snap.flow);
                    next.inputs_released = snap.inputs_released;
                    next.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
                    self.progress
                        .temple
                        .as_ref()
                        .unwrap()
                        .check_update(next.temple.as_ref().unwrap())
                        .map_err(anyhow::Error::msg)?;
                    publish(next.clone())?;
                    self.progress = next;
                    Ok(())
                })
            }};
        }
        match op {
            TempleOperation::Craft => run!(suibian_craft, CraftRun, craft),
            TempleOperation::Sales => run!(suibian_sales, SalesRun, sales),
            TempleOperation::GoodGoods => run!(suibian_good_goods, GoodGoodsRun, good_goods),
            TempleOperation::BooBox => run!(suibian_boo_box, BooBoxRun, boo_box),
            _ => anyhow::bail!("不是普通随便观子流程"),
        }
    }
}
