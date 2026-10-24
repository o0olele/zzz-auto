//! 模拟室卡片和电量恢复共用父控制器；没有关卡选中证明时不能据此出战。
use crate::{
    restore_charge::{RestoreEnd, RestoreRun},
    simulation_cards::{CardsEnd, CardsRun},
};
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, RecognitionImage, simulation::CardsInputEnd};
use od_core::{
    pipeline::{Interrupt, PipelineControl},
    restore_charge::{RestoreReturn, RestoreView},
    simulation_cards::{CardsCharge, SimulationView},
    simulation_preparation::{PreparationFlow, PreparationStage as Stage},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparationEnd {
    Completed,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationSnapshot {
    pub flow: PreparationFlow,
    pub active_ms: u64,
    pub cards_active_ms: u64,
    pub restore_active_ms: u64,
    pub inputs_released: bool,
}
pub struct PreparationRun {
    pub snapshot: PreparationSnapshot,
    control: PipelineControl,
    end: Option<PreparationEnd>,
    steps: u32,
}
impl PreparationRun {
    pub fn new(flow: PreparationFlow, control: PipelineControl) -> Result<Self> {
        flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            !flow.completed(),
            "已完成的准备不能重用旧电量证明启动新任务"
        );
        Ok(Self {
            snapshot: PreparationSnapshot {
                flow,
                active_ms: 0,
                cards_active_ms: 0,
                restore_active_ms: 0,
                inputs_released: true,
            },
            control,
            end: None,
            steps: 0,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(PreparationEnd::Paused) && self.snapshot.inputs_released,
            "模拟室准备尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<PreparationEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => PreparationEnd::Paused,
            Interrupt::Stop => PreparationEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(PreparationSnapshot) -> Result<()>,
    ) -> Result<PreparationEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(PreparationEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "模拟室准备不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<PreparationEnd> {
            // 卡片输入前即校验全部子流程资源，不能在方案改变后才发现恢复包不兼容。
            session.require_simulation()?;
            session.require_restore_charge()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(PreparationEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(PreparationEnd::Completed);
                }
                ensure!(self.steps < 4096, "模拟室准备步骤达到上限");
                self.steps += 1;
                match self.snapshot.flow.stage() {
                    Stage::Cards => {
                        let mut child = CardsRun::new(
                            self.snapshot.flow.cards().clone(),
                            self.control.clone(),
                        )?;
                        child.snapshot.active_ms = self.snapshot.cards_active_ms;
                        let result =
                            child.execute_with_budget(session, game, remaining(), |snapshot| {
                                self.snapshot
                                    .flow
                                    .update_cards(snapshot.flow)
                                    .map_err(anyhow::Error::msg)?;
                                self.snapshot.cards_active_ms = snapshot.active_ms;
                                self.snapshot.inputs_released = snapshot.inputs_released;
                                self.snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                publish(self.snapshot.clone())
                            });
                        self.snapshot
                            .flow
                            .update_cards(child.snapshot.flow)
                            .map_err(anyhow::Error::msg)?;
                        self.snapshot.cards_active_ms = child.snapshot.active_ms;
                        self.snapshot.inputs_released = child.snapshot.inputs_released;
                        let result = result?;
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(PreparationEnd::TimedOut);
                        }
                        match result {
                            CardsEnd::Prepared => self
                                .snapshot
                                .flow
                                .cards_finished()
                                .map_err(anyhow::Error::msg)?,
                            CardsEnd::Paused => return Ok(PreparationEnd::Paused),
                            CardsEnd::Stopped => return Ok(PreparationEnd::Stopped),
                            CardsEnd::TimedOut => return Ok(PreparationEnd::TimedOut),
                            CardsEnd::Failed => anyhow::bail!("卡片子流程失败"),
                        }
                        self.snapshot.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        publish(self.snapshot.clone())?;
                    }
                    Stage::Inspect | Stage::Verify => {
                        let view =
                            session.read_simulation(&self.control.cancellation(), remaining());
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(PreparationEnd::TimedOut);
                        }
                        let Some(view) = view? else {
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        };
                        let charge = match view {
                            SimulationView::Challenge {
                                current_charge,
                                required_charge,
                            } => CardsCharge {
                                current_charge,
                                required_charge,
                            },
                            SimulationView::Unknown | SimulationView::Blocked => {
                                std::thread::sleep(Duration::from_millis(10));
                                continue;
                            }
                            _ => anyhow::bail!("模拟室准备已离开副本详情"),
                        };
                        let open = self
                            .snapshot
                            .flow
                            .observe_details(charge)
                            .map_err(anyhow::Error::msg)?;
                        self.snapshot.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        publish(self.snapshot.clone())?;
                        if !open {
                            continue;
                        }
                        let before = self.snapshot.flow.clone();
                        self.snapshot
                            .flow
                            .prepare_open()
                            .map_err(anyhow::Error::msg)?;
                        if let Err(error) = publish(self.snapshot.clone()) {
                            self.snapshot.flow = before;
                            return Err(error);
                        }
                        if self.interrupted().is_some() || remaining().is_zero() {
                            self.snapshot
                                .flow
                                .not_posted_open()
                                .map_err(anyhow::Error::msg)?;
                            publish(self.snapshot.clone())?;
                            return Ok(self.interrupted().unwrap_or(PreparationEnd::TimedOut));
                        }
                        self.snapshot.inputs_released = false;
                        let input = session.act_simulation_restore_open(
                            &self.snapshot.flow,
                            &self.control.cancellation(),
                            remaining(),
                            game,
                        );
                        self.snapshot.inputs_released = session.release_inputs().is_ok();
                        ensure!(self.snapshot.inputs_released, "打开恢复弹窗后尚未释放输入");
                        match input {
                            Ok(CardsInputEnd::Posted) => self
                                .snapshot
                                .flow
                                .posted_open()
                                .map_err(anyhow::Error::msg)?,
                            Ok(CardsInputEnd::NotPosted | CardsInputEnd::Changed) => self
                                .snapshot
                                .flow
                                .not_posted_open()
                                .map_err(anyhow::Error::msg)?,
                            Ok(CardsInputEnd::TimedOut) => {
                                self.snapshot
                                    .flow
                                    .not_posted_open()
                                    .map_err(anyhow::Error::msg)?;
                                return Ok(PreparationEnd::TimedOut);
                            }
                            Err(error) => {
                                // 原生错误不证明未发送；保留下一步意图，暂停后只等待实际主弹窗。
                                if let Some(end) = self.interrupted() {
                                    return Ok(end);
                                }
                                if remaining().is_zero() {
                                    return Ok(PreparationEnd::TimedOut);
                                }
                                return Err(error);
                            }
                        }
                        self.snapshot.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        publish(self.snapshot.clone())?;
                    }
                    Stage::AwaitPopup => {
                        // 同一新帧上复用恢复识别和父详情识别；详情可读仍不证明弹窗已经打开。
                        let mut observed = None;
                        let view = session.read_restore_charge(
                            &self.control.cancellation(),
                            remaining(),
                            &mut |s, image, cancel, timeout| {
                                let view = s.inspect_simulation(image, cancel, timeout)?;
                                observed = view.clone();
                                Ok(match view {
                                    Some(SimulationView::Challenge {
                                        current_charge,
                                        required_charge,
                                    }) => Some(RestoreReturn {
                                        current_charge,
                                        required_charge,
                                    }),
                                    _ => None,
                                })
                            },
                        );
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(PreparationEnd::TimedOut);
                        }
                        match view? {
                            Some(RestoreView::Sources { .. }) => {
                                self.snapshot
                                    .flow
                                    .start_restore()
                                    .map_err(anyhow::Error::msg)?;
                                self.snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                publish(self.snapshot.clone())?;
                            }
                            Some(RestoreView::QuickUse { .. } | RestoreView::Reward { .. }) => {
                                anyhow::bail!("下一步后出现不属于本次恢复的子弹窗")
                            }
                            _ => {
                                ensure!(
                                    observed.is_none()
                                        || matches!(
                                            observed,
                                            Some(
                                                SimulationView::Unknown
                                                    | SimulationView::Blocked
                                                    | SimulationView::Challenge { .. }
                                            )
                                        ),
                                    "打开恢复弹窗后意外离开副本详情"
                                );
                                std::thread::sleep(Duration::from_millis(10));
                            }
                        }
                    }
                    Stage::Restore => {
                        let mut child = RestoreRun::new(
                            self.snapshot
                                .flow
                                .restore()
                                .ok_or_else(|| anyhow::anyhow!("缺少恢复检查点"))?
                                .clone(),
                            self.control.clone(),
                        )?;
                        child.snapshot.active_ms = self.snapshot.restore_active_ms;
                        let result = child.execute_with_budget(
                            session,
                            game,
                            remaining(),
                            simulation_restore_return,
                            |snapshot| {
                                self.snapshot
                                    .flow
                                    .update_restore(snapshot.flow)
                                    .map_err(anyhow::Error::msg)?;
                                self.snapshot.restore_active_ms = snapshot.active_ms;
                                self.snapshot.inputs_released = snapshot.inputs_released;
                                self.snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                publish(self.snapshot.clone())
                            },
                        );
                        self.snapshot
                            .flow
                            .update_restore(child.snapshot.flow)
                            .map_err(anyhow::Error::msg)?;
                        self.snapshot.restore_active_ms = child.snapshot.active_ms;
                        self.snapshot.inputs_released = child.snapshot.inputs_released;
                        let result = result?;
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(PreparationEnd::TimedOut);
                        }
                        match result {
                            RestoreEnd::Completed => self
                                .snapshot
                                .flow
                                .restore_finished()
                                .map_err(anyhow::Error::msg)?,
                            RestoreEnd::Paused => return Ok(PreparationEnd::Paused),
                            RestoreEnd::Stopped => return Ok(PreparationEnd::Stopped),
                            RestoreEnd::TimedOut => return Ok(PreparationEnd::TimedOut),
                            RestoreEnd::Failed => anyhow::bail!("恢复子流程失败"),
                        }
                        self.snapshot.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        publish(self.snapshot.clone())?;
                    }
                    Stage::Done => unreachable!(),
                }
            }
        })();
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() || !self.snapshot.inputs_released {
            self.end = Some(PreparationEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "模拟室准备收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(PreparationEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = PreparationEnd::TimedOut;
        }
        if end == PreparationEnd::Paused && !self.control.acknowledge_pause() {
            end = PreparationEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
/// 恢复执行器只能采用传入的同一帧；不会重新截图或采用背景数字。
pub fn simulation_restore_return(
    session: &NativeSession,
    image: &RecognitionImage,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Option<RestoreReturn>> {
    Ok(match session.inspect_simulation(image, cancel, timeout)? {
        Some(SimulationView::Challenge {
            current_charge,
            required_charge,
        }) => Some(RestoreReturn {
            current_charge,
            required_charge,
        }),
        _ => None,
    })
}
