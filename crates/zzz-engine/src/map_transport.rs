//! 咖啡前置路线的检查点；支持离线及受控实机试运行，不登记咖啡成功。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, PipelineEnd, PipelineExecution};
pub use od_core::map_transport::MapState;
use od_core::{
    map_transport::{MapConfig, MapPhase, MapProgress, MapStep},
    pipeline::{Interrupt, PipelineControl, PipelineRequest},
};
use serde::Serialize;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Serialize)]
pub struct MapRun {
    pub id: Uuid,
    config: MapConfig,
    pub state: MapState,
    pub phase: MapPhase,
    pub point_attempted: bool,
    pub confirm_attempted: bool,
    pub inputs_released: bool,
    pub active_ms: u64,
    pub message: String,
    pub executions: Vec<PipelineExecution>,
    timeout_ms: u64,
    #[serde(skip)]
    control: PipelineControl,
}

impl MapRun {
    pub fn new(timeout: Duration) -> Result<Self> {
        ensure!(
            (Duration::from_millis(1)..=Duration::from_secs(60)).contains(&timeout),
            "地图超时须为 1ms..60s"
        );
        Self::with_id(
            Uuid::new_v4(),
            MapConfig {
                timeout_ms: timeout.as_millis() as u32,
                start_delay_ms: 0,
            },
        )
    }

    pub fn with_id(id: Uuid, config: MapConfig) -> Result<Self> {
        config.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            id,
            config,
            state: MapState::Running,
            phase: MapPhase::Inspect,
            point_attempted: false,
            confirm_attempted: false,
            inputs_released: true,
            active_ms: 0,
            message: "地图试运行已创建".into(),
            executions: Vec::new(),
            timeout_ms: config.timeout_ms.into(),
            control: PipelineControl::default(),
        })
    }

    pub fn control(&self) -> PipelineRequest {
        self.control.requester()
    }

    pub fn interrupt_requested(&self) -> Option<Interrupt> {
        self.control.requested()
    }
    pub fn acknowledge_idle(&mut self) {
        self.inputs_released = true;
        self.interrupt();
    }
    pub fn progress(&self) -> MapProgress {
        MapProgress {
            config: self.config,
            state: self.state,
            phase: self.phase,
            point_attempted: self.point_attempted,
            confirm_attempted: self.confirm_attempted,
            inputs_released: self.inputs_released,
            active_ms: self.active_ms,
        }
    }

    fn progress_at(&self, started: Instant) -> MapProgress {
        let mut progress = self.progress();
        progress.active_ms = self
            .active_ms
            .saturating_add(started.elapsed().as_millis() as u64);
        progress
    }

    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.state == MapState::Paused && self.inputs_released,
            "只有已释放输入的暂停地图任务可以继续"
        );
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.state = MapState::Running;
        self.message = "重新识别地图检查点".into();
        Ok(())
    }

    fn interrupt(&mut self) -> bool {
        match self.control.requested() {
            Some(Interrupt::Stop) => {
                self.terminal(MapState::Stopped, "地图流程已停止，输入已释放");
                true
            }
            Some(Interrupt::Pause) => {
                if !self.control.acknowledge_pause() {
                    return self.interrupt();
                }
                self.state = MapState::Paused;
                self.message = "地图流程已暂停，输入已释放".into();
                true
            }
            None => false,
        }
    }

    fn terminal(&mut self, state: MapState, message: &str) {
        let stop = self.control.finish() == Some(Interrupt::Stop);
        self.state = if stop { MapState::Stopped } else { state };
        self.message = if stop {
            "地图流程已停止，输入已释放".into()
        } else {
            message.into()
        };
    }

    pub fn execute_fixture(&mut self, session: &mut NativeSession) -> Result<()> {
        self.execute_fixture_observed(session, |_| {})
    }

    pub fn execute_fixture_observed(
        &mut self,
        session: &mut NativeSession,
        progress: impl FnMut(MapProgress),
    ) -> Result<()> {
        self.execute_observed(session, false, progress)
    }

    pub fn execute_game(
        &mut self,
        session: &mut NativeSession,
        progress: impl FnMut(MapProgress),
    ) -> Result<()> {
        self.execute_observed(session, true, progress)
    }

    fn execute_observed(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        mut progress: impl FnMut(MapProgress),
    ) -> Result<()> {
        ensure!(
            self.state == MapState::Running
                || (self.state == MapState::Paused
                    && self.control.requested() == Some(Interrupt::Stop)),
            "地图任务已暂停或结束，不能直接重跑"
        );
        self.inputs_released = false;
        progress(self.progress());
        let preparation = if game {
            session.prepare_game_map(self.control.cancellation(), self.config.start_delay_ms)
        } else {
            session.prepare_fixture_map()
        };
        let started = Instant::now();
        let result = preparation.and_then(|()| {
            self.inputs_released = true;
            self.execute_inner(session, started, game, &mut progress)
        });
        self.active_ms = self
            .active_ms
            .saturating_add(started.elapsed().as_millis() as u64);
        if let Err(error) = &result {
            self.control.finish();
            self.state = MapState::Failed;
            self.message = format!("地图流程或输入收尾失败：{error:#}");
        }
        result
    }

    fn execute_inner(
        &mut self,
        session: &mut NativeSession,
        started: Instant,
        game: bool,
        progress: &mut impl FnMut(MapProgress),
    ) -> Result<()> {
        loop {
            if self.interrupt() {
                return Ok(());
            }
            let used = self
                .active_ms
                .saturating_add(started.elapsed().as_millis() as u64);
            if used >= self.timeout_ms {
                self.terminal(MapState::TimedOut, "地图流程超时，输入已释放");
                return Ok(());
            }
            let phase = self.phase;
            let step = match phase {
                MapPhase::Inspect => MapStep::Inspect,
                MapPhase::SelectArea => MapStep::SelectArea,
                MapPhase::SelectPoint => {
                    // 可能已经点击但尚未收到回执，后续只等待弹窗，不重新选择。
                    self.point_attempted = true;
                    self.phase = MapPhase::InspectDialog;
                    MapStep::SelectPoint
                }
                MapPhase::InspectDialog => MapStep::WaitDialog,
                MapPhase::Confirm => {
                    self.confirm_attempted = true;
                    self.phase = MapPhase::WaitArrival;
                    MapStep::Confirm
                }
                MapPhase::WaitArrival => MapStep::WaitArrival,
                MapPhase::Done => {
                    self.terminal(
                        MapState::Succeeded,
                        "传送流程已识别大世界并释放输入；请核对实际落地位置",
                    );
                    return Ok(());
                }
            };
            self.inputs_released = false;
            progress(self.progress_at(started));
            let remaining = Duration::from_millis(self.timeout_ms - used);
            let execution = if game {
                session.game_map_step(step, self.control.cancellation(), remaining)
            } else {
                session.fixture_map_step(step, self.control.cancellation(), remaining)
            }?;
            self.inputs_released = execution.report.released;
            ensure!(self.inputs_released, "地图输入尚未释放");
            let completed = |name: &str| {
                execution
                    .nodes
                    .iter()
                    .any(|node| node.name == name && node.completed)
            };
            if phase == MapPhase::SelectArea && completed("Map.ClickArea") {
                self.phase = MapPhase::SelectPoint;
            }
            if execution.report.end == PipelineEnd::Succeeded {
                self.phase = match phase {
                    MapPhase::Inspect if completed("Map.Area") || completed("Map.Point") => {
                        MapPhase::SelectArea
                    }
                    MapPhase::Inspect => {
                        self.executions.push(execution);
                        self.terminal(
                            MapState::Failed,
                            "起点必须是普通地图列表，不能接管已有传送弹窗或从大世界猜测完成",
                        );
                        return Ok(());
                    }
                    MapPhase::SelectArea => MapPhase::SelectPoint,
                    MapPhase::SelectPoint | MapPhase::InspectDialog => MapPhase::Confirm,
                    MapPhase::Confirm => MapPhase::WaitArrival,
                    MapPhase::WaitArrival => MapPhase::Done,
                    MapPhase::Done => unreachable!(),
                };
            }
            let end = execution.report.end;
            self.executions.push(execution);
            progress(self.progress_at(started));
            if self.interrupt() {
                return Ok(());
            }
            match end {
                PipelineEnd::Succeeded => {}
                PipelineEnd::TimedOut => {
                    self.terminal(
                        MapState::TimedOut,
                        "地图步骤超时，输入已释放，不重复投递传送",
                    );
                    return Ok(());
                }
                PipelineEnd::Failed | PipelineEnd::Cancelled => {
                    self.terminal(
                        MapState::Failed,
                        "地图步骤未完成，输入已释放，不重复投递传送",
                    );
                    return Ok(());
                }
            }
        }
    }
}
