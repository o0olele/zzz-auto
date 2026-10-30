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
    simulation_deploy::{DeployAction, DeployFlow, DeployRequest, DeploymentContext},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct PreparedDeployment {
    plan: AppCombatPlan,
    script: Script,
    root: PathBuf,
    manifest_hash: Vec<u8>,
}
impl PreparedDeployment {
    pub(crate) fn resource_root(&self) -> Result<&Path> {
        od_core::config::reject_links(&self.root)?;
        ensure!(
            Sha256::digest(std::fs::read(self.root.join("combat-manifest.json"))?).as_slice()
                == self.manifest_hash,
            "准备期间冻结战斗资源已变化"
        );
        CombatBundle::load(&self.root)?;
        Ok(&self.root)
    }
    pub(crate) fn plan(&self) -> &AppCombatPlan {
        &self.plan
    }
    /// 只克隆首次输入前的编译结果，不重新读取当前脚本导入选择。
    pub(crate) fn frozen_segment(&self, id: Uuid, control: PipelineControl) -> Result<CombatRun> {
        self.resource_root()?;
        let mut run =
            CombatRun::new_segment(id, self.plan.config.clone(), self.script.clone(), control)?;
        run.progress.script_revision = self.plan.script_revision.clone();
        Ok(run)
    }
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
pub struct DeploymentSnapshot<R = DeployRequest> {
    pub flow: DeployFlow<R>,
    pub combat: AppCombatPlan,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct DeploymentRun<R = DeployRequest> {
    pub id: Uuid,
    pub snapshot: DeploymentSnapshot<R>,
    prepared: PreparedDeployment,
    control: PipelineControl,
    end: Option<DeploymentEnd>,
}
impl<R: DeploymentContext> DeploymentRun<R> {
    pub(crate) fn parent_context(&self) -> Result<(&Path, &AppCombatPlan, PipelineControl)> {
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.end.is_none()
                && self.snapshot.active_ms == 0
                && self.snapshot.inputs_released
                && !self.snapshot.flow.completed()
                && self.snapshot.combat == self.prepared.plan,
            "模拟室父任务只能接收尚未执行的冻结出战流程"
        );
        Ok((
            &self.prepared.root,
            &self.prepared.plan,
            self.control.clone(),
        ))
    }
    pub fn after_teams(
        id: Uuid,
        request: R,
        teams_end: TeamsEnd,
        teams: &TeamsSnapshot,
        prepared: PreparedDeployment,
        control: PipelineControl,
    ) -> Result<Self> {
        teams.flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            teams_end == TeamsEnd::Prepared
                && teams.inputs_released
                && teams.flow.result().as_ref() == Some(request.team()),
            "选队未完成、输入未释放或目标与出战不一致"
        );
        ensure!(
            prepared.plan.config.script == request.team().script_name,
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
            matches!(
                self.end,
                Some(DeploymentEnd::Paused | DeploymentEnd::Entered)
            ) && self.snapshot.inputs_released
                && self.control.requested() == Some(Interrupt::Pause),
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
            self.control.requested().is_none(),
            "仍有控制请求，不能交接子战斗"
        );
        self.into_combat_for_parent()
    }
    /// 父流程转换不发送输入；转换与请求并发时由父流程在交接检查点后处理请求。
    pub(crate) fn into_combat_for_parent(self) -> Result<CombatRun> {
        ensure!(
            self.end == Some(DeploymentEnd::Entered)
                && self.snapshot.inputs_released
                && self.snapshot.flow.completed(),
            "出战未完成或输入未释放，不能交接子战斗"
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
        mut publish: impl FnMut(DeploymentSnapshot<R>) -> Result<()>,
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
            u64::from(self.snapshot.flow.request().timeout_ms()).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = |deploy_started: bool| {
            let total = budget.saturating_sub(started.elapsed());
            if deploy_started {
                total
            } else {
                // 上游出战按钮节点限时 60 秒，暂停前已消耗时间也计入。
                total.min(
                    Duration::from_millis(60_000u64.saturating_sub(initial))
                        .saturating_sub(started.elapsed()),
                )
            }
        };
        let result = (|| -> Result<DeploymentEnd> {
            session.check_combat_input_mode(od_core::InputMode::Foreground)?;
            session.require_simulation_deploy()?;
            session.release_inputs()?;
            if remaining(self.snapshot.flow.deployment_started()).is_zero() {
                return Ok(self.interrupted().unwrap_or(DeploymentEnd::TimedOut));
            }
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
                if remaining(self.snapshot.flow.deployment_started()).is_zero() {
                    return Ok(DeploymentEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(DeploymentEnd::Entered);
                }
                let read = session.read_simulation_deploy(
                    &recognizer,
                    &self.control.cancellation(),
                    remaining(self.snapshot.flow.deployment_started()),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "出战读帧后输入未释放");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining(self.snapshot.flow.deployment_started()).is_zero() {
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
                let deploy_started = action != DeployAction::Deploy;
                if self.interrupted().is_some() || remaining(deploy_started).is_zero() {
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
                    remaining(deploy_started),
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
                        if remaining(deploy_started).is_zero() {
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
        } else if remaining(self.snapshot.flow.deployment_started()).is_zero() {
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
