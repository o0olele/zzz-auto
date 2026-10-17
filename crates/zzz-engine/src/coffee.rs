use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, PipelineEnd, PipelineExecution};
use od_core::{
    coffee_approach::{CoffeeApproachPhase, CoffeeApproachProgress},
    coffee_run::{CoffeeConfig, CoffeePhase, CoffeeProgress, CoffeeState},
    pipeline::{Interrupt, PipelineControl, PipelineRequest},
};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub struct CoffeeRun {
    pub id: Uuid,
    pub progress: CoffeeProgress,
    pub message: String,
    pub executions: Vec<PipelineExecution>,
    control: PipelineControl,
}
impl CoffeeRun {
    pub fn with_id(id: Uuid, config: CoffeeConfig) -> Result<Self> {
        config.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            id,
            progress: CoffeeProgress::new(config),
            message: "咖啡试运行已创建".into(),
            executions: Vec::new(),
            control: PipelineControl::default(),
        })
    }
    pub fn control(&self) -> PipelineRequest {
        self.control.requester()
    }
    pub fn from_world(id: Uuid, config: CoffeeConfig) -> Result<Self> {
        let mut run = Self::with_id(id, config)?;
        run.progress.approach = Some(CoffeeApproachProgress::default());
        Ok(run)
    }
    pub fn interrupt_requested(&self) -> Option<Interrupt> {
        self.control.requested()
    }
    pub fn acknowledge_idle(&mut self) {
        self.progress.inputs_released = true;
        self.interrupt();
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.progress.state == CoffeeState::Paused && self.progress.inputs_released,
            "只有已释放输入的咖啡任务可以继续"
        );
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.progress.state = CoffeeState::Running;
        Ok(())
    }
    fn terminal(&mut self, state: CoffeeState, message: &str) {
        let stopped = self.control.finish() == Some(Interrupt::Stop);
        self.progress.state = if stopped { CoffeeState::Stopped } else { state };
        self.message = if stopped {
            "咖啡流程已停止，输入已释放".into()
        } else {
            message.into()
        };
    }
    fn interrupt(&mut self) -> bool {
        match self.control.requested() {
            Some(Interrupt::Stop) => {
                self.terminal(CoffeeState::Stopped, "咖啡流程已停止，输入已释放");
                true
            }
            Some(Interrupt::Pause) => {
                if !self.control.acknowledge_pause() {
                    return self.interrupt();
                }
                self.progress.state = CoffeeState::Paused;
                self.message = "咖啡流程已暂停，输入已释放".into();
                true
            }
            None => false,
        }
    }
    pub fn execute(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        mut observer: impl FnMut(CoffeeProgress),
    ) -> Result<()> {
        ensure!(
            self.progress.state == CoffeeState::Running
                || (self.progress.state == CoffeeState::Paused
                    && self.control.requested() == Some(Interrupt::Stop)),
            "咖啡任务已暂停或结束"
        );
        self.progress.inputs_released = false;
        observer(self.progress.clone());
        let preparation = if self.progress.approach.is_some() {
            session.prepare_coffee_approach(
                game,
                self.control.cancellation(),
                self.progress.config.start_delay_ms,
            )
        } else if game {
            session.prepare_game_coffee(
                self.control.cancellation(),
                self.progress.config.start_delay_ms,
            )
        } else {
            session.prepare_fixture_coffee()
        };
        let started = Instant::now();
        let result = preparation.and_then(|()| {
            self.progress.inputs_released = true;
            self.execute_inner(session, game, started, &mut observer)
        });
        self.progress.active_ms = self
            .progress
            .active_ms
            .saturating_add(started.elapsed().as_millis() as u64);
        if let Err(error) = &result {
            self.control.finish();
            self.progress.state = CoffeeState::Failed;
            self.message = format!("咖啡流程或输入收尾失败：{error:#}");
        }
        result
    }
    fn execute_inner(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        started: Instant,
        observer: &mut impl FnMut(CoffeeProgress),
    ) -> Result<()> {
        loop {
            if self.interrupt() {
                return Ok(());
            }
            let used = self
                .progress
                .active_ms
                .saturating_add(started.elapsed().as_millis() as u64);
            if used >= u64::from(self.progress.config.timeout_ms) {
                self.terminal(
                    CoffeeState::TimedOut,
                    "咖啡流程超时，输入已释放，不重新点单",
                );
                return Ok(());
            }
            if self
                .progress
                .approach
                .as_ref()
                .is_some_and(|p| p.phase != CoffeeApproachPhase::Done)
            {
                let end = self.execute_approach_step(
                    session,
                    game,
                    Duration::from_millis(u64::from(self.progress.config.timeout_ms) - used),
                    observer,
                )?;
                let mut progress = self.progress.clone();
                progress.active_ms = used;
                observer(progress);
                if self.interrupt() {
                    return Ok(());
                }
                match end {
                    PipelineEnd::Succeeded => continue,
                    PipelineEnd::TimedOut => self.terminal(
                        CoffeeState::TimedOut,
                        "咖啡前置步骤超时，输入已释放，不重复传送或交互",
                    ),
                    _ => self.terminal(CoffeeState::Failed, "咖啡前置步骤未完成，输入已释放"),
                }
                return Ok(());
            }
            let phase = self.progress.phase;
            let entry = match phase {
                CoffeePhase::Inspect => "Coffee.Inspect",
                CoffeePhase::Order => {
                    self.progress.order_attempted = true;
                    self.progress.phase = CoffeePhase::WaitEnergy;
                    "Coffee.Order"
                }
                CoffeePhase::WaitEnergy => "Coffee.WaitEnergy",
                CoffeePhase::ConfirmEnergy => {
                    self.progress.phase = CoffeePhase::AfterEnergy;
                    "Coffee.ConfirmEnergy"
                }
                CoffeePhase::AfterEnergy => "Coffee.AfterEnergy",
                CoffeePhase::ConfirmBuff => {
                    self.progress.phase = CoffeePhase::AfterEnergy;
                    "Coffee.ConfirmBuff"
                }
                CoffeePhase::Done => {
                    ensure!(
                        self.progress.order_attempted
                            && self.progress.energy_seen
                            && self.progress.inputs_released,
                        "咖啡完成证明不足"
                    );
                    self.terminal(
                        CoffeeState::Succeeded,
                        "咖啡已确认电量并返回大世界，输入已释放",
                    );
                    return Ok(());
                }
            };
            self.progress.inputs_released = false;
            let mut progress = self.progress.clone();
            progress.active_ms = used;
            observer(progress);
            let execution = session.coffee_step(
                entry,
                self.progress.config,
                self.control.cancellation(),
                Duration::from_millis(u64::from(self.progress.config.timeout_ms) - used),
                game,
            )?;
            self.progress.inputs_released = execution.report.released;
            ensure!(self.progress.inputs_released, "咖啡输入尚未释放");
            let completed = |name: &str| {
                execution
                    .nodes
                    .iter()
                    .any(|node| node.name == name && node.completed)
            };
            if self.progress.order_attempted && completed("Coffee.Energy") {
                self.progress.energy_seen = true;
            }
            if execution.report.end == PipelineEnd::Succeeded {
                self.progress.phase = match phase {
                    CoffeePhase::Inspect if completed("Coffee.Options") => CoffeePhase::Order,
                    CoffeePhase::Inspect => {
                        self.executions.push(execution);
                        self.terminal(
                            CoffeeState::Failed,
                            "起点须为咖啡点单选项，不能接管已有奖励或增益弹窗",
                        );
                        return Ok(());
                    }
                    CoffeePhase::Order | CoffeePhase::WaitEnergy => CoffeePhase::ConfirmEnergy,
                    CoffeePhase::ConfirmEnergy | CoffeePhase::ConfirmBuff => {
                        CoffeePhase::AfterEnergy
                    }
                    CoffeePhase::AfterEnergy if completed("Coffee.World") => CoffeePhase::Done,
                    CoffeePhase::AfterEnergy if completed("Coffee.Buff") => {
                        CoffeePhase::ConfirmBuff
                    }
                    CoffeePhase::AfterEnergy => CoffeePhase::ConfirmEnergy,
                    CoffeePhase::Done => unreachable!(),
                };
            }
            let end = execution.report.end;
            self.executions.push(execution);
            observer(self.progress.clone());
            if self.interrupt() {
                return Ok(());
            }
            match end {
                PipelineEnd::Succeeded => {}
                PipelineEnd::TimedOut => {
                    self.terminal(
                        CoffeeState::TimedOut,
                        "咖啡步骤超时，输入已释放，不重新点单",
                    );
                    return Ok(());
                }
                _ => {
                    self.terminal(
                        CoffeeState::Failed,
                        "咖啡步骤未完成，输入已释放，不重新点单",
                    );
                    return Ok(());
                }
            }
        }
    }

    fn execute_approach_step(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        timeout: Duration,
        observer: &mut impl FnMut(CoffeeProgress),
    ) -> Result<PipelineEnd> {
        use CoffeeApproachPhase::*;
        let approach = self.progress.approach.as_mut().expect("咖啡前置进度");
        let phase = approach.phase;
        let entry = match phase {
            InspectWorld => "CoffeeApproach.InspectWorld",
            OpenMap => {
                approach.phase = WaitMap;
                "CoffeeApproach.OpenMap"
            }
            WaitMap => "CoffeeApproach.WaitMap",
            SelectArea => "Map.SelectArea",
            SelectPoint => {
                approach.phase = WaitDialog;
                "Map.SelectPoint"
            }
            WaitDialog => "Map.WaitDialog",
            Confirm => {
                approach.phase = WaitArrival;
                "Map.Confirm"
            }
            WaitArrival => "Map.WaitArrival",
            Interact => {
                approach.phase = WaitDialogue;
                "CoffeeApproach.Interact"
            }
            WaitDialogue => "CoffeeApproach.WaitDialogue",
            AdvanceDialogue if approach.dialogue_advances < 12 => {
                approach.dialogue_advances += 1;
                "CoffeeApproach.AdvanceDialogue"
            }
            AdvanceDialogue => "CoffeeApproach.WaitOptions",
            Done => unreachable!(),
        };
        self.progress.inputs_released = false;
        observer(self.progress.clone());
        let execution = session.coffee_approach_step(
            entry,
            self.progress.config,
            self.control.cancellation(),
            timeout,
            game,
        )?;
        self.progress.inputs_released = execution.report.released;
        let end = execution.report.end;
        let completed = |name: &str| {
            execution
                .nodes
                .iter()
                .any(|n| n.name == name && n.completed)
        };
        let next = if end == PipelineEnd::Succeeded {
            match phase {
                InspectWorld => {
                    ensure!(
                        completed("Map.World"),
                        "咖啡自动路线须从大世界开始，不接管已有地图、对话或奖励"
                    );
                    OpenMap
                }
                OpenMap | WaitMap => SelectArea,
                SelectArea => SelectPoint,
                SelectPoint | WaitDialog => Confirm,
                Confirm => WaitArrival,
                WaitArrival => Interact,
                Interact | WaitDialogue if completed("Coffee.Options") => Done,
                Interact | WaitDialogue => AdvanceDialogue,
                AdvanceDialogue if completed("Coffee.Options") => Done,
                AdvanceDialogue => {
                    ensure!(
                        !completed("Map.World"),
                        "咖啡对话已退出，本次未点单，不记录领取成功"
                    );
                    AdvanceDialogue
                }
                Done => unreachable!(),
            }
        } else if phase == SelectArea && completed("Map.ClickArea") {
            SelectPoint
        } else {
            self.progress.approach.as_ref().expect("咖啡前置进度").phase
        };
        self.executions.push(execution);
        ensure!(self.progress.inputs_released, "咖啡前置输入尚未释放");
        let approach = self.progress.approach.as_mut().expect("咖啡前置进度");
        approach.phase = next;
        Ok(end)
    }
}
