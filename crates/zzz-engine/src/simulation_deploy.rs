//! 出战与子战斗交接。编译脚本先于消费输入，运行控制权由父任务持有。
use crate::{
    combat::CombatRun,
    simulation_teams::{TeamsEnd, TeamsSnapshot},
};
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, combat_vision::CombatBundle, simulation_deploy::DeployInputEnd};
use od_core::{
    app_combat::AppCombatPlan,
    conditional::Script,
    pipeline::{Interrupt, PipelineControl},
    simulation_deploy::{DeployFlow, DeployRequest},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub struct PreparedDeployment {
    plan: AppCombatPlan,
    script: Script,
    root: PathBuf,
    manifest_hash: Vec<u8>,
}
impl PreparedDeployment {
    pub fn load(plan: AppCombatPlan, root: &Path, data: &Path) -> Result<Self> {
        plan.validate().map_err(anyhow::Error::msg)?;
        let bundle = CombatBundle::load(root)?;
        ensure!(
            crate::combat::script_catalog(&bundle, data)?.revision == plan.script_revision,
            "出战前战斗脚本或资源已变化"
        );
        let source = od_core::combat_scripts::select_root(
            data,
            &bundle.root.join("scripts"),
            "auto_battle",
            &plan.config.script,
        )?;
        let script = Script::load_entry(
            &source,
            "auto_battle",
            &plan.config.script,
            plan.config.use_merged,
        )?;
        bundle.spec.validate_script(&script)?;
        plan.config.validate_script(&script)?;
        ensure!(
            crate::combat::script_catalog(&bundle, data)?.revision == plan.script_revision,
            "编译期间战斗脚本或资源已变化"
        );
        let manifest_hash =
            Sha256::digest(std::fs::read(bundle.root.join("combat-manifest.json"))?).to_vec();
        Ok(Self {
            plan,
            script,
            root: bundle.root,
            manifest_hash,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentEnd {
    Entered,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentSnapshot {
    pub flow: DeployFlow,
    pub combat: AppCombatPlan,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct DeploymentRun {
    pub id: Uuid,
    pub snapshot: DeploymentSnapshot,
    prepared: PreparedDeployment,
    control: PipelineControl,
    end: Option<DeploymentEnd>,
}
impl DeploymentRun {
    pub fn after_teams(
        id: Uuid,
        request: DeployRequest,
        teams_end: TeamsEnd,
        teams: &TeamsSnapshot,
        prepared: PreparedDeployment,
        control: PipelineControl,
    ) -> Result<Self> {
        teams.flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            teams_end == TeamsEnd::Prepared
                && teams.inputs_released
                && teams.flow.result().as_ref() == Some(&request.team),
            "选队未完成、输入未释放或目标与出战不一致"
        );
        ensure!(
            prepared.plan.config.script == request.team.script_name,
            "冻结战斗脚本与选队不一致"
        );
        let flow = DeployFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            id,
            snapshot: DeploymentSnapshot {
                flow,
                combat: prepared.plan.clone(),
                active_ms: 0,
                inputs_released: true,
            },
            prepared,
            control,
            end: None,
        })
    }
    fn interrupted(&self) -> Option<DeploymentEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => DeploymentEnd::Paused,
            Interrupt::Stop => DeploymentEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(DeploymentEnd::Paused) && self.snapshot.inputs_released,
            "出战尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    /// 父任务可在转换前按此方式交接控制器；不允许并行创建第二个输入入口。
    pub fn combat_mode(&self) -> od_core::InputMode {
        self.prepared.plan.config.mode
    }
    pub fn into_combat(self) -> Result<CombatRun> {
        ensure!(
            self.end == Some(DeploymentEnd::Entered)
                && self.snapshot.inputs_released
                && self.snapshot.flow.completed()
                && self.control.requested().is_none(),
            "出战未完成或仍有控制请求，不能交接子战斗"
        );
        ensure!(
            self.snapshot.combat == self.prepared.plan,
            "出战战斗快照与冻结配置不一致"
        );
        CombatRun::new_segment(
            self.id,
            self.prepared.plan.config,
            self.prepared.script,
            self.control,
        )
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(DeploymentSnapshot) -> Result<()>,
    ) -> Result<DeploymentEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(DeploymentEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "出战子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.snapshot.combat == self.prepared.plan,
            "出战冻结配置已变化"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<DeploymentEnd> {
            session.check_combat_input_mode(od_core::InputMode::Foreground)?;
            session.require_simulation_deploy()?;
            session.release_inputs()?;
            ensure!(
                Sha256::digest(std::fs::read(
                    self.prepared.root.join("combat-manifest.json")
                )?)
                .as_slice()
                    == self.prepared.manifest_hash,
                "出战期间战斗资源已变化"
            );
            let recognizer = session.load_combat_bundle(&self.prepared.root)?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(DeploymentEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(DeploymentEnd::Entered);
                }
                let read = session.read_simulation_deploy(
                    &recognizer,
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "出战读帧后输入未释放");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(DeploymentEnd::TimedOut);
                }
                let Some(view) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let action = self.snapshot.flow.next(view).map_err(anyhow::Error::msg)?;
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
                if self.interrupted().is_some() || remaining().is_zero() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    return Ok(self.interrupted().unwrap_or(DeploymentEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_simulation_deploy(
                    &self.snapshot.flow,
                    action,
                    &recognizer,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "出战动作后输入未释放");
                match input {
                    Ok(DeployInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(DeployInputEnd::Changed | DeployInputEnd::NotPosted) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(DeployInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(DeploymentEnd::TimedOut);
                    }
                    Err(error) => {
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(DeploymentEnd::TimedOut);
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
        self.end = Some(DeploymentEnd::Failed);
        ensure!(self.snapshot.inputs_released, "出战收尾无法确认输入释放");
        let mut end = result?;
        publish(self.snapshot.clone())?;
        if let Some(i) = self.interrupted() {
            end = i;
        } else if remaining().is_zero() {
            end = DeploymentEnd::TimedOut;
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if end == DeploymentEnd::Paused && !self.control.acknowledge_pause() {
            end = DeploymentEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
