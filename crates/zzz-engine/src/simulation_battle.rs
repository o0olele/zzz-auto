//! 单次模拟室出战、子战斗和领奖的父流程；不登记计划次数或应用成功。
use crate::{
    combat::{CombatRun, CombatSegmentEnd},
    simulation_deploy::{DeploymentEnd, DeploymentRun, DeploymentSnapshot},
    simulation_entry::EntryRun,
    simulation_settlement::{SettlementEnd, SettlementRun, SettlementSnapshot},
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{
    GameSessionConfig, InferenceDevice, NativeSession, SessionHandoff,
    combat_vision::{CombatBundle, CombatRecognizer},
};
use od_core::{
    CaptureMethod, InputMode, WindowInfo,
    app_combat::AppCombatPlan,
    battle_result::NormalBattleResult,
    combat::CombatProgress,
    pipeline::{Interrupt, PipelineControl, PipelineRequest},
    simulation_cards::CardsCharge,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub enum BattleTarget {
    Game {
        window: WindowInfo,
        capture: CaptureMethod,
    },
    /// 已有的隔离控制器仅复用前台 CPU 会话，不能假装完成实际设备切换。
    Fixture,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BattleStage {
    Deployment,
    Combat,
    Settlement,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleEnd {
    Completed,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BattleSnapshot {
    pub id: Uuid,
    pub stage: BattleStage,
    pub deployment: DeploymentSnapshot,
    pub combat: Option<CombatProgress>,
    pub settlement: Option<SettlementSnapshot>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
/// 只提供本次单战和领奖证明；调用方仍须持久化、核对父计划再登记次数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleProof {
    pub run_id: Uuid,
    pub mission: od_core::simulation_mission::MissionReceipt,
    pub charge_after: CardsCharge,
}

struct Resources {
    root: PathBuf,
    simulation: PathBuf,
    deployment: PathBuf,
    settlement: PathBuf,
    combat: PathBuf,
    manifests: Vec<(PathBuf, Vec<u8>)>,
}
impl Resources {
    fn load(root: &Path, combat: &Path) -> Result<Self> {
        od_core::config::reject_links(root)?;
        let root = root.canonicalize()?;
        let simulation = maa_adapter::simulation::validate_simulation_bundle(
            &root.join("generated/simulation"),
        )?;
        let deployment = maa_adapter::simulation_deploy::validate_simulation_deploy_bundle(
            &root.join("generated/simulation-deploy"),
        )?;
        let settlement = maa_adapter::simulation_settlement::validate_simulation_settlement_bundle(
            &root.join("generated/simulation-settlement"),
        )?;
        let combat = CombatBundle::load(combat)?.root;
        let manifests = [
            simulation.join("simulation-manifest.json"),
            deployment.join("simulation_deploy-manifest.json"),
            settlement.join("simulation_settlement-manifest.json"),
            combat.join("combat-manifest.json"),
        ]
        .into_iter()
        .map(|p| Ok((p.clone(), Sha256::digest(std::fs::read(p)?).to_vec())))
        .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            root,
            simulation,
            deployment,
            settlement,
            combat,
            manifests,
        })
    }
    fn verify(&self) -> Result<()> {
        for (path, hash) in &self.manifests {
            od_core::config::reject_links(path)?;
            ensure!(
                Sha256::digest(std::fs::read(path)?).as_slice() == hash,
                "模拟室父任务冻结资源已变化"
            );
        }
        maa_adapter::simulation::validate_simulation_bundle(&self.simulation)?;
        maa_adapter::simulation_deploy::validate_simulation_deploy_bundle(&self.deployment)?;
        maa_adapter::simulation_settlement::validate_simulation_settlement_bundle(
            &self.settlement,
        )?;
        CombatBundle::load(&self.combat)?;
        Ok(())
    }
}
enum Child {
    Deployment(Box<DeploymentRun>),
    Combat(Box<CombatRun>),
    Settlement(Box<SettlementRun>),
}
pub struct SimulationBattleRun {
    snapshot: BattleSnapshot,
    plan: AppCombatPlan,
    target: BattleTarget,
    resources: Resources,
    timeout_ms: u32,
    control: PipelineControl,
    child: Option<Child>,
    child_started: bool,
    loaded: Option<BattleStage>,
    recognizer: Option<CombatRecognizer>,
    end: Option<BattleEnd>,
}
impl SimulationBattleRun {
    pub fn after_entry(
        entry: EntryRun,
        resources: &Path,
        target: BattleTarget,
        timeout_ms: u32,
    ) -> Result<Self> {
        Self::new(entry.into_deployment()?, resources, target, timeout_ms)
    }
    pub fn new(
        deployment: DeploymentRun,
        resources: &Path,
        target: BattleTarget,
        timeout_ms: u32,
    ) -> Result<Self> {
        ensure!(
            (1000..=900_000).contains(&timeout_ms),
            "模拟室单战父预算无效"
        );
        let (root, plan, control) = deployment.parent_context()?;
        let resources = Resources::load(resources, root)?;
        let plan = plan.clone();
        if matches!(target, BattleTarget::Fixture) {
            ensure!(
                plan.config.mode == InputMode::Foreground && plan.config.directml_device.is_none(),
                "隔离模拟室只复用前台 CPU 控制器，不能替代实际设备交接"
            );
        }
        if let BattleTarget::Game { window, .. } = &target {
            ensure!(
                window.width == 1920 && window.height == 1080,
                "模拟室父任务须冻结 1080p 游戏窗口"
            );
        }
        Ok(Self {
            snapshot: BattleSnapshot {
                id: deployment.id,
                stage: BattleStage::Deployment,
                deployment: deployment.snapshot.clone(),
                combat: None,
                settlement: None,
                active_ms: 0,
                inputs_released: true,
            },
            plan,
            target,
            resources,
            timeout_ms,
            control,
            child: Some(Child::Deployment(Box::new(deployment))),
            child_started: false,
            loaded: None,
            recognizer: None,
            end: None,
        })
    }
    pub fn snapshot(&self) -> &BattleSnapshot {
        &self.snapshot
    }
    pub fn control(&self) -> PipelineRequest {
        self.control.requester()
    }
    pub fn proof(&self) -> Option<BattleProof> {
        (self.end == Some(BattleEnd::Completed)
            && self.snapshot.inputs_released
            && self.control.requested().is_none())
        .then(|| BattleProof {
            run_id: self.snapshot.id,
            mission: self.snapshot.deployment.flow.request().mission.clone(),
            charge_after: self
                .snapshot
                .settlement
                .as_ref()
                .unwrap()
                .flow
                .charge()
                .unwrap(),
        })
    }
    fn interrupted(&self) -> Option<BattleEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => BattleEnd::Paused,
            Interrupt::Stop => BattleEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(BattleEnd::Paused) && self.snapshot.inputs_released,
            "模拟室父流程尚未安全暂停"
        );
        if !self.child_started {
            ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        } else {
            match self.child.as_mut().context("缺少暂停中的模拟室子流程")? {
                Child::Deployment(r) => {
                    r.resume()?;
                    self.snapshot.deployment = r.snapshot.clone();
                }
                Child::Combat(r) => {
                    r.resume_segment_after_parent_pause()?;
                    self.snapshot.combat = Some(r.progress.clone());
                }
                Child::Settlement(r) => {
                    r.resume()?;
                    self.snapshot.settlement = Some(r.snapshot.clone());
                    self.snapshot.stage = BattleStage::Settlement;
                }
            }
        }
        self.end = None;
        Ok(())
    }
    fn session<'a>(
        &mut self,
        slot: &'a mut Option<NativeSession>,
    ) -> Result<Option<&'a mut NativeSession>> {
        let battle = self.snapshot.stage == BattleStage::Combat;
        let mode = if battle {
            self.plan.config.mode
        } else {
            InputMode::Foreground
        };
        let device = if battle {
            self.plan
                .config
                .directml_device
                .map_or(InferenceDevice::Cpu, InferenceDevice::DirectMl)
        } else {
            InferenceDevice::Cpu
        };
        let cancel = self.control.cancellation();
        // 识别器和它的推理取消句柄先归还，再由唯一槽位销毁 / 重建控制器。
        if self.loaded != Some(self.snapshot.stage) {
            self.recognizer = None;
        }
        match &self.target {
            BattleTarget::Fixture => {
                let session = slot.as_mut().context("模拟室隔离控制器缺失")?;
                session.quiesce()?;
                session.check_combat_input_mode(mode)?;
                if cancel.load(Ordering::Acquire) {
                    return Ok(None);
                }
            }
            BattleTarget::Game { window, capture } => {
                match NativeSession::handoff_game(
                    slot,
                    GameSessionConfig {
                        window,
                        resources: &self.resources.root,
                        capture: *capture,
                        input: mode,
                        device,
                    },
                    &cancel,
                )? {
                    SessionHandoff::Cancelled => return Ok(None),
                    SessionHandoff::Created => {
                        self.loaded = None;
                        self.recognizer = None;
                    }
                    SessionHandoff::Reused => {}
                }
            }
        }
        let session = slot.as_mut().context("模拟室控制器交接后缺失")?;
        session.check_combat_input_mode(mode)?;
        if self.loaded != Some(self.snapshot.stage) {
            self.resources.verify()?;
            match self.snapshot.stage {
                BattleStage::Deployment => {
                    session.load_simulation_deploy_bundle(&self.resources.deployment)?;
                }
                BattleStage::Combat => {
                    self.recognizer = Some(session.load_combat_bundle(&self.resources.combat)?);
                }
                BattleStage::Settlement => {
                    session.load_simulation_bundle(&self.resources.simulation)?;
                    session.load_simulation_settlement_bundle(&self.resources.settlement)?;
                }
                BattleStage::Done => anyhow::bail!("完成后不能再交接控制器"),
            }
            self.loaded = Some(self.snapshot.stage);
        }
        Ok(Some(session))
    }
    pub fn execute_with_budget(
        &mut self,
        slot: &mut Option<NativeSession>,
        parent_remaining: Duration,
        mut publish: impl FnMut(BattleSnapshot) -> Result<()>,
    ) -> Result<BattleEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(BattleEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "模拟室父流程不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(u64::from(self.timeout_ms).saturating_sub(initial))
            .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let mut publication_failed = false;
        let mut save = |snapshot| {
            let result = publish(snapshot);
            publication_failed |= result.is_err();
            result
        };
        let result = (|| -> Result<BattleEnd> {
            // 槽位由外层持有，暂停期间可能已经被重建；不能沿用旧句柄的装载缓存。
            self.recognizer = None;
            self.loaded = None;
            if let Some(s) = slot.as_mut() {
                s.quiesce()?;
            }
            self.resources.verify()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(BattleEnd::TimedOut);
                }
                let Some(session) = self.session(slot)? else {
                    return Ok(self.interrupted().unwrap_or(BattleEnd::Stopped));
                };
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(BattleEnd::TimedOut);
                }
                self.child_started = true;
                let snapshot = &mut self.snapshot;
                match self.child.as_mut().context("缺少模拟室当前子流程")? {
                    Child::Deployment(child) => {
                        let result = child.execute_with_budget(
                            session,
                            matches!(self.target, BattleTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.deployment = s;
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.deployment = child.snapshot.clone();
                        snapshot.inputs_released = child.snapshot.inputs_released;
                        let end = result?;
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(BattleEnd::TimedOut);
                        }
                        match end {
                            DeploymentEnd::Entered => {}
                            DeploymentEnd::Paused => return Ok(BattleEnd::Paused),
                            DeploymentEnd::Stopped => return Ok(BattleEnd::Stopped),
                            DeploymentEnd::TimedOut => return Ok(BattleEnd::TimedOut),
                            DeploymentEnd::Failed => anyhow::bail!("模拟室出战失败"),
                        }
                        let Some(Child::Deployment(child)) = self.child.take() else {
                            unreachable!()
                        };
                        let combat = child.into_combat_for_parent()?;
                        self.snapshot.combat = Some(combat.progress.clone());
                        self.child = Some(Child::Combat(Box::new(combat)));
                        self.snapshot.stage = BattleStage::Combat;
                    }
                    Child::Combat(child) => {
                        let result = child.execute_segment_with_budget(
                            session,
                            self.recognizer.as_mut().context("缺少冻结战斗识别器")?,
                            matches!(self.target, BattleTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.combat = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.combat = Some(child.progress.clone());
                        snapshot.inputs_released = child.progress.inputs_released;
                        let end = result?;
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(BattleEnd::TimedOut);
                        }
                        match end {
                            CombatSegmentEnd::Interrupted(Interrupt::Pause) => {
                                return Ok(BattleEnd::Paused);
                            }
                            CombatSegmentEnd::Interrupted(Interrupt::Stop) => {
                                return Ok(BattleEnd::Stopped);
                            }
                            CombatSegmentEnd::Result(NormalBattleResult::Retreat) => {
                                anyhow::bail!("模拟室战斗撤退，不能领奖或登记成功")
                            }
                            CombatSegmentEnd::Result(NormalBattleResult::Completed) => {}
                        }
                        ensure!(
                            self.snapshot.inputs_released,
                            "战斗未释放输入，不能交接领奖"
                        );
                        let settlement = SettlementRun::after_combat(
                            end,
                            true,
                            self.timeout_ms.min(300_000),
                            self.control.clone(),
                        )?;
                        self.snapshot.settlement = Some(settlement.snapshot.clone());
                        self.child = Some(Child::Settlement(Box::new(settlement)));
                        self.snapshot.stage = BattleStage::Settlement;
                    }
                    Child::Settlement(child) => {
                        let result = child.execute_with_budget(
                            session,
                            matches!(self.target, BattleTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.settlement = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.settlement = Some(child.snapshot.clone());
                        snapshot.inputs_released = child.snapshot.inputs_released;
                        let end = result?;
                        let end = match end {
                            SettlementEnd::Collected => BattleEnd::Completed,
                            SettlementEnd::Paused => BattleEnd::Paused,
                            SettlementEnd::Stopped => BattleEnd::Stopped,
                            SettlementEnd::TimedOut => BattleEnd::TimedOut,
                            SettlementEnd::Failed => anyhow::bail!("模拟室领奖失败"),
                        };
                        if end == BattleEnd::Completed {
                            ensure!(
                                self.snapshot
                                    .settlement
                                    .as_ref()
                                    .unwrap()
                                    .flow
                                    .charge()
                                    .is_some_and(|c| c.required_charge
                                        == self
                                            .snapshot
                                            .deployment
                                            .flow
                                            .request()
                                            .mission
                                            .charge
                                            .required_charge),
                                "领奖返回的卡片方案与冻结关卡不一致"
                            );
                        }
                        return Ok(end);
                    }
                }
                self.child_started = false;
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                // 保存阶段交接后才允许切换会话；回调中的控制请求在循环顶部重新检查。
                save(self.snapshot.clone())?;
            }
        })();
        self.end = Some(BattleEnd::Failed);
        self.snapshot.inputs_released = if let Some(s) = slot.as_mut() {
            s.quiesce().is_ok()
        } else {
            self.snapshot.inputs_released
        };
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        ensure!(
            self.snapshot.inputs_released,
            "模拟室父流程收尾未确认输入释放"
        );
        let mut end = match result {
            Ok(end) => end,
            Err(_) if !publication_failed && remaining().is_zero() => BattleEnd::TimedOut,
            Err(error) => return Err(error),
        };
        publish(self.snapshot.clone())?;
        if let Some(request) = self.interrupted() {
            end = request;
        } else if remaining().is_zero() {
            end = BattleEnd::TimedOut;
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if end == BattleEnd::Paused && !self.control.acknowledge_pause() {
            end = BattleEnd::Stopped;
        }
        if end == BattleEnd::Completed {
            self.snapshot.stage = BattleStage::Done;
        }
        self.end = Some(end);
        Ok(end)
    }
}
