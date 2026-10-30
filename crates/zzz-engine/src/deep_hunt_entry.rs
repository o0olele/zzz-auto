//! 由本次传送取得目标，再完成深度追猎的独立电量和出战页准备。
use crate::{
    charge_transport::TrainingTransportRun,
    restore_charge::{RestoreEnd, RestoreRun, RestoreSnapshot},
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{NativeSession, deep_hunt::HuntInputEnd};
use od_core::{
    deep_hunt::{HuntEntryFlow, HuntEntryStage, HuntRequest, HuntView},
    pipeline::{Interrupt, PipelineControl},
    restore_charge::{RestoreFlow, RestoreReturn},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuntEntryEnd {
    Ready,
    Insufficient,
    WeeklyRewardsAvailable,
    Paused,
    Stopped,
    TimedOut,
    Failed,
    AwaitingVerification,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HuntEntrySnapshot {
    pub flow: HuntEntryFlow,
    pub restore: Option<RestoreSnapshot>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct HuntEntryRun {
    snapshot: HuntEntrySnapshot,
    control: PipelineControl,
    restore: Option<RestoreRun>,
    restore_started: bool,
    end: Option<HuntEntryEnd>,
    steps: u32,
}
impl HuntEntryRun {
    pub(crate) fn after_transport(
        request: HuntRequest,
        route: &TrainingTransportRun,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(
            route.uses_control(&control) && route.snapshot().flow.request().plan == request.plan,
            "深度追猎传送与冻结计划或控制句柄不符"
        );
        let receipt = route
            .receipt_for_parent()
            .context("深度追猎尚无保存并释放的本次传送路径")?
            .clone();
        let flow = HuntEntryFlow::new(request, receipt).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: HuntEntrySnapshot {
                flow,
                restore: None,
                active_ms: 0,
                inputs_released: true,
            },
            control,
            restore: None,
            restore_started: false,
            end: None,
            steps: 0,
        })
    }
    pub fn snapshot(&self) -> HuntEntrySnapshot {
        self.snapshot.clone()
    }
    pub(crate) fn ready_for_parent(&self) -> bool {
        self.end == Some(HuntEntryEnd::Ready)
            && self.snapshot.inputs_released
            && self.snapshot.flow.ready()
    }
    pub(crate) fn shortage_for_parent(&self) -> bool {
        self.end == Some(HuntEntryEnd::Insufficient)
            && self.snapshot.inputs_released
            && self.snapshot.flow.insufficient()
    }
    fn interrupted(&self) -> Option<HuntEntryEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => HuntEntryEnd::Paused,
            Interrupt::Stop => HuntEntryEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(HuntEntryEnd::Paused | HuntEntryEnd::Ready | HuntEntryEnd::Insufficient)
            ) && self.snapshot.inputs_released,
            "深度追猎详情尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        if self.restore_started {
            self.restore
                .as_mut()
                .context("深度追猎恢复子流程缺失")?
                .resume()?;
        } else {
            ensure!(self.control.resume(), "停止不能继续");
        }
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    pub fn execute_with_budget(
        &mut self,
        n: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(HuntEntrySnapshot) -> Result<()>,
    ) -> Result<HuntEntryEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(HuntEntryEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "深度追猎详情不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<_> {
            n.check_ether_synthesis_target(game)?;
            n.require_deep_hunt()?;
            n.require_restore_charge()?;
            loop {
                if let Some(e) = self.interrupted() {
                    return Ok(e);
                }
                if remaining().is_zero() {
                    return Ok(HuntEntryEnd::TimedOut);
                }
                if self.snapshot.flow.ready() {
                    return Ok(HuntEntryEnd::Ready);
                }
                if self.snapshot.flow.weekly_blocked().is_some() {
                    return Ok(HuntEntryEnd::WeeklyRewardsAvailable);
                }
                if self.snapshot.flow.insufficient() {
                    return Ok(HuntEntryEnd::Insufficient);
                }
                if self.snapshot.flow.needs_verification() {
                    return Ok(HuntEntryEnd::AwaitingVerification);
                }
                ensure!(self.steps < 4096, "深度追猎详情步骤超限");
                self.steps += 1;
                if self.snapshot.flow.stage() == HuntEntryStage::Restore {
                    if self.restore.is_none() {
                        let c = RestoreRun::new(
                            RestoreFlow::new(
                                self.snapshot
                                    .flow
                                    .restore_request()
                                    .map_err(anyhow::Error::msg)?,
                            )
                            .map_err(anyhow::Error::msg)?,
                            self.control.clone(),
                        )?;
                        self.snapshot.restore = Some(c.snapshot.clone());
                        self.restore = Some(c);
                        self.restore_started = false;
                        publish(self.snapshot.clone())?;
                        continue;
                    }
                    self.restore_started = true;
                    let c = self.restore.as_mut().context("深度追猎恢复缺失")?;
                    let expected = self
                        .snapshot
                        .flow
                        .request()
                        .plan
                        .execution_mission_type()
                        .to_owned();
                    let snapshot = &mut self.snapshot;
                    let result = c.execute_with_budget(
                        n,
                        game,
                        remaining(),
                        |s, image, cancel, timeout| {
                            Ok(
                                match s.inspect_deep_hunt(image, &expected, cancel, timeout)? {
                                    Some(HuntView::Detail {
                                        charge: Some(charge),
                                        mission_type,
                                        weekly: od_core::deep_hunt::WeeklyState::Exhausted,
                                        mode: od_core::deep_hunt::DeepState::On,
                                        ..
                                    }) if od_core::deep_hunt::valid_charge(charge)
                                        && od_core::deep_hunt::same_mission(
                                            &expected,
                                            &mission_type,
                                        ) =>
                                    {
                                        Some(RestoreReturn {
                                            current_charge: charge.current_charge,
                                            required_charge: 60,
                                        })
                                    }
                                    _ => None,
                                },
                            )
                        },
                        |r| {
                            snapshot.inputs_released = r.inputs_released;
                            snapshot.restore = Some(r);
                            snapshot.active_ms =
                                initial.saturating_add(started.elapsed().as_millis() as u64);
                            publish(snapshot.clone())
                        },
                    );
                    self.snapshot.restore = Some(c.snapshot.clone());
                    self.snapshot.inputs_released = c.snapshot.inputs_released;
                    match result? {
                        RestoreEnd::Completed => {
                            self.snapshot
                                .flow
                                .restored(&c.snapshot.flow)
                                .map_err(anyhow::Error::msg)?;
                            self.restore = None;
                            self.restore_started = false;
                            publish(self.snapshot.clone())?;
                            continue;
                        }
                        RestoreEnd::Paused => return Ok(HuntEntryEnd::Paused),
                        RestoreEnd::Stopped => return Ok(HuntEntryEnd::Stopped),
                        RestoreEnd::TimedOut => return Ok(HuntEntryEnd::TimedOut),
                        RestoreEnd::Failed => anyhow::bail!("深度追猎恢复失败"),
                    }
                }
                let read = n.read_deep_hunt(
                    self.snapshot.flow.request().plan.execution_mission_type(),
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = n.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "深度追猎详情读帧后输入未释放"
                );
                if let Some(e) = self.interrupted() {
                    return Ok(e);
                }
                if remaining().is_zero() {
                    return Ok(HuntEntryEnd::TimedOut);
                }
                let Some(view) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let action = self.snapshot.flow.next(&view).map_err(anyhow::Error::msg)?;
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                let Some(a) = action else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let before = self.snapshot.flow.clone();
                self.snapshot.flow.prepare(a).map_err(anyhow::Error::msg)?;
                if let Err(e) = publish(self.snapshot.clone()) {
                    self.snapshot.flow = before;
                    return Err(e);
                }
                if self.interrupted().is_some() || remaining().is_zero() {
                    self.snapshot
                        .flow
                        .not_posted(a)
                        .map_err(anyhow::Error::msg)?;
                    return Ok(self.interrupted().unwrap_or(HuntEntryEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = n.act_deep_hunt_entry(
                    &self.snapshot.flow,
                    a,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = n.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "深度追猎详情动作后输入未释放"
                );
                match input {
                    Ok(HuntInputEnd::Posted) => {
                        self.snapshot.flow.posted(a).map_err(anyhow::Error::msg)?
                    }
                    Ok(HuntInputEnd::NotPosted | HuntInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(a)
                        .map_err(anyhow::Error::msg)?,
                    Ok(HuntInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(a)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(HuntEntryEnd::TimedOut);
                    }
                    Err(e) => {
                        if let Some(e) = self.interrupted() {
                            return Ok(e);
                        }
                        if remaining().is_zero() {
                            return Ok(HuntEntryEnd::TimedOut);
                        }
                        return Err(e);
                    }
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.snapshot.inputs_released = n.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        self.end = Some(HuntEntryEnd::Failed);
        ensure!(self.snapshot.inputs_released, "深度追猎详情收尾未释放输入");
        let mut end = result?;
        publish(self.snapshot.clone())?;
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(e) = self.interrupted() {
            end = e;
        } else if remaining().is_zero() {
            end = HuntEntryEnd::TimedOut;
        }
        if end == HuntEntryEnd::Paused && !self.control.acknowledge_pause() {
            end = HuntEntryEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
