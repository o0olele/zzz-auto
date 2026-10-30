//! 已验证关卡之后的准备、选队、战斗与领奖；不生成关卡证明或登记运行次数。
use crate::{
    simulation_battle::{
        BattleEnd, BattleProof, BattleSnapshot, BattleTarget, Resources as BattleResources,
        SimulationBattleRun,
    },
    simulation_deploy::PreparedDeployment,
    simulation_entry::{EntryEnd, EntryRun, EntrySnapshot},
    simulation_preparation::{PreparationEnd, PreparationRun, PreparationSnapshot},
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{GameSessionConfig, InferenceDevice, NativeSession, SessionHandoff};
use od_core::{
    InputMode,
    charge_plan::{ChargePlanItem, RestoreCharge},
    pipeline::{Interrupt, PipelineControl, PipelineRequest},
    predefined_teams::TeamConfig,
    simulation_entry::EntryRequest,
    simulation_mission::{MissionReceipt, MissionRequest},
    simulation_preparation::{
        PreparationFlow, PreparationOutcome, PreparationRequest, PreparationResult,
    },
    simulation_teams::TeamsRequest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeRequest {
    pub plan: ChargePlanItem,
    pub mission: MissionReceipt,
    pub restore_charge: RestoreCharge,
    pub teams: TeamConfig,
    pub timeout_ms: u32,
}
impl ChallengeRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        let expected = MissionRequest::from_plan(&self.plan, self.timeout_ms)?;
        PreparationRequest::from_plan(&self.plan, self.restore_charge, self.timeout_ms)?;
        TeamsRequest::from_plan(&self.plan, &self.teams, self.timeout_ms)?;
        self.mission.charge.validate()?;
        if !self.mission.selection_verified
            || self.mission.mission_type != expected.mission_type
            || self.mission.mission_name != expected.mission_name
        {
            return Err("完整挑战缺少本次冻结计划的关卡选中证明");
        }
        Ok(())
    }
    fn entry(&self, prepared: PreparationResult) -> EntryRequest {
        EntryRequest {
            plan: self.plan.clone(),
            mission: self.mission.clone(),
            prepared,
            teams: self.teams.clone(),
            timeout_ms: self.timeout_ms,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeStage {
    Preparation,
    Entry,
    Battle,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeEnd {
    Completed,
    InsufficientCharge,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeSnapshot {
    pub id: Uuid,
    pub request: ChallengeRequest,
    pub stage: ChallengeStage,
    pub preparation: PreparationSnapshot,
    pub entry: Option<EntrySnapshot>,
    pub battle: Option<BattleSnapshot>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChallengeProof {
    pub execution_id: Uuid,
    pub run_id: Uuid,
    pub plan: ChargePlanItem,
    pub battle: BattleProof,
}
struct Resources {
    root: PathBuf,
    simulation: PathBuf,
    restore: PathBuf,
    daily: PathBuf,
    battle: BattleResources,
    manifests: Vec<(PathBuf, Vec<u8>)>,
}
impl Resources {
    fn load(root: &Path, combat: &Path) -> Result<Self> {
        od_core::config::reject_links(root)?;
        let battle = BattleResources::load(root, combat)?;
        let root = root.canonicalize()?;
        let simulation = root.join("generated/simulation");
        let restore = maa_adapter::restore_charge::validate_restore_charge_bundle(
            &root.join("generated/restore-charge"),
        )?;
        let daily = maa_adapter::validate_basic_daily_bundle(&root.join("generated/basic-daily"))?;
        let manifests = [
            restore.join("restore-charge-manifest.json"),
            daily.join("basic-daily-manifest.json"),
        ]
        .into_iter()
        .map(|p| Ok((p.clone(), Sha256::digest(std::fs::read(p)?).to_vec())))
        .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            root,
            simulation,
            restore,
            daily,
            battle,
            manifests,
        })
    }
    fn verify(&self) -> Result<()> {
        self.battle.verify()?;
        for (path, hash) in &self.manifests {
            od_core::config::reject_links(path)?;
            ensure!(
                Sha256::digest(std::fs::read(path)?).as_slice() == hash,
                "完整挑战的冻结资源已变化"
            );
        }
        maa_adapter::restore_charge::validate_restore_charge_bundle(&self.restore)?;
        maa_adapter::validate_basic_daily_bundle(&self.daily)?;
        Ok(())
    }
}
enum Child {
    Preparation(Box<PreparationRun>),
    Entry(Box<EntryRun>),
    Battle(Box<SimulationBattleRun>),
}
pub struct SimulationChallengeRun {
    execution_id: Uuid,
    snapshot: ChallengeSnapshot,
    resources: Resources,
    target: Option<BattleTarget>,
    prepared: Option<PreparedDeployment>,
    control: PipelineControl,
    child: Option<Child>,
    child_started: bool,
    end: Option<ChallengeEnd>,
}
impl SimulationChallengeRun {
    pub fn new(
        id: Uuid,
        request: ChallengeRequest,
        prepared: PreparedDeployment,
        resources: &Path,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<Self> {
        request.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            prepared.plan().config.script
                == TeamsRequest::from_plan(&request.plan, &request.teams, request.timeout_ms)
                    .map_err(anyhow::Error::msg)?
                    .script_name,
            "完整挑战的冻结脚本与编队不一致"
        );
        if matches!(target, BattleTarget::Fixture) {
            ensure!(
                prepared.plan().config.mode == InputMode::Foreground
                    && prepared.plan().config.directml_device.is_none(),
                "隔离挑战不能替代手柄或 DirectML 的实际交接"
            );
        }
        if let BattleTarget::Game { window, .. } = &target {
            ensure!(
                window.width == 1920 && window.height == 1080,
                "完整挑战须冻结 1080p 游戏窗口"
            );
        }
        let resources = Resources::load(resources, prepared.resource_root()?)?;
        let preparation = PreparationRun::new(
            PreparationFlow::new(
                PreparationRequest::from_plan(
                    &request.plan,
                    request.restore_charge,
                    request.timeout_ms,
                )
                .map_err(anyhow::Error::msg)?,
            )
            .map_err(anyhow::Error::msg)?,
            control.clone(),
        )?;
        Ok(Self {
            execution_id: Uuid::new_v4(),
            snapshot: ChallengeSnapshot {
                id,
                request,
                stage: ChallengeStage::Preparation,
                preparation: preparation.snapshot.clone(),
                entry: None,
                battle: None,
                active_ms: 0,
                inputs_released: true,
            },
            resources,
            target: Some(target),
            prepared: Some(prepared),
            control,
            child: Some(Child::Preparation(Box::new(preparation))),
            child_started: false,
            end: None,
        })
    }
    pub fn snapshot(&self) -> &ChallengeSnapshot {
        &self.snapshot
    }
    pub(crate) fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub(crate) fn execution_id(&self) -> Uuid {
        self.execution_id
    }
    pub fn control(&self) -> PipelineRequest {
        self.control.requester()
    }
    pub fn proof(&self) -> Option<ChallengeProof> {
        if self.end != Some(ChallengeEnd::Completed)
            || !self.snapshot.inputs_released
            || self.control.requested().is_some()
        {
            return None;
        }
        let Child::Battle(battle) = self.child.as_ref()? else {
            return None;
        };
        Some(ChallengeProof {
            execution_id: self.execution_id,
            run_id: self.snapshot.id,
            plan: self.snapshot.request.plan.clone(),
            battle: battle.proof()?,
        })
    }
    pub fn shortage(&self) -> Option<PreparationResult> {
        (self.end == Some(ChallengeEnd::InsufficientCharge)
            && self.snapshot.inputs_released
            && self.control.requested().is_none())
        .then(|| self.snapshot.preparation.flow.result())
        .flatten()
    }
    fn interrupted(&self) -> Option<ChallengeEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => ChallengeEnd::Paused,
            Interrupt::Stop => ChallengeEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(ChallengeEnd::Paused) && self.snapshot.inputs_released,
            "完整挑战尚未安全暂停"
        );
        if !self.child_started {
            ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        } else {
            match self.child.as_mut().context("缺少暂停中的挑战子流程")? {
                Child::Preparation(r) => {
                    r.resume_after_parent_pause()?;
                    self.snapshot.preparation = r.snapshot.clone();
                    self.snapshot.stage = ChallengeStage::Preparation;
                }
                Child::Entry(r) => {
                    r.resume_after_parent_pause()?;
                    self.snapshot.entry = Some(r.snapshot().clone());
                }
                Child::Battle(r) => {
                    r.resume_after_parent_pause()?;
                    self.snapshot.battle = Some(r.snapshot().clone());
                    self.snapshot.stage = ChallengeStage::Battle;
                }
            }
        }
        self.end = None;
        Ok(())
    }
    fn interface_session<'a>(
        &self,
        slot: &'a mut Option<NativeSession>,
    ) -> Result<Option<&'a mut NativeSession>> {
        match self.target.as_ref().context("缺少挑战界面目标")? {
            BattleTarget::Game { window, capture } => {
                if NativeSession::handoff_game(
                    slot,
                    GameSessionConfig {
                        window,
                        resources: &self.resources.root,
                        capture: *capture,
                        input: InputMode::Foreground,
                        device: InferenceDevice::Cpu,
                    },
                    &self.control.cancellation(),
                )? == SessionHandoff::Cancelled
                {
                    return Ok(None);
                }
            }
            BattleTarget::Fixture => {
                let s = slot.as_mut().context("完整挑战隔离控制器缺失")?;
                s.quiesce()?;
                s.check_combat_input_mode(InputMode::Foreground)?;
            }
        }
        let s = slot.as_mut().context("完整挑战原生会话缺失")?;
        s.load_simulation_bundle(&self.resources.simulation)?;
        match self.snapshot.stage {
            ChallengeStage::Preparation => s.load_restore_charge_bundle(&self.resources.restore)?,
            ChallengeStage::Entry => s.load_basic_daily_bundle(&self.resources.daily)?,
            _ => anyhow::bail!("挑战阶段不能接管界面会话"),
        }
        Ok(Some(s))
    }
    pub fn execute_with_budget(
        &mut self,
        slot: &mut Option<NativeSession>,
        parent_remaining: Duration,
        mut publish: impl FnMut(ChallengeSnapshot) -> Result<()>,
    ) -> Result<ChallengeEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(ChallengeEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "完整挑战不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.request.timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let mut publication_failed = false;
        let mut save = |s| {
            let result = publish(s);
            publication_failed |= result.is_err();
            result
        };
        let result = (|| -> Result<ChallengeEnd> {
            if let Some(s) = slot.as_mut() {
                s.quiesce()?;
            }
            self.resources.verify()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(ChallengeEnd::TimedOut);
                }
                if self.snapshot.stage == ChallengeStage::Battle {
                    self.child_started = true;
                    let snapshot = &mut self.snapshot;
                    let Some(Child::Battle(child)) = &mut self.child else {
                        anyhow::bail!("缺少挑战单战流程");
                    };
                    let result = child.execute_with_budget(slot, remaining(), |s| {
                        snapshot.inputs_released = s.inputs_released;
                        snapshot.battle = Some(s);
                        snapshot.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        save(snapshot.clone())
                    });
                    snapshot.inputs_released = child.snapshot().inputs_released;
                    snapshot.battle = Some(child.snapshot().clone());
                    let end = match result? {
                        BattleEnd::Completed => ChallengeEnd::Completed,
                        BattleEnd::Paused => ChallengeEnd::Paused,
                        BattleEnd::Stopped => ChallengeEnd::Stopped,
                        BattleEnd::TimedOut => ChallengeEnd::TimedOut,
                        BattleEnd::Failed => anyhow::bail!("完整挑战的单战失败"),
                    };
                    if end == ChallengeEnd::Completed {
                        let proof = child
                            .proof_for_parent()
                            .context("完整挑战缺少正常单战证明")?;
                        let prepared = snapshot
                            .preparation
                            .flow
                            .result()
                            .context("缺少挑战准备结果")?;
                        ensure!(
                            proof.run_id == snapshot.id
                                && proof.mission.selection_verified
                                && proof.mission.mission_type
                                    == snapshot.request.mission.mission_type
                                && proof.mission.mission_name
                                    == snapshot.request.mission.mission_name
                                && proof.charge_after.required_charge
                                    == prepared.charge.required_charge,
                            "完整挑战的单战身份或数量与冻结计划不一致"
                        );
                        snapshot.stage = ChallengeStage::Done;
                    }
                    return Ok(end);
                }
                let Some(s) = self.interface_session(slot)? else {
                    return Ok(self.interrupted().unwrap_or(ChallengeEnd::Stopped));
                };
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(ChallengeEnd::TimedOut);
                }
                self.child_started = true;
                let snapshot = &mut self.snapshot;
                match self.child.as_mut().context("缺少挑战当前子流程")? {
                    Child::Preparation(child) => {
                        let result = child.execute_with_budget(
                            s,
                            matches!(self.target, Some(BattleTarget::Game { .. })),
                            remaining(),
                            |v| {
                                snapshot.inputs_released = v.inputs_released;
                                snapshot.preparation = v;
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.inputs_released = child.snapshot.inputs_released;
                        snapshot.preparation = child.snapshot.clone();
                        let end = result?;
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(ChallengeEnd::TimedOut);
                        }
                        match end {
                            PreparationEnd::Completed => {}
                            PreparationEnd::Paused => return Ok(ChallengeEnd::Paused),
                            PreparationEnd::Stopped => return Ok(ChallengeEnd::Stopped),
                            PreparationEnd::TimedOut => return Ok(ChallengeEnd::TimedOut),
                            PreparationEnd::Failed => anyhow::bail!("完整挑战准备失败"),
                        }
                        let prepared = self
                            .snapshot
                            .preparation
                            .flow
                            .result()
                            .context("完整挑战缺少实际准备结果")?;
                        ensure!(self.snapshot.inputs_released, "挑战准备未释放输入");
                        if prepared.outcome != PreparationOutcome::Ready {
                            self.snapshot.stage = ChallengeStage::Done;
                            return Ok(ChallengeEnd::InsufficientCharge);
                        }
                        let entry = EntryRun::after_preparation(
                            self.snapshot.id,
                            self.snapshot.request.entry(prepared),
                            end,
                            &self.snapshot.preparation,
                            self.prepared.take().context("挑战冻结脚本已被采用")?,
                            self.control.clone(),
                        )?;
                        self.snapshot.entry = Some(entry.snapshot().clone());
                        self.child = Some(Child::Entry(Box::new(entry)));
                        self.snapshot.stage = ChallengeStage::Entry;
                    }
                    Child::Entry(child) => {
                        let result = child.execute_with_budget(
                            s,
                            matches!(self.target, Some(BattleTarget::Game { .. })),
                            remaining(),
                            |v| {
                                snapshot.inputs_released = v.inputs_released;
                                snapshot.entry = Some(v);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.inputs_released = child.snapshot().inputs_released;
                        snapshot.entry = Some(child.snapshot().clone());
                        let end = result?;
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(ChallengeEnd::TimedOut);
                        }
                        match end {
                            EntryEnd::Ready => {}
                            EntryEnd::Paused => return Ok(ChallengeEnd::Paused),
                            EntryEnd::Stopped => return Ok(ChallengeEnd::Stopped),
                            EntryEnd::TimedOut => return Ok(ChallengeEnd::TimedOut),
                            EntryEnd::Failed => anyhow::bail!("完整挑战的详情推进失败"),
                        }
                        let Some(Child::Entry(entry)) = self.child.take() else {
                            unreachable!()
                        };
                        let battle = SimulationBattleRun::new(
                            entry.into_deployment_for_parent()?,
                            &self.resources.root,
                            self.target.take().context("挑战目标已被采用")?,
                            self.snapshot.request.timeout_ms,
                        )?;
                        self.snapshot.battle = Some(battle.snapshot().clone());
                        self.child = Some(Child::Battle(Box::new(battle)));
                        self.snapshot.stage = ChallengeStage::Battle;
                    }
                    Child::Battle(_) => unreachable!(),
                }
                self.child_started = false;
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                save(self.snapshot.clone())?;
                self.resources.verify()?;
            }
        })();
        self.end = Some(ChallengeEnd::Failed);
        self.snapshot.inputs_released = if let Some(s) = slot.as_mut() {
            s.quiesce().is_ok()
        } else {
            self.snapshot.inputs_released
        };
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        ensure!(self.snapshot.inputs_released, "完整挑战收尾未确认输入释放");
        let mut end = match result {
            Ok(end) => end,
            Err(_) if !publication_failed && remaining().is_zero() => ChallengeEnd::TimedOut,
            Err(error) => return Err(error),
        };
        publish(self.snapshot.clone())?;
        if let Some(request) = self.interrupted() {
            end = request;
        } else if remaining().is_zero() {
            end = ChallengeEnd::TimedOut;
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if end == ChallengeEnd::Paused && !self.control.acknowledge_pause() {
            end = ChallengeEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
