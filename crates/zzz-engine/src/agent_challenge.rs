//! 代理人导航与已知具体挑战复用同一会话，私有结果仍绑定原计划。
use crate::{
    agent_transport::{AgentTransportEnd, AgentTransportRun, AgentTransportSnapshot},
    area_patrol::{AreaPatrolRun, PatrolEnd, PatrolRequest, PatrolResources, PatrolSnapshot},
    charge_transport::AgentRouteHandoff,
    deep_hunt::{DeepHuntRun, HuntEnd, HuntRequest, HuntResources, HuntSnapshot},
    expert_challenge::{
        ExpertChallengeRun, ExpertEnd, ExpertRequest, ExpertResources, ExpertSnapshot,
    },
    simulation_battle::BattleTarget,
    simulation_deploy::PreparedDeployment,
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{GameSessionConfig, InferenceDevice, NativeSession, SessionHandoff};
pub use od_core::agent_challenge::{AgentChallengeRequest, AgentChallengeResult};
use od_core::{
    InputMode,
    agent_transport::{AgentTransportRequest, AgentTransportTarget},
    charge_plan::ChargePlanItem,
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentChallengeStage {
    Transport,
    Challenge,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentChallengeEnd {
    Completed,
    Unavailable,
    InsufficientCharge,
    WeeklyRewardsAvailable,
    AwaitingVerification,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentChallengeSnapshot {
    pub id: Uuid,
    pub request: AgentChallengeRequest,
    pub stage: AgentChallengeStage,
    pub navigation: AgentTransportSnapshot,
    pub resolved_plan: Option<ChargePlanItem>,
    pub patrol: Option<PatrolSnapshot>,
    pub expert: Option<ExpertSnapshot>,
    pub hunt: Option<HuntSnapshot>,
    pub result: Option<AgentChallengeResult>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentChallengeProof {
    pub run_id: Uuid,
    pub execution_id: Uuid,
    pub plan: ChargePlanItem,
    pub target: Option<AgentTransportTarget>,
    pub result: AgentChallengeResult,
}
#[derive(Clone)]
enum UnderlyingResources {
    Patrol(PatrolResources),
    Expert(ExpertResources),
    Hunt(HuntResources),
}
#[derive(Clone)]
pub(crate) struct AgentResources {
    root: PathBuf,
    combat_root: PathBuf,
    navigation: PathBuf,
    goal: PathBuf,
    underlying: UnderlyingResources,
    hashes: Vec<(PathBuf, Vec<u8>)>,
}
impl AgentResources {
    pub(crate) fn load(
        request: &AgentChallengeRequest,
        root: &Path,
        combat: &Path,
    ) -> Result<Self> {
        request.validate().map_err(anyhow::Error::msg)?;
        od_core::config::reject_links(root)?;
        let root = root.canonicalize()?;
        od_core::config::reject_links(combat)?;
        let combat_root = combat.canonicalize()?;
        let navigation = maa_adapter::agent_transport::validate_agent_transport_bundle(
            &root.join("generated/agent-transport"),
        )?;
        let goal = maa_adapter::agent_goal::validate_agent_goal_bundle(
            &root.join("generated/agent-goal"),
        )?;
        let underlying = match request.plan.category_name.as_str() {
            "区域巡防" => UnderlyingResources::Patrol(PatrolResources::load(&root, combat)?),
            "专业挑战室" => UnderlyingResources::Expert(ExpertResources::load(&root, combat)?),
            "恶名狩猎" => UnderlyingResources::Hunt(HuntResources::load(
                &root,
                combat,
                request.plan.level != "默认等级",
            )?),
            _ => anyhow::bail!("代理人挑战分类尚未接入"),
        };
        let hashes = [
            navigation.join("agent_transport-manifest.json"),
            goal.join("agent_goal-manifest.json"),
        ]
        .into_iter()
        .map(|p| Ok((p.clone(), Sha256::digest(std::fs::read(p)?).to_vec())))
        .collect::<Result<_>>()?;
        Ok(Self {
            root,
            combat_root,
            navigation,
            goal,
            underlying,
            hashes,
        })
    }
    pub(crate) fn verify(&self) -> Result<()> {
        for (p, hash) in &self.hashes {
            od_core::config::reject_links(p)?;
            ensure!(
                Sha256::digest(std::fs::read(p)?).as_slice() == hash,
                "代理人冻结资源清单已改变"
            );
        }
        maa_adapter::agent_transport::validate_agent_transport_bundle(&self.navigation)?;
        maa_adapter::agent_goal::validate_agent_goal_bundle(&self.goal)?;
        match &self.underlying {
            UnderlyingResources::Patrol(r) => r.verify(),
            UnderlyingResources::Expert(r) => r.verify(),
            UnderlyingResources::Hunt(r) => r.verify(),
        }
    }
}
enum Child {
    Transport(Box<AgentTransportRun>),
    Patrol(Box<AreaPatrolRun>),
    Expert(Box<ExpertChallengeRun>),
    Hunt(Box<DeepHuntRun>),
}
pub struct AgentChallengeRun {
    snapshot: AgentChallengeSnapshot,
    execution_id: Uuid,
    resources: AgentResources,
    prepared: Option<PreparedDeployment>,
    target: BattleTarget,
    control: PipelineControl,
    child: Child,
    child_started: bool,
    nav_loaded: bool,
    end: Option<AgentChallengeEnd>,
    captured: Option<AgentChallengeProof>,
}
impl AgentChallengeRun {
    pub fn new(
        id: Uuid,
        request: AgentChallengeRequest,
        root: &Path,
        prepared: PreparedDeployment,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<Self> {
        let resources = AgentResources::load(&request, root, prepared.resource_root()?)?;
        Self::from_frozen(id, request, resources, prepared, target, control)
    }
    pub(crate) fn from_frozen(
        id: Uuid,
        request: AgentChallengeRequest,
        resources: AgentResources,
        prepared: PreparedDeployment,
        target: BattleTarget,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(!id.is_nil(), "代理人挑战缺少运行身份");
        request.validate().map_err(anyhow::Error::msg)?;
        resources.verify()?;
        ensure!(
            resources.combat_root == prepared.resource_root()?,
            "代理人冻结战斗资源与脚本不一致"
        );
        ensure!(
            matches!(
                (&resources.underlying, request.plan.category_name.as_str()),
                (UnderlyingResources::Patrol(_), "区域巡防")
                    | (UnderlyingResources::Expert(_), "专业挑战室")
                    | (UnderlyingResources::Hunt(_), "恶名狩猎")
            ),
            "代理人挑战分类与冻结资源不符"
        );
        ensure!(
            prepared.plan().config.script
                == request
                    .plan
                    .script(&request.teams)
                    .map_err(anyhow::Error::msg)?,
            "代理人战斗脚本与原计划不符"
        );
        if matches!(target, BattleTarget::Fixture) {
            ensure!(
                prepared.plan().config.mode == InputMode::Foreground
                    && prepared.plan().config.directml_device.is_none(),
                "隔离代理人挑战只允许前台 CPU"
            );
        }
        if let BattleTarget::Game { window, .. } = &target {
            ensure!(
                window.width == 1920 && window.height == 1080,
                "代理人挑战须冻结 1080p 窗口"
            );
        }
        let navigation = AgentTransportRun::new(
            AgentTransportRequest {
                plan: request.plan.clone(),
                timeout_ms: request.timeout_ms,
            },
            control.clone(),
        )?;
        Ok(Self {
            snapshot: AgentChallengeSnapshot {
                id,
                request,
                stage: AgentChallengeStage::Transport,
                navigation: navigation.snapshot(),
                resolved_plan: None,
                patrol: None,
                expert: None,
                hunt: None,
                result: None,
                active_ms: 0,
                inputs_released: true,
            },
            execution_id: Uuid::new_v4(),
            resources,
            prepared: Some(prepared),
            target,
            control,
            child: Child::Transport(Box::new(navigation)),
            child_started: false,
            nav_loaded: false,
            end: None,
            captured: None,
        })
    }
    pub fn snapshot(&self) -> AgentChallengeSnapshot {
        self.snapshot.clone()
    }
    pub fn uses_control(&self, c: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &c.cancellation())
    }
    pub fn proof(&self) -> Option<AgentChallengeProof> {
        if self.control.requested().is_some() {
            None
        } else {
            self.result_for_parent()
        }
    }
    pub(crate) fn result_for_parent(&self) -> Option<AgentChallengeProof> {
        let fact = self.captured.as_ref()?;
        (self.snapshot.stage == AgentChallengeStage::Done
            && self.snapshot.inputs_released
            && self.snapshot.result.as_ref() == Some(&fact.result))
        .then(|| fact.clone())
    }
    fn capture(&mut self, result: AgentChallengeResult) -> Result<()> {
        ensure!(
            self.captured.is_none() && self.snapshot.inputs_released,
            "代理人结果重复或输入尚未释放"
        );
        let target = self.snapshot.navigation.flow.target().cloned();
        if result != AgentChallengeResult::Unavailable {
            ensure!(
                target.is_some() && self.snapshot.resolved_plan.is_some(),
                "代理人正常结果缺少本次具体目标"
            );
        }
        self.captured = Some(AgentChallengeProof {
            run_id: self.snapshot.id,
            execution_id: self.execution_id,
            plan: self.snapshot.request.plan.clone(),
            target,
            result: result.clone(),
        });
        self.snapshot.result = Some(result);
        self.snapshot.stage = AgentChallengeStage::Done;
        Ok(())
    }
    fn result_end(&self) -> AgentChallengeEnd {
        match self.snapshot.result {
            Some(AgentChallengeResult::Completed { .. }) => AgentChallengeEnd::Completed,
            Some(AgentChallengeResult::Unavailable) => AgentChallengeEnd::Unavailable,
            Some(AgentChallengeResult::Insufficient { .. }) => {
                AgentChallengeEnd::InsufficientCharge
            }
            Some(AgentChallengeResult::WeeklyRewardsAvailable { .. }) => {
                AgentChallengeEnd::WeeklyRewardsAvailable
            }
            None => AgentChallengeEnd::Failed,
        }
    }
    fn interrupted(&self) -> Option<AgentChallengeEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => AgentChallengeEnd::Paused,
            Interrupt::Stop => AgentChallengeEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(AgentChallengeEnd::Paused) && self.snapshot.inputs_released,
            "代理人挑战尚未安全暂停"
        );
        if self.captured.is_some() || !self.child_started {
            ensure!(self.control.resume(), "停止不能继续");
        } else {
            match &mut self.child {
                Child::Transport(c) => c.resume_after_parent_pause()?,
                Child::Patrol(c) => c.resume()?,
                Child::Expert(c) => c.resume()?,
                Child::Hunt(c) => c.resume()?,
            }
        }
        self.end = None;
        self.nav_loaded = false;
        Ok(())
    }
    fn navigation_session<'a>(
        &mut self,
        slot: &'a mut Option<NativeSession>,
    ) -> Result<Option<&'a mut NativeSession>> {
        match &self.target {
            BattleTarget::Fixture => {
                let n = slot.as_mut().context("代理人隔离会话缺失")?;
                n.quiesce()?;
                n.check_ether_synthesis_target(false)?;
            }
            BattleTarget::Game { window, capture } => match NativeSession::handoff_game(
                slot,
                GameSessionConfig {
                    window,
                    resources: &self.resources.root,
                    capture: *capture,
                    input: InputMode::Foreground,
                    device: InferenceDevice::Cpu,
                },
                &self.control.cancellation(),
            )? {
                SessionHandoff::Cancelled => return Ok(None),
                SessionHandoff::Created => self.nav_loaded = false,
                SessionHandoff::Reused => (),
            },
        }
        let n = slot.as_mut().context("代理人会话交接后缺失")?;
        n.check_combat_input_mode(InputMode::Foreground)?;
        if self
            .control
            .cancellation()
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(None);
        }
        if !self.nav_loaded {
            self.resources.verify()?;
            n.clear_agent_goal_bundle()?;
            n.load_agent_transport_bundle(&self.resources.navigation)?;
            self.nav_loaded = true;
        }
        Ok(Some(n))
    }
    pub fn execute_with_budget(
        &mut self,
        slot: &mut Option<NativeSession>,
        parent_remaining: Duration,
        mut publish: impl FnMut(AgentChallengeSnapshot) -> Result<()>,
    ) -> Result<AgentChallengeEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(AgentChallengeEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "代理人挑战不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.request.timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<AgentChallengeEnd> {
            if let Some(n) = slot.as_mut() {
                n.quiesce()?;
            }
            self.resources.verify()?;
            self.nav_loaded = false;
            loop {
                if self.captured.is_some() {
                    return Ok(self.result_end());
                }
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(AgentChallengeEnd::TimedOut);
                }
                if self.snapshot.stage == AgentChallengeStage::Transport {
                    let game = matches!(self.target, BattleTarget::Game { .. });
                    let Some(n) = self.navigation_session(slot)? else {
                        return Ok(self.interrupted().unwrap_or(AgentChallengeEnd::Stopped));
                    };
                    self.child_started = true;
                    let Child::Transport(c) = &mut self.child else {
                        anyhow::bail!("代理人导航阶段不一致");
                    };
                    let snapshot = &mut self.snapshot;
                    let end = c.execute_with_budget(n, game, remaining(), |s| {
                        snapshot.inputs_released = s.inputs_released;
                        snapshot.navigation = s;
                        snapshot.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        publish(snapshot.clone())
                    })?;
                    self.snapshot.navigation = c.snapshot();
                    self.snapshot.inputs_released = c.snapshot().inputs_released;
                    match end {
                        AgentTransportEnd::Unavailable => {
                            ensure!(c.snapshot().flow.unavailable(), "代理人未找到结果尚未确认");
                            self.capture(AgentChallengeResult::Unavailable)?;
                        }
                        AgentTransportEnd::AwaitingVerification => {
                            let receipt = c
                                .receipt_for_parent()
                                .context("代理人导航未正常保存或释放")?;
                            let resolved = receipt.resolved_plan().map_err(anyhow::Error::msg)?;
                            ensure!(
                                receipt.plan == self.snapshot.request.plan,
                                "代理人导航改写了原计划"
                            );
                            let request = &self.snapshot.request;
                            let prepared =
                                self.prepared.take().context("代理人冻结战斗脚本缺失")?;
                            let handoff = AgentRouteHandoff {
                                route: c,
                                goal_bundle: self.resources.goal.clone(),
                            };
                            let child = match &self.resources.underlying {
                                UnderlyingResources::Patrol(r) => {
                                    Child::Patrol(Box::new(AreaPatrolRun::after_agent_transport(
                                        self.snapshot.id,
                                        PatrolRequest {
                                            plan: resolved.clone(),
                                            restore_charge: request.restore_charge,
                                            teams: request.teams.clone(),
                                            timeout_ms: request.timeout_ms,
                                        },
                                        r.clone(),
                                        prepared,
                                        self.target.clone(),
                                        self.control.clone(),
                                        handoff,
                                    )?))
                                }
                                UnderlyingResources::Expert(r) => Child::Expert(Box::new(
                                    ExpertChallengeRun::after_agent_transport(
                                        self.snapshot.id,
                                        ExpertRequest {
                                            plan: resolved.clone(),
                                            restore_charge: request.restore_charge,
                                            teams: request.teams.clone(),
                                            timeout_ms: request.timeout_ms,
                                        },
                                        r.clone(),
                                        prepared,
                                        self.target.clone(),
                                        self.control.clone(),
                                        handoff,
                                    )?,
                                )),
                                UnderlyingResources::Hunt(r) => {
                                    Child::Hunt(Box::new(DeepHuntRun::after_agent_transport(
                                        self.snapshot.id,
                                        HuntRequest {
                                            plan: resolved.clone(),
                                            restore_charge: request.restore_charge,
                                            teams: request.teams.clone(),
                                            timeout_ms: request.timeout_ms,
                                        },
                                        r.clone(),
                                        prepared,
                                        self.target.clone(),
                                        self.control.clone(),
                                        handoff,
                                    )?))
                                }
                            };
                            match &child {
                                Child::Patrol(c) => self.snapshot.patrol = Some(c.snapshot()),
                                Child::Expert(c) => self.snapshot.expert = Some(c.snapshot()),
                                Child::Hunt(c) => self.snapshot.hunt = Some(c.snapshot()),
                                Child::Transport(_) => unreachable!(),
                            }
                            self.snapshot.resolved_plan = Some(resolved);
                            self.snapshot.stage = AgentChallengeStage::Challenge;
                            self.child = child;
                            self.child_started = false;
                        }
                        AgentTransportEnd::Paused => return Ok(AgentChallengeEnd::Paused),
                        AgentTransportEnd::Stopped => return Ok(AgentChallengeEnd::Stopped),
                        AgentTransportEnd::TimedOut => return Ok(AgentChallengeEnd::TimedOut),
                        AgentTransportEnd::Failed => anyhow::bail!("代理人导航失败"),
                    }
                } else {
                    self.child_started = true;
                    let expected = self
                        .snapshot
                        .resolved_plan
                        .as_ref()
                        .context("代理人挑战具体目标缺失")?
                        .clone();
                    let snapshot = &mut self.snapshot;
                    let (fact, end) = match &mut self.child {
                        Child::Patrol(c) => {
                            let end = c.execute_with_budget(slot, remaining(), |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.patrol = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                publish(snapshot.clone())
                            });
                            snapshot.patrol = Some(c.snapshot());
                            snapshot.inputs_released = c.snapshot().inputs_released;
                            let fact = if let Some((proof, goal)) = c.agent_completion_for_parent()
                            {
                                ensure!(
                                    proof.run_id == snapshot.id && proof.plan == expected,
                                    "代理人区域结果不属于本次具体目标"
                                );
                                Some(AgentChallengeResult::Completed {
                                    charge_after: proof.charge_after,
                                    goal,
                                })
                            } else {
                                c.shortage_for_parent()
                                    .map(|charge| AgentChallengeResult::Insufficient { charge })
                            };
                            let end = match end {
                                Ok(PatrolEnd::Completed | PatrolEnd::InsufficientCharge) => {
                                    Ok(None)
                                }
                                Ok(PatrolEnd::Paused) => Ok(Some(AgentChallengeEnd::Paused)),
                                Ok(PatrolEnd::Stopped) => Ok(Some(AgentChallengeEnd::Stopped)),
                                Ok(PatrolEnd::TimedOut) => Ok(Some(AgentChallengeEnd::TimedOut)),
                                Ok(PatrolEnd::Failed) => Err(anyhow::anyhow!("代理人区域挑战失败")),
                                Err(e) => Err(e),
                            };
                            (fact, end)
                        }
                        Child::Expert(c) => {
                            let end = c.execute_with_budget(slot, remaining(), |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.expert = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                publish(snapshot.clone())
                            });
                            snapshot.expert = Some(c.snapshot());
                            snapshot.inputs_released = c.snapshot().inputs_released;
                            let fact = if let Some((proof, goal)) = c.agent_completion_for_parent()
                            {
                                ensure!(
                                    proof.run_id == snapshot.id && proof.plan == expected,
                                    "代理人专业结果不属于本次具体目标"
                                );
                                Some(AgentChallengeResult::Completed {
                                    charge_after: proof.charge_after,
                                    goal,
                                })
                            } else {
                                c.shortage_for_parent()
                                    .map(|charge| AgentChallengeResult::Insufficient { charge })
                            };
                            let end = match end {
                                Ok(ExpertEnd::Completed | ExpertEnd::InsufficientCharge) => {
                                    Ok(None)
                                }
                                Ok(ExpertEnd::Paused) => Ok(Some(AgentChallengeEnd::Paused)),
                                Ok(ExpertEnd::Stopped) => Ok(Some(AgentChallengeEnd::Stopped)),
                                Ok(ExpertEnd::TimedOut) => Ok(Some(AgentChallengeEnd::TimedOut)),
                                Ok(ExpertEnd::AwaitingVerification) => {
                                    Ok(Some(AgentChallengeEnd::AwaitingVerification))
                                }
                                Ok(ExpertEnd::Failed) => Err(anyhow::anyhow!("代理人专业挑战失败")),
                                Err(e) => Err(e),
                            };
                            (fact, end)
                        }
                        Child::Hunt(c) => {
                            let end = c.execute_with_budget(slot, remaining(), |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.hunt = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                publish(snapshot.clone())
                            });
                            snapshot.hunt = Some(c.snapshot());
                            snapshot.inputs_released = c.snapshot().inputs_released;
                            let fact = if let Some((proof, goal)) = c.agent_completion_for_parent()
                            {
                                ensure!(
                                    proof.run_id == snapshot.id && proof.plan == expected,
                                    "代理人追猎结果不属于本次具体目标"
                                );
                                Some(AgentChallengeResult::Completed {
                                    charge_after: proof.charge_after,
                                    goal,
                                })
                            } else if let Some(charge) = c.shortage_for_parent() {
                                Some(AgentChallengeResult::Insufficient { charge })
                            } else {
                                c.weekly_for_parent().map(|remaining| {
                                    AgentChallengeResult::WeeklyRewardsAvailable { remaining }
                                })
                            };
                            let end = match end {
                                Ok(
                                    HuntEnd::Completed
                                    | HuntEnd::InsufficientCharge
                                    | HuntEnd::WeeklyRewardsAvailable,
                                ) => Ok(None),
                                Ok(HuntEnd::Paused) => Ok(Some(AgentChallengeEnd::Paused)),
                                Ok(HuntEnd::Stopped) => Ok(Some(AgentChallengeEnd::Stopped)),
                                Ok(HuntEnd::TimedOut) => Ok(Some(AgentChallengeEnd::TimedOut)),
                                Ok(HuntEnd::AwaitingVerification) => {
                                    Ok(Some(AgentChallengeEnd::AwaitingVerification))
                                }
                                Ok(HuntEnd::Failed) => Err(anyhow::anyhow!("代理人追猎挑战失败")),
                                Err(e) => Err(e),
                            };
                            (fact, end)
                        }
                        Child::Transport(_) => anyhow::bail!("代理人挑战阶段不一致"),
                    };
                    if let Some(fact) = fact {
                        self.capture(fact)?;
                    } else if let Some(end) = end? {
                        return Ok(end);
                    } else {
                        anyhow::bail!("代理人挑战没有可核实结果");
                    }
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.snapshot.inputs_released = slot.as_mut().is_none_or(|n| n.release_inputs().is_ok());
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() || !self.snapshot.inputs_released {
            self.end = Some(AgentChallengeEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "代理人挑战收尾未能确认输入释放"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(AgentChallengeEnd::Failed);
            return Err(error);
        }
        if let Some(interrupted) = self.interrupted() {
            end = interrupted;
        } else if remaining().is_zero() {
            end = AgentChallengeEnd::TimedOut;
        }
        if end == AgentChallengeEnd::Paused && !self.control.acknowledge_pause() {
            end = AgentChallengeEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
