//! 周常普通传送到免费奖励余量观察，同一父控制器下不出战、不领奖。
use crate::charge_transport::{
    TrainingTransportEnd, TrainingTransportRun, TrainingTransportSnapshot,
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{NativeSession, deep_hunt::HuntInputEnd};
use od_core::{
    charge_transport::TrainingTransportRequest,
    notorious_hunt_entry::{WeeklyEntryFlow, WeeklyEntryRequest, WeeklyEntryStage},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeeklyApproachEnd {
    Observed,
    Exhausted,
    AwaitingVerification,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyApproachSnapshot {
    pub id: Uuid,
    pub request: WeeklyEntryRequest,
    pub transport: TrainingTransportSnapshot,
    pub entry: Option<WeeklyEntryFlow>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeeklyApproachReceipt {
    pub id: Uuid,
    pub plan_id: String,
    pub rewards_left: u8,
}
struct Resources {
    route: PathBuf,
    hunt: PathBuf,
    manifests: Vec<(PathBuf, Vec<u8>)>,
}
impl Resources {
    fn load(root: &Path) -> Result<Self> {
        od_core::config::reject_links(root)?;
        let route = maa_adapter::charge_transport::validate_charge_transport_bundle(
            &root.join("generated/charge-transport"),
        )?;
        let hunt =
            maa_adapter::deep_hunt::validate_deep_hunt_bundle(&root.join("generated/deep-hunt"))?;
        let manifests = [
            route.join("charge_transport-manifest.json"),
            hunt.join("deep_hunt-manifest.json"),
        ]
        .into_iter()
        .map(|p| Ok((p.clone(), Sha256::digest(std::fs::read(p)?).to_vec())))
        .collect::<Result<_>>()?;
        Ok(Self {
            route,
            hunt,
            manifests,
        })
    }
    fn verify(&self) -> Result<()> {
        for (p, hash) in &self.manifests {
            od_core::config::reject_links(p)?;
            ensure!(
                Sha256::digest(std::fs::read(p)?).as_slice() == hash,
                "周常入口冻结资源已变化"
            );
        }
        maa_adapter::charge_transport::validate_charge_transport_bundle(&self.route)?;
        maa_adapter::deep_hunt::validate_deep_hunt_bundle(&self.hunt)?;
        Ok(())
    }
}
pub struct WeeklyHuntApproachRun {
    snapshot: WeeklyApproachSnapshot,
    resources: Resources,
    control: PipelineControl,
    transport: TrainingTransportRun,
    transport_started: bool,
    end: Option<WeeklyApproachEnd>,
    steps: u32,
}
impl WeeklyHuntApproachRun {
    pub fn new(
        id: Uuid,
        request: WeeklyEntryRequest,
        root: &Path,
        control: PipelineControl,
    ) -> Result<Self> {
        request.validate().map_err(anyhow::Error::msg)?;
        let resources = Resources::load(root)?;
        let transport = TrainingTransportRun::new(
            TrainingTransportRequest {
                plan: request.plan.clone(),
                timeout_ms: request.timeout_ms,
            },
            control.clone(),
        )?;
        Ok(Self {
            snapshot: WeeklyApproachSnapshot {
                id,
                request,
                transport: transport.snapshot(),
                entry: None,
                active_ms: 0,
                inputs_released: true,
            },
            resources,
            control,
            transport,
            transport_started: false,
            end: None,
            steps: 0,
        })
    }
    pub fn snapshot(&self) -> WeeklyApproachSnapshot {
        self.snapshot.clone()
    }
    pub fn receipt(&self) -> Option<WeeklyApproachReceipt> {
        if self.control.requested().is_some() {
            return None;
        }
        self.receipt_for_parent()
    }
    pub(crate) fn receipt_for_parent(&self) -> Option<WeeklyApproachReceipt> {
        if !matches!(
            self.end,
            Some(WeeklyApproachEnd::Observed | WeeklyApproachEnd::Exhausted)
        ) || !self.snapshot.inputs_released
        {
            return None;
        }
        Some(WeeklyApproachReceipt {
            id: self.snapshot.id,
            plan_id: self.snapshot.request.plan.plan_id.clone(),
            rewards_left: self.snapshot.entry.as_ref()?.rewards()?,
        })
    }
    fn interrupted(&self) -> Option<WeeklyApproachEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => WeeklyApproachEnd::Paused,
            Interrupt::Stop => WeeklyApproachEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(WeeklyApproachEnd::Paused) && self.snapshot.inputs_released,
            "周常入口尚未安全暂停"
        );
        self.resume_after_parent_pause()
    }
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(
                    WeeklyApproachEnd::Paused
                        | WeeklyApproachEnd::Observed
                        | WeeklyApproachEnd::Exhausted
                        | WeeklyApproachEnd::AwaitingVerification
                )
            ) && self.snapshot.inputs_released,
            "周常入口尚未安全交回"
        );
        if let Some(flow) = &mut self.snapshot.entry {
            flow.resume().map_err(anyhow::Error::msg)?;
            ensure!(self.control.resume(), "停止不能继续");
        } else if self.transport_started {
            self.transport.resume_after_parent_pause()?;
            self.snapshot.transport = self.transport.snapshot();
        } else {
            ensure!(self.control.resume(), "停止不能继续");
        }
        self.end = None;
        Ok(())
    }
    pub fn execute_with_budget(
        &mut self,
        n: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(WeeklyApproachSnapshot) -> Result<()>,
    ) -> Result<WeeklyApproachEnd> {
        use WeeklyApproachEnd as E;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "周常入口不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.request.timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<E> {
            n.quiesce()?;
            n.check_ether_synthesis_target(game)?;
            self.resources.verify()?;
            if let Some(end) = self.interrupted() {
                return Ok(end);
            }
            if remaining().is_zero() {
                return Ok(E::TimedOut);
            }
            if self.snapshot.entry.is_none() {
                n.load_charge_transport_bundle(&self.resources.route)?;
                self.transport_started = true;
                let snapshot = &mut self.snapshot;
                let result = self
                    .transport
                    .execute_with_budget(n, game, remaining(), |s| {
                        snapshot.inputs_released = s.inputs_released;
                        snapshot.transport = s;
                        snapshot.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        publish(snapshot.clone())
                    });
                self.snapshot.transport = self.transport.snapshot();
                self.snapshot.inputs_released = self.snapshot.transport.inputs_released;
                let end = result?;
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                match end {
                    TrainingTransportEnd::AwaitingVerification => {}
                    TrainingTransportEnd::Paused => return Ok(E::Paused),
                    TrainingTransportEnd::Stopped => return Ok(E::Stopped),
                    TrainingTransportEnd::TimedOut => return Ok(E::TimedOut),
                    TrainingTransportEnd::Failed => anyhow::bail!("周常入口传送失败"),
                }
                let route = self
                    .transport
                    .receipt_for_parent()
                    .context("周常尚无保存并释放的本次传送回执")?
                    .clone();
                self.snapshot.entry = Some(
                    WeeklyEntryFlow::new(self.snapshot.request.clone(), route)
                        .map_err(anyhow::Error::msg)?,
                );
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
            self.resources.verify()?;
            n.clear_charge_level_bundle()?;
            n.load_deep_hunt_bundle(&self.resources.hunt)?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                let flow = self.snapshot.entry.as_ref().context("周常详情执行器缺失")?;
                flow.validate().map_err(anyhow::Error::msg)?;
                if let Some(left) = flow.rewards() {
                    return Ok(if left == 0 { E::Exhausted } else { E::Observed });
                }
                if flow.stage() == WeeklyEntryStage::AwaitingVerification {
                    return Ok(E::AwaitingVerification);
                }
                ensure!(self.steps < 4096, "周常入口观察达到上限");
                self.steps += 1;
                let read = n.read_weekly_hunt(
                    &self.snapshot.request.plan.mission_type_name,
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = n.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "周常入口读帧未释放输入");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                let Some(view) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let action = self
                    .snapshot
                    .entry
                    .as_mut()
                    .context("周常详情缺失")?
                    .next(&view)
                    .map_err(anyhow::Error::msg)?;
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                let Some(action) = action else {
                    continue;
                };
                let before = self.snapshot.entry.clone();
                self.snapshot
                    .entry
                    .as_mut()
                    .context("周常详情缺失")?
                    .prepare(action)
                    .map_err(anyhow::Error::msg)?;
                if let Err(error) = publish(self.snapshot.clone()) {
                    self.snapshot.entry = before;
                    return Err(error);
                }
                self.resources.verify()?;
                if self.interrupted().is_some() || remaining().is_zero() {
                    self.snapshot
                        .entry
                        .as_mut()
                        .context("周常详情缺失")?
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(E::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = n.act_weekly_hunt_entry(
                    self.snapshot.entry.as_ref().context("周常详情缺失")?,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = n.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "周常入口动作未释放输入");
                let flow = self.snapshot.entry.as_mut().context("周常详情缺失")?;
                match input {
                    Ok(HuntInputEnd::Posted) => flow.posted(action).map_err(anyhow::Error::msg)?,
                    Ok(HuntInputEnd::NotPosted | HuntInputEnd::Changed) => {
                        flow.not_posted(action).map_err(anyhow::Error::msg)?
                    }
                    Ok(HuntInputEnd::TimedOut) => {
                        flow.not_posted(action).map_err(anyhow::Error::msg)?;
                        return Ok(E::TimedOut);
                    }
                    Err(error) => {
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        return Err(error);
                    }
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.end = Some(E::Failed);
        self.snapshot.inputs_released = n.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        ensure!(self.snapshot.inputs_released, "周常入口收尾未释放输入");
        let mut end = result?;
        publish(self.snapshot.clone())?;
        self.resources.verify()?;
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = E::TimedOut;
        }
        if end == E::Paused && !self.control.acknowledge_pause() {
            end = E::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
