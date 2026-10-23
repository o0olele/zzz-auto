//! 父副本调用的卡片准备，不终结共享控制句柄或写入运行次数。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, simulation::CardsInputEnd};
use od_core::{
    pipeline::{Interrupt, PipelineControl},
    simulation_cards::{CardsFlow, SimulationView},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardsEnd {
    Prepared,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardsSnapshot {
    pub flow: CardsFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct CardsRun {
    pub snapshot: CardsSnapshot,
    control: PipelineControl,
    end: Option<CardsEnd>,
    steps: u32,
}
impl CardsRun {
    pub fn new(flow: CardsFlow, control: PipelineControl) -> Result<Self> {
        flow.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: CardsSnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
            },
            control,
            end: None,
            steps: 0,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(CardsEnd::Paused) && self.snapshot.inputs_released,
            "卡片准备尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<CardsEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => CardsEnd::Paused,
            Interrupt::Stop => CardsEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(CardsSnapshot) -> Result<()>,
    ) -> Result<CardsEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(CardsEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "卡片子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<CardsEnd> {
            session.require_simulation()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(CardsEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(CardsEnd::Prepared);
                }
                ensure!(self.steps < 4096, "卡片准备步骤达到上限");
                self.steps += 1;
                let read = session.read_simulation(&self.control.cancellation(), remaining());
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "卡片读帧后尚未释放输入");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(CardsEnd::TimedOut);
                }
                let Some(view) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                ensure!(
                    !matches!(
                        view,
                        SimulationView::Deployment
                            | SimulationView::Settlement
                            | SimulationView::Reward
                    ),
                    "卡片准备已离开副本详情，不能继续调整"
                );
                let action = self.snapshot.flow.next(&view).map_err(anyhow::Error::msg)?;
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                let Some(action) = action else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let before = self.snapshot.flow.clone();
                self.snapshot
                    .flow
                    .prepare(action)
                    .map_err(anyhow::Error::msg)?;
                if let Err(error) = publish(self.snapshot.clone()) {
                    self.snapshot.flow = before;
                    return Err(error);
                }
                if remaining().is_zero() || self.interrupted().is_some() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(CardsEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_simulation_cards(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "卡片输入后尚未释放输入");
                match input {
                    Ok(CardsInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(CardsInputEnd::NotPosted | CardsInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(CardsInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(CardsEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(CardsEnd::TimedOut);
                        }
                        return Err(error);
                    }
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        if result.is_err() || !self.snapshot.inputs_released {
            self.end = Some(CardsEnd::Failed);
        }
        ensure!(self.snapshot.inputs_released, "卡片收尾无法确认释放输入");
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(CardsEnd::Failed);
            return Err(error);
        }
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        }
        if end == CardsEnd::Paused && !self.control.acknowledge_pause() {
            end = CardsEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
