//! 本次代理人导航直接交接详情目标选择；回执不提供选中或消费许可。
use crate::{
    agent_mission::{
        AgentMissionEnd as MissionEnd, AgentMissionRun as MissionRun,
        AgentMissionSnapshot as MissionSnapshot,
    },
    agent_transport::{
        AgentTransportEnd as TransportEnd, AgentTransportRun as TransportRun,
        AgentTransportSnapshot as TransportSnapshot,
    },
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{GameSessionConfig, InferenceDevice, NativeSession, SessionHandoff};
use od_core::{
    CaptureMethod, InputMode, WindowInfo,
    agent_mission::{AgentMissionReceipt as MissionReceipt, AgentMissionRequest as MissionRequest},
    agent_transport::{AgentTransportReceipt, AgentTransportRequest},
    charge_plan::ChargePlanItem,
    pipeline::{Interrupt, PipelineControl, PipelineRequest},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub enum ApproachTarget {
    Game {
        window: WindowInfo,
        capture: CaptureMethod,
    },
    /// 隔离回放复用已有控制器，不作为真实游戏交接证据。
    Fixture,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApproachStage {
    Transport,
    Mission,
    AwaitingVerification,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApproachEnd {
    AwaitingVerification,
    Unavailable,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproachSnapshot {
    pub id: Uuid,
    pub plan: ChargePlanItem,
    pub stage: ApproachStage,
    pub transport: TransportSnapshot,
    pub mission: Option<MissionSnapshot>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApproachReceipt {
    pub run_id: Uuid,
    pub plan_id: String,
    pub mission: MissionReceipt,
    pub transport: AgentTransportReceipt,
}
struct Resources {
    root: PathBuf,
    transport: PathBuf,
    mission: PathBuf,
    manifests: Vec<(PathBuf, Vec<u8>)>,
}
impl Resources {
    fn load(root: &Path) -> Result<Self> {
        od_core::config::reject_links(root)?;
        let transport = maa_adapter::agent_transport::validate_agent_transport_bundle(
            &root.join("generated/agent-transport"),
        )?;
        let mission = maa_adapter::agent_mission::validate_agent_mission_bundle(
            &root.join("generated/agent-mission"),
        )?;
        let manifests = [
            transport.join("agent_transport-manifest.json"),
            mission.join("agent_mission-manifest.json"),
        ]
        .into_iter()
        .map(|p| Ok((p.clone(), Sha256::digest(std::fs::read(p)?).to_vec())))
        .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            root: root.canonicalize()?,
            transport,
            mission,
            manifests,
        })
    }
    fn verify(&self) -> Result<()> {
        for (path, hash) in &self.manifests {
            od_core::config::reject_links(path)?;
            ensure!(
                Sha256::digest(std::fs::read(path)?).as_slice() == hash,
                "代理人模拟室传送选关的冻结资源已变化"
            );
        }
        maa_adapter::agent_transport::validate_agent_transport_bundle(&self.transport)?;
        maa_adapter::agent_mission::validate_agent_mission_bundle(&self.mission)?;
        Ok(())
    }
}
enum Child {
    Transport(Box<TransportRun>),
    Mission(Box<MissionRun>),
}
pub struct AgentSimulationApproachRun {
    snapshot: ApproachSnapshot,
    resources: Resources,
    target: ApproachTarget,
    timeout_ms: u32,
    control: PipelineControl,
    child: Child,
    child_started: bool,
    end: Option<ApproachEnd>,
}
impl AgentSimulationApproachRun {
    pub fn new(
        id: Uuid,
        plan: ChargePlanItem,
        resources: &Path,
        target: ApproachTarget,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(
            !id.is_nil() && plan.category_name == "实战模拟室",
            "代理人模拟室导航身份或分类无效"
        );
        let request = AgentTransportRequest {
            plan: plan.clone(),
            timeout_ms,
        };
        request.validate().map_err(anyhow::Error::msg)?;
        if let ApproachTarget::Game { window, .. } = &target {
            ensure!(
                window.width == 1920 && window.height == 1080,
                "代理人模拟室传送选关须冻结 1080p 游戏窗口"
            );
        }
        let resources = Resources::load(resources)?;
        let transport = TransportRun::new(request, control.clone())?;
        Ok(Self {
            snapshot: ApproachSnapshot {
                id,
                plan,
                stage: ApproachStage::Transport,
                transport: transport.snapshot(),
                mission: None,
                active_ms: 0,
                inputs_released: true,
            },
            resources,
            target,
            timeout_ms,
            control,
            child: Child::Transport(Box::new(transport)),
            child_started: false,
            end: None,
        })
    }
    pub fn snapshot(&self) -> &ApproachSnapshot {
        &self.snapshot
    }
    pub fn control(&self) -> PipelineRequest {
        self.control.requester()
    }
    /// 只交回未验证回执，不能作为后续消费或运行成功证明。
    pub fn receipt(&self) -> Option<ApproachReceipt> {
        if self.end != Some(ApproachEnd::AwaitingVerification)
            || !self.snapshot.inputs_released
            || self.control.requested().is_some()
        {
            return None;
        }
        Some(ApproachReceipt {
            run_id: self.snapshot.id,
            plan_id: self.snapshot.plan.plan_id.clone(),
            mission: self.snapshot.mission.as_ref()?.flow.receipt()?.clone(),
            transport: self.snapshot.transport.flow.receipt()?.clone(),
        })
    }
    fn interrupted(&self) -> Option<ApproachEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => ApproachEnd::Paused,
            Interrupt::Stop => ApproachEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(ApproachEnd::Paused) && self.snapshot.inputs_released,
            "代理人模拟室传送选关尚未安全暂停"
        );
        if !self.child_started || self.snapshot.stage == ApproachStage::Unavailable {
            ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        } else {
            match &mut self.child {
                Child::Transport(r) => {
                    r.resume_after_parent_pause()?;
                    self.snapshot.transport = r.snapshot();
                }
                Child::Mission(r) => {
                    r.resume_after_parent_pause()?;
                    self.snapshot.mission = Some(r.snapshot());
                    self.snapshot.stage = ApproachStage::Mission;
                }
            }
        }
        self.end = None;
        Ok(())
    }
    fn session<'a>(
        &self,
        slot: &'a mut Option<NativeSession>,
    ) -> Result<Option<&'a mut NativeSession>> {
        match &self.target {
            ApproachTarget::Game { window, capture } => {
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
            ApproachTarget::Fixture => {
                let session = slot
                    .as_mut()
                    .context("代理人模拟室传送选关的隔离控制器缺失")?;
                session.quiesce()?;
                session.check_combat_input_mode(InputMode::Foreground)?;
            }
        }
        let session = slot
            .as_mut()
            .context("代理人模拟室传送选关的原生会话缺失")?;
        match self.snapshot.stage {
            ApproachStage::Transport => {
                session.load_agent_transport_bundle(&self.resources.transport)?
            }
            ApproachStage::Mission => session.load_agent_mission_bundle(&self.resources.mission)?,
            ApproachStage::Unavailable | ApproachStage::AwaitingVerification => {
                anyhow::bail!("未验证回执不能继续执行")
            }
        }
        Ok(Some(session))
    }
    pub fn execute_with_budget(
        &mut self,
        slot: &mut Option<NativeSession>,
        parent_remaining: Duration,
        mut publish: impl FnMut(ApproachSnapshot) -> Result<()>,
    ) -> Result<ApproachEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(ApproachEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "代理人模拟室传送选关不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(u64::from(self.timeout_ms).saturating_sub(initial))
            .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let mut publication_failed = false;
        let mut save = |s| {
            let result = publish(s);
            publication_failed |= result.is_err();
            result
        };
        let result = (|| -> Result<ApproachEnd> {
            if let Some(session) = slot.as_mut() {
                session.quiesce()?;
            }
            self.resources.verify()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(ApproachEnd::TimedOut);
                }
                if self.snapshot.stage == ApproachStage::Unavailable {
                    return Ok(ApproachEnd::Unavailable);
                }
                let Some(session) = self.session(slot)? else {
                    return Ok(self.interrupted().unwrap_or(ApproachEnd::Stopped));
                };
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(ApproachEnd::TimedOut);
                }
                self.child_started = true;
                let snapshot = &mut self.snapshot;
                match &mut self.child {
                    Child::Transport(child) => {
                        let result = child.execute_with_budget(
                            session,
                            matches!(self.target, ApproachTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.transport = s;
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.transport = child.snapshot();
                        snapshot.inputs_released = child.snapshot().inputs_released;
                        let end = result?;
                        if let Some(request) = self.control.requested() {
                            return Ok(match request {
                                Interrupt::Pause => ApproachEnd::Paused,
                                Interrupt::Stop => ApproachEnd::Stopped,
                            });
                        }
                        if remaining().is_zero() {
                            return Ok(ApproachEnd::TimedOut);
                        }
                        match end {
                            TransportEnd::AwaitingVerification => {}
                            TransportEnd::Unavailable => {
                                self.snapshot.stage = ApproachStage::Unavailable;
                                return Ok(ApproachEnd::Unavailable);
                            }
                            TransportEnd::Paused => return Ok(ApproachEnd::Paused),
                            TransportEnd::Stopped => return Ok(ApproachEnd::Stopped),
                            TransportEnd::TimedOut => return Ok(ApproachEnd::TimedOut),
                            TransportEnd::Failed => anyhow::bail!("模拟室传送失败"),
                        }
                        let route = child
                            .receipt_for_parent()
                            .context("代理人导航尚未完成保存和释放")?;
                        ensure!(
                            child.uses_control(&self.control)
                                && route.plan == self.snapshot.plan
                                && self.snapshot.inputs_released
                                && self.snapshot.transport.flow.completed(),
                            "代理人模拟室选关须属于本次真实导航和共享控制"
                        );
                        route.validate().map_err(anyhow::Error::msg)?;
                        let request = MissionRequest {
                            mission_type: route.target.mission_type.clone(),
                            timeout_ms: self.timeout_ms,
                        };
                        request.validate().map_err(anyhow::Error::msg)?;
                        let mission = MissionRun::new(request, self.control.clone())?;
                        self.snapshot.mission = Some(mission.snapshot());
                        self.child = Child::Mission(Box::new(mission));
                        self.snapshot.stage = ApproachStage::Mission;
                        self.child_started = false;
                        self.snapshot.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        save(self.snapshot.clone())?;
                        self.resources.verify()?;
                    }
                    Child::Mission(child) => {
                        let result = child.execute_with_budget(
                            session,
                            matches!(self.target, ApproachTarget::Game { .. }),
                            remaining(),
                            |s| {
                                snapshot.inputs_released = s.inputs_released;
                                snapshot.mission = Some(s);
                                snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                save(snapshot.clone())
                            },
                        );
                        snapshot.mission = Some(child.snapshot());
                        snapshot.inputs_released = child.snapshot().inputs_released;
                        let end = match result? {
                            MissionEnd::AwaitingVerification => ApproachEnd::AwaitingVerification,
                            MissionEnd::Unavailable => ApproachEnd::Unavailable,
                            MissionEnd::Paused => ApproachEnd::Paused,
                            MissionEnd::Stopped => ApproachEnd::Stopped,
                            MissionEnd::TimedOut => ApproachEnd::TimedOut,
                            MissionEnd::Failed => anyhow::bail!("模拟室关卡选择失败"),
                        };
                        if end == ApproachEnd::AwaitingVerification {
                            ensure!(
                                snapshot.inputs_released
                                    && child.snapshot().flow.receipt().is_some_and(|r| !r
                                        .selection_verified
                                        && r.mission_type
                                            == child.snapshot().flow.request().mission_type
                                        && r.target
                                            .validate(child.snapshot().flow.request())
                                            .is_ok()),
                                "代理人关卡回读缺少本次未验证目标"
                            );
                            snapshot.stage = ApproachStage::AwaitingVerification;
                        }
                        if end == ApproachEnd::Unavailable {
                            snapshot.stage = ApproachStage::Unavailable;
                        }
                        return Ok(end);
                    }
                }
            }
        })();
        self.end = Some(ApproachEnd::Failed);
        self.snapshot.inputs_released = if let Some(session) = slot.as_mut() {
            session.quiesce().is_ok()
        } else {
            self.snapshot.inputs_released
        };
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        ensure!(
            self.snapshot.inputs_released,
            "代理人模拟室传送选关收尾未确认输入释放"
        );
        let mut end = match result {
            Ok(end) => end,
            Err(_) if !publication_failed && remaining().is_zero() => ApproachEnd::TimedOut,
            Err(error) => return Err(error),
        };
        publish(self.snapshot.clone())?;
        if let Some(request) = self.interrupted() {
            end = request;
        } else if remaining().is_zero() {
            end = ApproachEnd::TimedOut;
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if end == ApproachEnd::Paused && !self.control.acknowledge_pause() {
            end = ApproachEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
