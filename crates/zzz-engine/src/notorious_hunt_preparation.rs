//! 同一次周常入口、等级与选队交接；执行器只到出战页，不发送出战。
use crate::{
    notorious_hunt_entry::{WeeklyApproachEnd, WeeklyApproachSnapshot, WeeklyHuntApproachRun},
    simulation_teams::{TeamsEnd, TeamsRun, TeamsSnapshot},
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{NativeSession, deep_hunt::HuntInputEnd};
use od_core::{
    notorious_hunt_preparation::{
        WeeklyPreparationFlow, WeeklyPreparationRequest, WeeklyPreparationStage,
    },
    pipeline::{Interrupt, PipelineControl},
    simulation_teams::{TeamsFlow, TeamsResult},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeeklyPreparationEnd {
    Prepared,
    Exhausted,
    AwaitingVerification,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyPreparationPhase {
    Approach,
    Level,
    Teams,
    Prepared,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyPreparationSnapshot {
    pub id: Uuid,
    pub request: WeeklyPreparationRequest,
    pub phase: WeeklyPreparationPhase,
    pub approach: WeeklyApproachSnapshot,
    pub preparation: Option<WeeklyPreparationFlow>,
    pub teams: Option<TeamsSnapshot>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeeklyPreparationReceipt {
    pub id: Uuid,
    pub plan_id: String,
    pub rewards_left: u8,
    pub level: String,
    pub teams: TeamsResult,
}
struct Resources {
    route: PathBuf,
    hunt: PathBuf,
    level: PathBuf,
    daily: PathBuf,
    simulation: PathBuf,
    hashes: Vec<(PathBuf, Vec<u8>)>,
}
impl Resources {
    fn load(root: &Path) -> Result<Self> {
        od_core::config::reject_links(root)?;
        let mut r = Self {
            route: maa_adapter::charge_transport::validate_charge_transport_bundle(
                &root.join("generated/charge-transport"),
            )?,
            hunt: maa_adapter::deep_hunt::validate_deep_hunt_bundle(
                &root.join("generated/deep-hunt"),
            )?,
            level: maa_adapter::charge_level::validate_charge_level_bundle(
                &root.join("generated/charge-level"),
            )?,
            daily: maa_adapter::validate_basic_daily_bundle(&root.join("generated/basic-daily"))?,
            simulation: maa_adapter::simulation::validate_simulation_bundle(
                &root.join("generated/simulation"),
            )?,
            hashes: Vec::new(),
        };
        r.hashes = [
            r.route.join("charge_transport-manifest.json"),
            r.hunt.join("deep_hunt-manifest.json"),
            r.level.join("charge_level-manifest.json"),
            r.daily.join("basic-daily-manifest.json"),
            r.simulation.join("simulation-manifest.json"),
        ]
        .into_iter()
        .map(|p| Ok((p.clone(), Sha256::digest(std::fs::read(p)?).to_vec())))
        .collect::<Result<_>>()?;
        Ok(r)
    }
    fn verify(&self) -> Result<()> {
        for (p, h) in &self.hashes {
            od_core::config::reject_links(p)?;
            ensure!(
                Sha256::digest(std::fs::read(p)?).as_slice() == h,
                "周常准备冻结资源已变化"
            );
        }
        maa_adapter::charge_transport::validate_charge_transport_bundle(&self.route)?;
        maa_adapter::deep_hunt::validate_deep_hunt_bundle(&self.hunt)?;
        maa_adapter::charge_level::validate_charge_level_bundle(&self.level)?;
        maa_adapter::validate_basic_daily_bundle(&self.daily)?;
        maa_adapter::simulation::validate_simulation_bundle(&self.simulation)?;
        Ok(())
    }
}
pub struct WeeklyHuntPreparationRun {
    snapshot: WeeklyPreparationSnapshot,
    approach: WeeklyHuntApproachRun,
    teams: Option<TeamsRun>,
    resources: Resources,
    control: PipelineControl,
    end: Option<WeeklyPreparationEnd>,
    approach_started: bool,
    child_paused: bool,
    steps: u32,
}
impl WeeklyHuntPreparationRun {
    pub fn new(
        id: Uuid,
        request: WeeklyPreparationRequest,
        root: &Path,
        control: PipelineControl,
    ) -> Result<Self> {
        request.validate().map_err(anyhow::Error::msg)?;
        let resources = Resources::load(root)?;
        let approach =
            WeeklyHuntApproachRun::new(id, request.entry_request(), root, control.clone())?;
        Ok(Self {
            snapshot: WeeklyPreparationSnapshot {
                id,
                request,
                phase: WeeklyPreparationPhase::Approach,
                approach: approach.snapshot(),
                preparation: None,
                teams: None,
                active_ms: 0,
                inputs_released: true,
            },
            approach,
            teams: None,
            resources,
            control,
            end: None,
            approach_started: false,
            child_paused: false,
            steps: 0,
        })
    }
    pub fn snapshot(&self) -> WeeklyPreparationSnapshot {
        self.snapshot.clone()
    }
    pub fn receipt(&self) -> Option<WeeklyPreparationReceipt> {
        if self.end != Some(WeeklyPreparationEnd::Prepared)
            || !self.snapshot.inputs_released
            || self.control.requested().is_some()
        {
            return None;
        }
        Some(WeeklyPreparationReceipt {
            id: self.snapshot.id,
            plan_id: self.snapshot.request.plan.plan_id.clone(),
            rewards_left: self.snapshot.preparation.as_ref()?.rewards_left(),
            level: self.snapshot.request.plan.level.clone(),
            teams: self.snapshot.teams.as_ref()?.flow.result()?,
        })
    }
    fn interrupted(&self) -> Option<WeeklyPreparationEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => WeeklyPreparationEnd::Paused,
            Interrupt::Stop => WeeklyPreparationEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(WeeklyPreparationEnd::Paused) && self.snapshot.inputs_released,
            "周常准备尚未安全暂停"
        );
        use WeeklyPreparationPhase as P;
        if self.snapshot.phase == P::Approach && !self.child_paused && self.approach_started {
            self.approach.resume_after_parent_pause()?;
            self.snapshot.approach = self.approach.snapshot();
        } else if self.child_paused {
            match self.snapshot.phase {
                P::Approach => {
                    self.approach.resume()?;
                    self.snapshot.approach = self.approach.snapshot();
                }
                P::Teams => {
                    let child = self.teams.as_mut().context("周常选队缺失")?;
                    child.resume()?;
                    self.snapshot.teams = Some(child.snapshot.clone());
                }
                _ => anyhow::bail!("周常暂停阶段与子流程不一致"),
            }
        } else {
            ensure!(self.control.resume(), "停止不能继续");
            if self.snapshot.phase == P::Level {
                self.snapshot
                    .preparation
                    .as_mut()
                    .context("周常等级准备缺失")?
                    .resume()
                    .map_err(anyhow::Error::msg)?;
            } else if self.snapshot.phase == P::Prepared {
                // 终态保存期间暂停：只重读出战页，不再打开已确认的编队。
                let mut flow = self
                    .snapshot
                    .teams
                    .as_ref()
                    .context("周常选队缺失")?
                    .flow
                    .clone();
                flow.resume().map_err(anyhow::Error::msg)?;
                let child = TeamsRun::new(flow, self.control.clone())?;
                self.snapshot.teams = Some(child.snapshot.clone());
                self.teams = Some(child);
                self.snapshot.phase = P::Teams;
            }
        }
        self.child_paused = false;
        self.end = None;
        Ok(())
    }
    pub fn execute_with_budget(
        &mut self,
        n: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(WeeklyPreparationSnapshot) -> Result<()>,
    ) -> Result<WeeklyPreparationEnd> {
        use WeeklyPreparationEnd as E;
        use WeeklyPreparationPhase as P;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "周常准备不能直接重跑"
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
            let mut loaded = None;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                if self.snapshot.phase == P::Prepared {
                    return Ok(E::Prepared);
                }
                if loaded != Some(self.snapshot.phase) {
                    self.resources.verify()?;
                    match self.snapshot.phase {
                        P::Level => {
                            n.load_deep_hunt_bundle(&self.resources.hunt)?;
                            n.load_charge_level_bundle(&self.resources.level)?;
                        }
                        P::Teams => {
                            n.load_simulation_bundle(&self.resources.simulation)?;
                            n.load_basic_daily_bundle(&self.resources.daily)?;
                        }
                        _ => (),
                    }
                    loaded = Some(self.snapshot.phase);
                    continue;
                }
                match self.snapshot.phase {
                    P::Approach => {
                        self.approach_started = true;
                        let s = &mut self.snapshot;
                        let result = self
                            .approach
                            .execute_with_budget(n, game, remaining(), |v| {
                                s.inputs_released = v.inputs_released;
                                s.approach = v;
                                s.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                publish(s.clone())
                            });
                        self.snapshot.approach = self.approach.snapshot();
                        self.snapshot.inputs_released = self.snapshot.approach.inputs_released;
                        let end = result?;
                        self.child_paused = end == WeeklyApproachEnd::Paused;
                        match end {
                            WeeklyApproachEnd::Observed => (),
                            WeeklyApproachEnd::Exhausted => return Ok(E::Exhausted),
                            WeeklyApproachEnd::AwaitingVerification => {
                                return Ok(E::AwaitingVerification);
                            }
                            WeeklyApproachEnd::Paused => return Ok(E::Paused),
                            WeeklyApproachEnd::Stopped => return Ok(E::Stopped),
                            WeeklyApproachEnd::TimedOut => return Ok(E::TimedOut),
                            WeeklyApproachEnd::Failed => anyhow::bail!("周常准备的入口失败"),
                        }
                        ensure!(
                            self.approach.receipt_for_parent().is_some(),
                            "周常入口尚未保存并安全交回"
                        );
                        self.snapshot.preparation = Some(
                            WeeklyPreparationFlow::after_entry(
                                self.snapshot.request.clone(),
                                self.snapshot
                                    .approach
                                    .entry
                                    .as_ref()
                                    .context("周常入口证明缺失")?,
                            )
                            .map_err(anyhow::Error::msg)?,
                        );
                        self.snapshot.phase = P::Level;
                    }
                    P::Level => {
                        let flow = self.snapshot.preparation.as_ref().context("周常准备缺失")?;
                        if flow.stage() == WeeklyPreparationStage::AwaitingVerification {
                            return Ok(E::AwaitingVerification);
                        }
                        if flow.stage() == WeeklyPreparationStage::Ready {
                            let child = TeamsRun::new(
                                TeamsFlow::new(
                                    self.snapshot
                                        .request
                                        .team_request()
                                        .map_err(anyhow::Error::msg)?,
                                )
                                .map_err(anyhow::Error::msg)?,
                                self.control.clone(),
                            )?;
                            self.snapshot.teams = Some(child.snapshot.clone());
                            self.teams = Some(child);
                            self.snapshot.phase = P::Teams;
                        } else {
                            ensure!(self.steps < 4096, "周常准备观察达到上限");
                            self.steps += 1;
                            let read = n.read_weekly_hunt_preparation(
                                &self.snapshot.request.plan.mission_type_name,
                                &self.control.cancellation(),
                                remaining(),
                            );
                            self.snapshot.inputs_released = n.release_inputs().is_ok();
                            ensure!(self.snapshot.inputs_released, "周常准备读帧未释放输入");
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
                                .preparation
                                .as_mut()
                                .context("周常准备缺失")?
                                .next(&view)
                                .map_err(anyhow::Error::msg)?;
                            self.snapshot.active_ms =
                                initial.saturating_add(started.elapsed().as_millis() as u64);
                            publish(self.snapshot.clone())?;
                            if let Some(action) = action {
                                let before = self.snapshot.preparation.clone();
                                self.snapshot
                                    .preparation
                                    .as_mut()
                                    .context("周常准备缺失")?
                                    .prepare(action)
                                    .map_err(anyhow::Error::msg)?;
                                if let Err(error) = publish(self.snapshot.clone()) {
                                    self.snapshot.preparation = before;
                                    return Err(error);
                                }
                                self.resources.verify()?;
                                if self.interrupted().is_some() || remaining().is_zero() {
                                    self.snapshot
                                        .preparation
                                        .as_mut()
                                        .context("周常准备缺失")?
                                        .not_posted(action)
                                        .map_err(anyhow::Error::msg)?;
                                    publish(self.snapshot.clone())?;
                                    return Ok(self.interrupted().unwrap_or(E::TimedOut));
                                }
                                self.snapshot.inputs_released = false;
                                let input = n.act_weekly_hunt_preparation(
                                    self.snapshot.preparation.as_ref().context("周常准备缺失")?,
                                    action,
                                    &self.control.cancellation(),
                                    remaining(),
                                    game,
                                );
                                self.snapshot.inputs_released = n.release_inputs().is_ok();
                                ensure!(self.snapshot.inputs_released, "周常准备动作未释放输入");
                                let f =
                                    self.snapshot.preparation.as_mut().context("周常准备缺失")?;
                                match input {
                                    Ok(HuntInputEnd::Posted) => {
                                        f.posted(action).map_err(anyhow::Error::msg)?
                                    }
                                    Ok(HuntInputEnd::Changed | HuntInputEnd::NotPosted) => {
                                        f.not_posted(action).map_err(anyhow::Error::msg)?
                                    }
                                    Ok(HuntInputEnd::TimedOut) => {
                                        f.not_posted(action).map_err(anyhow::Error::msg)?;
                                        return Ok(E::TimedOut);
                                    }
                                    Err(error) => {
                                        if let Some(end) = self.interrupted() {
                                            return Ok(end);
                                        }
                                        return Err(error);
                                    }
                                }
                            }
                        }
                    }
                    P::Teams => {
                        let child = self.teams.as_mut().context("周常选队缺失")?;
                        let s = &mut self.snapshot;
                        let result = child.execute_with_budget(n, game, remaining(), |v| {
                            s.inputs_released = v.inputs_released;
                            s.teams = Some(v);
                            s.active_ms =
                                initial.saturating_add(started.elapsed().as_millis() as u64);
                            publish(s.clone())
                        });
                        self.snapshot.teams = Some(child.snapshot.clone());
                        self.snapshot.inputs_released = child.snapshot.inputs_released;
                        let end = result?;
                        self.child_paused = end == TeamsEnd::Paused;
                        match end {
                            TeamsEnd::Prepared => self.snapshot.phase = P::Prepared,
                            TeamsEnd::Paused => return Ok(E::Paused),
                            TeamsEnd::Stopped => return Ok(E::Stopped),
                            TeamsEnd::TimedOut => return Ok(E::TimedOut),
                            TeamsEnd::Failed => anyhow::bail!("周常选队失败"),
                        }
                    }
                    P::Prepared => unreachable!(),
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.end = Some(E::Failed);
        self.snapshot.inputs_released = n.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        ensure!(self.snapshot.inputs_released, "周常准备收尾未释放输入");
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
