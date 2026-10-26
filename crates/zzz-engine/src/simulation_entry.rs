//! 详情准备到选队的父级交接；首个下一步输入前已冻结战斗脚本。
use crate::{
    simulation_deploy::{DeploymentRun, PreparedDeployment},
    simulation_preparation::{PreparationEnd, PreparationSnapshot},
    simulation_teams::{TeamsEnd, TeamsRun, TeamsSnapshot},
};
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, simulation::CardsInputEnd};
use od_core::{
    pipeline::{Interrupt, PipelineControl},
    simulation_entry::{EntryFlow, EntryRequest},
    simulation_teams::TeamsFlow,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntrySnapshot {
    pub flow: EntryFlow,
    pub teams: Option<TeamsSnapshot>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct EntryRun {
    pub id: Uuid,
    snapshot: EntrySnapshot,
    prepared: PreparedDeployment,
    control: PipelineControl,
    end: Option<EntryEnd>,
}
impl EntryRun {
    pub fn after_preparation(
        id: Uuid,
        request: EntryRequest,
        preparation_end: PreparationEnd,
        preparation: &PreparationSnapshot,
        prepared: PreparedDeployment,
        control: PipelineControl,
    ) -> Result<Self> {
        request.validate().map_err(anyhow::Error::msg)?;
        preparation.flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            preparation_end == PreparationEnd::Completed
                && preparation.inputs_released
                && preparation.flow.completed()
                && preparation.flow.result() == Some(request.prepared)
                && preparation.flow.request().card_num == request.plan.card_num,
            "详情推进前准备未完成、输入未释放或冻结方案不一致"
        );
        ensure!(
            prepared.plan().config.script
                == request
                    .team_request()
                    .map_err(anyhow::Error::msg)?
                    .script_name,
            "详情推进前战斗脚本与冻结编队不一致"
        );
        Ok(Self {
            id,
            snapshot: EntrySnapshot {
                flow: EntryFlow::new(request).map_err(anyhow::Error::msg)?,
                teams: None,
                active_ms: 0,
                inputs_released: true,
            },
            prepared,
            control,
            end: None,
        })
    }
    pub fn snapshot(&self) -> &EntrySnapshot {
        &self.snapshot
    }
    fn interrupted(&self) -> Option<EntryEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => EntryEnd::Paused,
            Interrupt::Stop => EntryEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(EntryEnd::Paused) && self.snapshot.inputs_released,
            "详情推进尚未安全暂停"
        );
        let mut snapshot = self.snapshot.clone();
        if let Some(teams) = &mut snapshot.teams {
            teams.flow.resume().map_err(anyhow::Error::msg)?;
        } else {
            snapshot.flow.resume().map_err(anyhow::Error::msg)?;
        }
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot = snapshot;
        self.end = None;
        Ok(())
    }
    /// 消费 self，保留同一 ID、控制句柄和已编译脚本；转换本身不发送出战输入。
    pub fn into_deployment(self) -> Result<DeploymentRun> {
        ensure!(
            self.end == Some(EntryEnd::Ready)
                && self.snapshot.inputs_released
                && self.snapshot.flow.completed()
                && self.control.requested().is_none(),
            "详情或选队未完成，不能交接出战"
        );
        let teams = self
            .snapshot
            .teams
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("缺少选队检查点"))?;
        let result = teams
            .flow
            .result()
            .ok_or_else(|| anyhow::anyhow!("缺少选队完成证明"))?;
        let request = self
            .snapshot
            .flow
            .request()
            .deployment(result)
            .map_err(anyhow::Error::msg)?;
        DeploymentRun::after_teams(
            self.id,
            request,
            TeamsEnd::Prepared,
            teams,
            self.prepared,
            self.control,
        )
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(EntrySnapshot) -> Result<()>,
    ) -> Result<EntryEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(EntryEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "详情推进不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<EntryEnd> {
            self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
            session.check_combat_input_mode(od_core::InputMode::Foreground)?;
            // 选队资源在下一步输入之前检查，不能推进之后才发现缺包。
            session.require_simulation_teams()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(EntryEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    if self.snapshot.teams.is_none() {
                        self.snapshot.teams = Some(TeamsSnapshot {
                            flow: TeamsFlow::new(
                                self.snapshot
                                    .flow
                                    .request()
                                    .team_request()
                                    .map_err(anyhow::Error::msg)?,
                            )
                            .map_err(anyhow::Error::msg)?,
                            active_ms: 0,
                            inputs_released: true,
                        });
                        self.snapshot.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        publish(self.snapshot.clone())?;
                        continue;
                    }
                    let teams = self.snapshot.teams.as_ref().unwrap();
                    let mut child = TeamsRun::new(teams.flow.clone(), self.control.clone())?;
                    child.snapshot.active_ms = teams.active_ms;
                    let child_end =
                        child.execute_with_budget(session, game, remaining(), |snapshot| {
                            self.snapshot.inputs_released = snapshot.inputs_released;
                            self.snapshot.teams = Some(snapshot);
                            self.snapshot.active_ms =
                                initial.saturating_add(started.elapsed().as_millis() as u64);
                            publish(self.snapshot.clone())
                        });
                    self.snapshot.inputs_released = child.snapshot.inputs_released;
                    self.snapshot.teams = Some(child.snapshot);
                    let child_end = child_end?;
                    if let Some(end) = self.interrupted() {
                        return Ok(end);
                    }
                    if remaining().is_zero() {
                        return Ok(EntryEnd::TimedOut);
                    }
                    return Ok(match child_end {
                        TeamsEnd::Prepared => EntryEnd::Ready,
                        TeamsEnd::Paused => EntryEnd::Paused,
                        TeamsEnd::Stopped => EntryEnd::Stopped,
                        TeamsEnd::TimedOut => EntryEnd::TimedOut,
                        TeamsEnd::Failed => anyhow::bail!("详情推进后的选队失败"),
                    });
                }
                let read = session.read_simulation(&self.control.cancellation(), remaining());
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "详情读帧后输入未释放");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(EntryEnd::TimedOut);
                }
                let Some(view) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let next = self
                    .snapshot
                    .flow
                    .observe(&view)
                    .map_err(anyhow::Error::msg)?;
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                if !next {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                let before = self.snapshot.flow.clone();
                self.snapshot
                    .flow
                    .prepare_next()
                    .map_err(anyhow::Error::msg)?;
                if let Err(error) = publish(self.snapshot.clone()) {
                    self.snapshot.flow = before;
                    return Err(error);
                }
                if self.interrupted().is_some() || remaining().is_zero() {
                    self.snapshot
                        .flow
                        .not_posted_next()
                        .map_err(anyhow::Error::msg)?;
                    return Ok(self.interrupted().unwrap_or(EntryEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_simulation_entry_next(
                    &self.snapshot.flow,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "详情推进后输入未释放");
                match input {
                    Ok(CardsInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted_next()
                        .map_err(anyhow::Error::msg)?,
                    Ok(CardsInputEnd::NotPosted | CardsInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted_next()
                        .map_err(anyhow::Error::msg)?,
                    Ok(CardsInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted_next()
                            .map_err(anyhow::Error::msg)?;
                        return Ok(EntryEnd::TimedOut);
                    }
                    Err(error) => {
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(EntryEnd::TimedOut);
                        }
                        return Err(error);
                    }
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.end = Some(EntryEnd::Failed);
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        ensure!(
            self.snapshot.inputs_released,
            "详情推进收尾无法确认输入释放"
        );
        let mut end = result?;
        publish(self.snapshot.clone())?;
        if let Some(request) = self.interrupted() {
            end = request;
        } else if remaining().is_zero() {
            end = EntryEnd::TimedOut;
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if end == EntryEnd::Paused && !self.control.acknowledge_pause() {
            end = EntryEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
