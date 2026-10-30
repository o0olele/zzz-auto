//! 从入口到本次真实局内页面；首次输入前冻结资源和所有候选脚本。
use crate::{
    lost_void_classic_teams::{
        LostVoidClassicTeamsEnd, LostVoidClassicTeamsRun, LostVoidClassicTeamsSnapshot,
    },
    lost_void_combat::FrozenLostVoidPreparation,
    lost_void_deploy::{LostVoidDeployEnd, LostVoidDeployRun, LostVoidDeploySnapshot},
    lost_void_entry::{LostVoidEntryEnd, LostVoidEntryRun, LostVoidEntrySnapshot},
    lost_void_matrix::{LostVoidMatrixEnd, LostVoidMatrixRun, LostVoidMatrixSnapshot},
    lost_void_strategy::{LostVoidStrategyEnd, LostVoidStrategyRun, LostVoidStrategySnapshot},
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{NativeSession, combat_vision::CombatRecognizer};
use od_core::{
    lost_void::{ArtifactSelection, LostVoidConfig, LostVoidMission as M},
    lost_void_classic_teams::{LostVoidClassicTeamsFlow, LostVoidClassicTeamsRequest},
    lost_void_deploy::{DeploymentParty, LostVoidDeployReceipt, LostVoidDeployRequest},
    lost_void_entry::{EntryOutcome, LostVoidEntryReceipt, LostVoidEntryRequest},
    lost_void_matrix::LostVoidMatrixRequest,
    lost_void_roster::RosterRequest,
    lost_void_strategy::{LostVoidStrategyReceipt, LostVoidStrategyRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationPhase {
    Entry,
    Strategy,
    ClassicTeams,
    Matrix,
    Deploy,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidPreparationEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
pub(crate) struct LostVoidUpObservation {
    pub agents: Vec<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub observed_at: chrono::DateTime<chrono::Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum LostVoidPreparationReceipt {
    BountyAlreadyComplete {
        entry: LostVoidEntryReceipt,
    },
    Entered {
        entry: LostVoidEntryReceipt,
        strategy: Option<LostVoidStrategyReceipt>,
        deployment: Box<LostVoidDeployReceipt>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidPreparationSnapshot {
    pub run_id: Uuid,
    pub config: LostVoidConfig,
    pub timeout_ms: u32,
    pub phase: PreparationPhase,
    pub active_ms: u64,
    pub inputs_released: bool,
    pub entry: LostVoidEntrySnapshot,
    pub strategy: Option<LostVoidStrategySnapshot>,
    pub classic: Option<LostVoidClassicTeamsSnapshot>,
    pub matrix: Option<LostVoidMatrixSnapshot>,
    pub deploy: Option<LostVoidDeploySnapshot>,
    pub receipt: Option<LostVoidPreparationReceipt>,
}
pub struct LostVoidPreparationRun {
    config: LostVoidConfig,
    timeout_ms: u32,
    phase: PreparationPhase,
    frozen: FrozenLostVoidPreparation,
    control: PipelineControl,
    entry: LostVoidEntryRun,
    strategy: Option<LostVoidStrategyRun>,
    classic: Option<LostVoidClassicTeamsRun>,
    matrix: Option<LostVoidMatrixRun>,
    deploy: Option<LostVoidDeployRun>,
    recognizer: Option<CombatRecognizer>,
    receipt: Option<LostVoidPreparationReceipt>,
    active_ms: u64,
    inputs_released: bool,
    child_ran: bool,
    end: Option<LostVoidPreparationEnd>,
}
impl LostVoidPreparationRun {
    pub fn new(
        config: LostVoidConfig,
        frozen: FrozenLostVoidPreparation,
        control: PipelineControl,
        timeout_ms: u32,
    ) -> Result<Self> {
        config.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            (1000..=1_800_000).contains(&timeout_ms),
            "迷失之地完整准备预算无效"
        );
        ensure!(
            config.mission_name == frozen.seed().mission
                && config.challenge == frozen.seed().challenge,
            "迷失之地完整准备与首次输入前冻结配置不一致"
        );
        frozen.seed().validate_roster_settings()?;
        frozen.verify()?;
        let entry = LostVoidEntryRun::new(
            LostVoidEntryRequest {
                run_id: frozen.seed().run_id,
                config: config.clone(),
                timeout_ms: 300_000,
            },
            control.clone(),
        )?;
        Ok(Self {
            config,
            timeout_ms,
            frozen,
            control,
            entry,
            phase: PreparationPhase::Entry,
            strategy: None,
            classic: None,
            matrix: None,
            deploy: None,
            recognizer: None,
            receipt: None,
            active_ms: 0,
            inputs_released: true,
            child_ran: false,
            end: None,
        })
    }
    pub fn snapshot(&self) -> LostVoidPreparationSnapshot {
        LostVoidPreparationSnapshot {
            run_id: self.frozen.seed().run_id,
            config: self.config.clone(),
            timeout_ms: self.timeout_ms,
            phase: self.phase,
            active_ms: self.active_ms,
            inputs_released: self.inputs_released,
            entry: self.entry.snapshot(),
            strategy: self.strategy.as_ref().map(|r| r.snapshot()),
            classic: self.classic.as_ref().map(|r| r.snapshot()),
            matrix: self.matrix.as_ref().map(|r| r.snapshot()),
            deploy: self.deploy.as_ref().map(|r| r.snapshot()),
            receipt: self.receipt.clone(),
        }
    }
    pub fn uses_control(&self, c: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &c.cancellation())
    }
    pub(crate) fn actual_up_observation(&self) -> Option<LostVoidUpObservation> {
        self.receipt()?;
        let strategy = self.strategy.as_ref()?;
        let receipt = strategy.receipt()?;
        let snapshot = strategy.snapshot();
        let started = snapshot.up_observation_started_at?;
        let finished = snapshot.up_observed_at?;
        (receipt.up_agents.len() == 3 && started <= finished).then_some(LostVoidUpObservation {
            agents: receipt.up_agents.clone(),
            started_at: started,
            observed_at: finished,
        })
    }
    pub fn receipt(&self) -> Option<&LostVoidPreparationReceipt> {
        (self.end == Some(LostVoidPreparationEnd::Ready)
            && self.inputs_released
            && self.control.requested().is_none())
        .then_some(self.receipt.as_ref())
        .flatten()
    }
    pub fn frozen(&self) -> &FrozenLostVoidPreparation {
        &self.frozen
    }
    pub(crate) fn pristine(&self) -> bool {
        self.end.is_none()
            && self.phase == PreparationPhase::Entry
            && self.active_ms == 0
            && !self.child_ran
            && self.receipt.is_none()
    }
    pub fn recognizer(&self) -> Option<&CombatRecognizer> {
        self.recognizer.as_ref()
    }
    /// 来源为本次出战回执；保持当前的计数在出战前确认，不从 HUD 补值。
    pub fn roster_request(
        &self,
        level_id: Uuid,
        options: ArtifactSelection,
        timeout_ms: u32,
    ) -> Result<RosterRequest> {
        let LostVoidPreparationReceipt::Entered { deployment, .. } =
            self.receipt().context("完整准备尚未安全交回")?
        else {
            anyhow::bail!("已满悬赏入口不能作为局内队伍证明");
        };
        deployment.validate().map_err(anyhow::Error::msg)?;
        self.frozen.roster_request(
            deployment.party.plan(),
            level_id,
            deployment.member_count,
            options,
            timeout_ms,
        )
    }
    fn teams(&mut self) -> Result<()> {
        let seed = self.frozen.seed();
        if seed.mission == M::MatrixAction {
            self.matrix = Some(LostVoidMatrixRun::new(
                LostVoidMatrixRequest {
                    run_id: seed.run_id,
                    challenge: seed.challenge.clone(),
                    teams: seed.teams.clone(),
                    timeout_ms: 600_000,
                },
                self.control.clone(),
            )?);
            self.phase = PreparationPhase::Matrix;
        } else {
            let up = self
                .strategy
                .as_ref()
                .and_then(|s| s.receipt_for_parent())
                .filter(|s| s.mission == M::TaskForce)
                .map(|s| s.up_agents.clone());
            let flow = LostVoidClassicTeamsFlow::new(LostVoidClassicTeamsRequest {
                run_id: seed.run_id,
                mission: seed.mission,
                challenge: seed.challenge.clone(),
                teams: seed.teams.clone(),
                up_agents: up,
                up_completed_this_week: seed.up_completed_this_week,
                timeout_ms: 900_000,
            })
            .map_err(anyhow::Error::msg)?;
            self.classic = Some(LostVoidClassicTeamsRun::new(flow, self.control.clone())?);
            self.phase = PreparationPhase::ClassicTeams;
        }
        self.child_ran = false;
        Ok(())
    }
    fn advance(&mut self) -> Result<()> {
        match self.phase {
            PreparationPhase::Entry => {
                let entry = self
                    .entry
                    .receipt_for_parent()
                    .context("入口未安全交回")?
                    .clone();
                if entry.outcome == EntryOutcome::BountyComplete {
                    self.receipt =
                        Some(LostVoidPreparationReceipt::BountyAlreadyComplete { entry });
                    self.phase = PreparationPhase::Done;
                } else if self.config.mission_name == M::MatrixAction {
                    self.teams()?;
                } else {
                    self.strategy = Some(LostVoidStrategyRun::new(
                        LostVoidStrategyRequest {
                            run_id: entry.run_id,
                            mission: entry.mission,
                            challenge: self.config.challenge.clone(),
                            timeout_ms: 300_000,
                        },
                        self.control.clone(),
                    )?);
                    self.phase = PreparationPhase::Strategy;
                }
            }
            PreparationPhase::Strategy => {
                self.teams()?;
            }
            PreparationPhase::ClassicTeams | PreparationPhase::Matrix => {
                let (party, matrix) = if self.phase == PreparationPhase::ClassicTeams {
                    (
                        DeploymentParty::Classic {
                            receipt: self
                                .classic
                                .as_ref()
                                .unwrap()
                                .receipt_for_parent()
                                .context("普通选队未安全交回")?
                                .clone(),
                        },
                        None,
                    )
                } else {
                    let run = self.matrix.as_ref().unwrap();
                    (
                        DeploymentParty::Matrix {
                            receipt: run
                                .receipt_for_parent()
                                .context("矩阵编队未安全交回")?
                                .clone(),
                        },
                        Some(run.snapshot().flow),
                    )
                };
                let up = (!party.plan().up_agents.is_empty())
                    .then_some(party.plan().up_agents.as_slice());
                ensure!(
                    party.run_id() == self.frozen.seed().run_id
                        && party.mission() == self.config.mission_name
                        && self.frozen.resolve_plan(up)? == *party.plan(),
                    "出战编队与冻结运行计划不一致"
                );
                self.deploy = Some(LostVoidDeployRun::new(
                    LostVoidDeployRequest {
                        party,
                        timeout_ms: 300_000,
                    },
                    matrix,
                    self.control.clone(),
                )?);
                self.phase = PreparationPhase::Deploy;
            }
            PreparationPhase::Deploy => {
                let deployment = self
                    .deploy
                    .as_ref()
                    .unwrap()
                    .receipt_for_parent()
                    .context("出战尚未安全交回")?
                    .clone();
                self.receipt = Some(LostVoidPreparationReceipt::Entered {
                    entry: self
                        .entry
                        .receipt_for_parent()
                        .context("本次入口回执丢失")?
                        .clone(),
                    strategy: self
                        .strategy
                        .as_ref()
                        .and_then(|r| r.receipt_for_parent())
                        .cloned(),
                    deployment: Box::new(deployment),
                });
                self.phase = PreparationPhase::Done;
            }
            PreparationPhase::Done => {}
        }
        self.child_ran = false;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidPreparationEnd::Paused) && self.inputs_released,
            "完整准备尚未安全暂停"
        );
        self.resume_after_parent_pause()
    }
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidPreparationEnd::Paused | LostVoidPreparationEnd::Ready)
            ) && self.inputs_released,
            "完整准备尚未安全交回或暂停"
        );
        if self.phase == PreparationPhase::Done {
            self.phase = if self.deploy.is_some() {
                PreparationPhase::Deploy
            } else {
                PreparationPhase::Entry
            };
            self.child_ran = true;
        }
        if self.phase == PreparationPhase::Deploy
            && !self.deploy.as_ref().unwrap().snapshot().flow.launched()
        {
            // 暂停期间可能修改了出战页。重新核对 / 选择编队，而不是复用关闭列表前的证明。
            self.deploy = None;
            self.teams()?;
            ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        } else if self.child_ran {
            match self.phase {
                PreparationPhase::Entry => self.entry.resume_after_parent_pause()?,
                PreparationPhase::Strategy => self
                    .strategy
                    .as_mut()
                    .unwrap()
                    .resume_after_parent_pause()?,
                PreparationPhase::ClassicTeams => {
                    self.classic.as_mut().unwrap().resume_after_parent_pause()?
                }
                PreparationPhase::Matrix => {
                    self.matrix.as_mut().unwrap().resume_after_parent_pause()?
                }
                PreparationPhase::Deploy => {
                    self.deploy.as_mut().unwrap().resume_after_parent_pause()?
                }
                PreparationPhase::Done => unreachable!(),
            }
        } else {
            ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        }
        self.receipt = None;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidPreparationEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidPreparationEnd::Paused,
            Interrupt::Stop => LostVoidPreparationEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidPreparationSnapshot) -> Result<()>,
    ) -> Result<LostVoidPreparationEnd> {
        use LostVoidPreparationEnd as E;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "完整准备不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.active_ms;
        let budget = Duration::from_millis(u64::from(self.timeout_ms).saturating_sub(initial))
            .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<E> {
            session.check_ether_synthesis_target(game)?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                self.frozen.verify()?;
                if self.recognizer.is_none() {
                    session.release_inputs()?;
                    if let Some(root) = self.frozen.daily_root() {
                        session.load_basic_daily_bundle(root)?;
                    }
                    session.load_lost_void_scene_bundle(self.frozen.scene_root())?;
                    self.recognizer = Some(session.load_combat_bundle(self.frozen.combat_root())?);
                }
                session.require_lost_void_preparation_roots(
                    self.frozen.scene_root(),
                    self.frozen.daily_root(),
                    self.frozen.combat_root(),
                )?;
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                if self.phase == PreparationPhase::Done {
                    return Ok(E::Ready);
                }
                self.child_ran = true;
                self.inputs_released = false;
                let mut saved = self.snapshot();
                // 每个子保存都嵌入父检查点。保存和资源预检耗时计入同一剩余活动预算。
                macro_rules! execute {
                    ($run:expr, $field:ident, $end:ident, $ready:ident) => {{
                        let r = $run.execute_with_budget(session, game, remaining(), |s| {
                            saved.$field = Some(s);
                            saved.active_ms =
                                initial.saturating_add(started.elapsed().as_millis() as u64);
                            saved.inputs_released = saved.$field.as_ref().unwrap().inputs_released;
                            publish(saved.clone())
                        })?;
                        match r {
                            $end::$ready => E::Ready,
                            $end::Paused => E::Paused,
                            $end::Stopped => E::Stopped,
                            $end::TimedOut => E::TimedOut,
                            $end::Failed => E::Failed,
                        }
                    }};
                }
                let end = match self.phase {
                    PreparationPhase::Entry => {
                        let r =
                            self.entry
                                .execute_with_budget(session, game, remaining(), |s| {
                                    saved.entry = s;
                                    saved.inputs_released = saved.entry.inputs_released;
                                    saved.active_ms = initial
                                        .saturating_add(started.elapsed().as_millis() as u64);
                                    publish(saved.clone())
                                })?;
                        match r {
                            LostVoidEntryEnd::Ready => E::Ready,
                            LostVoidEntryEnd::Paused => E::Paused,
                            LostVoidEntryEnd::Stopped => E::Stopped,
                            LostVoidEntryEnd::TimedOut => E::TimedOut,
                            LostVoidEntryEnd::Failed => E::Failed,
                        }
                    }
                    PreparationPhase::Strategy => execute!(
                        self.strategy.as_mut().unwrap(),
                        strategy,
                        LostVoidStrategyEnd,
                        Ready
                    ),
                    PreparationPhase::ClassicTeams => execute!(
                        self.classic.as_mut().unwrap(),
                        classic,
                        LostVoidClassicTeamsEnd,
                        Prepared
                    ),
                    PreparationPhase::Matrix => execute!(
                        self.matrix.as_mut().unwrap(),
                        matrix,
                        LostVoidMatrixEnd,
                        Ready
                    ),
                    PreparationPhase::Deploy => execute!(
                        self.deploy.as_mut().unwrap(),
                        deploy,
                        LostVoidDeployEnd,
                        Ready
                    ),
                    PreparationPhase::Done => unreachable!(),
                };
                self.inputs_released = session.release_inputs().is_ok();
                ensure!(self.inputs_released, "完整准备的子流程未释放输入");
                if end != E::Ready {
                    return Ok(end);
                }
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                self.advance()?;
                self.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot())?;
            }
        })();
        self.inputs_released = session.release_inputs().is_ok();
        self.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() || !self.inputs_released {
            self.end = Some(E::Failed);
        }
        ensure!(self.inputs_released, "完整准备收尾无法确认释放输入");
        let mut end = result?;
        if let Err(error) = publish(self.snapshot()) {
            self.end = Some(E::Failed);
            return Err(error);
        }
        self.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
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
