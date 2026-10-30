//! 层内导航、战斗、事件与 NPC 移动共用记忆及预算；实际跨层才交回新层身份。
use crate::{
    lost_void_battle::{LostVoidBattleEnd, LostVoidBattleRun, LostVoidBattleSnapshot},
    lost_void_combat::{FrozenLostVoidPreparation, LostVoidCombatSeed},
    lost_void_event::{LostVoidEventEnd, LostVoidEventRun, LostVoidEventSnapshot},
    lost_void_navigation::{
        LostVoidNavigationEnd, LostVoidNavigationRun, LostVoidNavigationSnapshot,
    },
    lost_void_npc::{LostVoidNpcEnd, LostVoidNpcRun, LostVoidNpcSnapshot},
    lost_void_preparation::{LostVoidPreparationReceipt, LostVoidPreparationRun},
    lost_void_roster::{LostVoidRosterEnd, LostVoidRosterRun, LostVoidRosterSnapshot},
    lost_void_transition::{
        LostVoidTransitionEnd, LostVoidTransitionRun, LostVoidTransitionSnapshot,
    },
};
use anyhow::{Context, Result, ensure};
use maa_adapter::{
    NativeSession, combat_vision::CombatRecognizer, lost_void_roster::LostVoidHudReader,
};
pub use od_core::lost_void_level::{LevelOutcome, LostVoidLevelReceipt};
use od_core::{
    lost_void::{ArtifactSelection, choose_artifacts},
    lost_void_battle::{BattleOutcome, BattleReceipt},
    lost_void_deploy::DeploymentParty,
    lost_void_event::{LostVoidEventReceipt, LostVoidEventRequest},
    lost_void_navigation::{NavOutcome, NavRequest},
    lost_void_npc::{NpcMemory, NpcTarget},
    lost_void_recovery::{LayerFailure, LayerRetryBudget, RetryReason},
    lost_void_roster::RosterMember,
    lost_void_scene::LostVoidScreen as S,
    lost_void_team_plan::LostVoidTeamPlan,
    lost_void_transition::{TransitionOutcome, TransitionReceipt},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use uuid::Uuid;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelPhase {
    Roster,
    Navigation,
    Battle,
    Event,
    NpcMove,
    Transition,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidLevelEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidAttemptLimits {
    pub navigation_ms: u32,
    pub battle_ms: u32,
}
impl Default for LostVoidAttemptLimits {
    fn default() -> Self {
        Self {
            navigation_ms: 300_000,
            battle_ms: 600_000,
        }
    }
}
impl LostVoidAttemptLimits {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1000..=300_000).contains(&self.navigation_ms)
                && (1000..=600_000).contains(&self.battle_ms),
            "层尝试步骤预算无效"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelEventOrigin {
    Initial,
    Navigation,
    Battle,
    RosterPage,
    NpcPage,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "receipt", rename_all = "snake_case")]
pub enum LevelStart {
    Preparation(Box<od_core::lost_void_deploy::LostVoidDeployReceipt>),
    Transition(Box<TransitionReceipt>),
    Settlement(Box<od_core::lost_void_settlement::SettlementReceipt>),
    Recovery(Box<od_core::lost_void_recovery_menu::RecoveryReceipt>),
    FindExit { failure: LayerFailure },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidLevelSnapshot {
    pub run_id: Uuid,
    pub level_id: Uuid,
    pub region: String,
    pub start: LevelStart,
    pub plan: LostVoidTeamPlan,
    pub members: u8,
    pub actual_members: Option<Vec<RosterMember>>,
    pub phase: LevelPhase,
    pub event_origin: LevelEventOrigin,
    pub resume_roster: bool,
    pub timeout_ms: u32,
    pub active_ms: u64,
    pub inputs_released: bool,
    pub memory: NpcMemory,
    pub options: ArtifactSelection,
    pub retry_budget: Option<LayerRetryBudget>,
    pub attempt_limits: LostVoidAttemptLimits,
    pub child_timeout: bool,
    pub roster: Option<LostVoidRosterSnapshot>,
    pub navigation: Option<LostVoidNavigationSnapshot>,
    pub battle: Option<LostVoidBattleSnapshot>,
    pub event: Option<LostVoidEventSnapshot>,
    pub npc: Option<LostVoidNpcSnapshot>,
    pub transition: Option<LostVoidTransitionSnapshot>,
    pub receipt: Option<LostVoidLevelReceipt>,
}
enum ChildEnd {
    Ready,
    Page(S),
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
pub struct LostVoidLevelRun {
    seed: LostVoidCombatSeed,
    scene: PathBuf,
    combat: PathBuf,
    detector: PathBuf,
    level_id: Uuid,
    region: String,
    start: LevelStart,
    plan: LostVoidTeamPlan,
    members: u8,
    actual_members: Option<Vec<RosterMember>>,
    memory: NpcMemory,
    options: ArtifactSelection,
    initial_options: ArtifactSelection,
    retry_budget: Option<LayerRetryBudget>,
    recovery_reserved: bool,
    attempt_limits: LostVoidAttemptLimits,
    child_timeout: bool,
    phase: LevelPhase,
    event_origin: LevelEventOrigin,
    resume_roster: bool,
    timeout_ms: u32,
    active_ms: u64,
    inputs_released: bool,
    control: PipelineControl,
    recognizer: Option<CombatRecognizer>,
    reader: Option<LostVoidHudReader>,
    roster: Option<LostVoidRosterRun>,
    navigation: Option<LostVoidNavigationRun>,
    battle: Option<LostVoidBattleRun>,
    event: Option<LostVoidEventRun>,
    npc: Option<LostVoidNpcRun>,
    transition: Option<LostVoidTransitionRun>,
    last_battle: Option<BattleReceipt>,
    receipt: Option<LostVoidLevelReceipt>,
    completion_phase: Option<LevelPhase>,
    child_started: bool,
    end: Option<LostVoidLevelEnd>,
    steps: u32,
}
impl LostVoidLevelRun {
    pub fn after_preparation(
        preparation: &LostVoidPreparationRun,
        mut options: ArtifactSelection,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(
            preparation.uses_control(&control),
            "层流程与完整准备须共用父控制"
        );
        let LostVoidPreparationReceipt::Entered { deployment, .. } =
            preparation.receipt().context("准备尚未安全交回局内页面")?
        else {
            anyhow::bail!("悬赏已满入口不是层流程起点");
        };
        if let DeploymentParty::Matrix { receipt } = &deployment.party {
            for rule in &receipt.support_rules {
                if !options.battle_priority.contains(rule) {
                    options.battle_priority.push(rule.clone());
                }
            }
        }
        let count = deployment
            .party
            .expected_count()
            .context("保持当前队伍仍缺出战页完整人数来源")?;
        let frozen = preparation.frozen();
        Self::new(
            frozen,
            Uuid::new_v4(),
            "入口".into(),
            LevelStart::Preparation(deployment.clone()),
            deployment.party.plan().clone(),
            count,
            None,
            options,
            deployment.arrival,
            timeout_ms,
            control,
        )
    }
    pub fn after_transition(
        frozen: &FrozenLostVoidPreparation,
        transition: &LostVoidTransitionRun,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(
            transition.uses_control(&control),
            "新层与实际跨层须共用父控制"
        );
        transition.verify_frozen(frozen)?;
        let receipt = transition.receipt().context("跨层尚未安全交回")?;
        let TransitionOutcome::Arrived(arrival) = &receipt.outcome else {
            anyhow::bail!("事件或结果页面不是下一层到达证明");
        };
        let snapshot = transition.snapshot();
        Self::new(
            frozen,
            arrival.level_id,
            arrival.region.clone(),
            LevelStart::Transition(Box::new(receipt.clone())),
            snapshot.flow.request().roster.plan.clone(),
            arrival.members.len() as u8,
            Some(arrival.members.clone()),
            transition.artifact_options()?,
            S::World,
            timeout_ms,
            control,
        )
    }
    pub fn after_settlement(
        frozen: &FrozenLostVoidPreparation,
        settlement: &crate::lost_void_settlement::LostVoidSettlementRun,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(
            settlement.uses_control(&control),
            "挚交会谈与实际结算须共用控制"
        );
        settlement.verify_frozen(frozen)?;
        let receipt = settlement.receipt().context("中途结算尚未安全交回")?;
        let od_core::lost_void_settlement::SettlementOutcome::Continue { arrival } =
            &receipt.outcome
        else {
            anyhow::bail!("最终结束不能创建下一层");
        };
        Self::new(
            frozen,
            arrival.level_id,
            arrival.region.clone(),
            LevelStart::Settlement(Box::new(receipt.clone())),
            settlement.snapshot().flow.request().plan.clone(),
            arrival.members.len() as u8,
            Some(arrival.members.clone()),
            receipt.options.clone(),
            S::World,
            timeout_ms,
            control,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn new(
        frozen: &FrozenLostVoidPreparation,
        level_id: Uuid,
        region: String,
        start: LevelStart,
        plan: LostVoidTeamPlan,
        members: u8,
        actual_members: Option<Vec<RosterMember>>,
        options: ArtifactSelection,
        screen: S,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(
            (1000..=1_800_000).contains(&timeout_ms) && (1..=3).contains(&members),
            "层流程预算或实际人数无效"
        );
        frozen.seed().validate_roster_settings()?;
        frozen.verify()?;
        choose_artifacts(&frozen.seed().challenge, &[], 0, &options).map_err(anyhow::Error::msg)?;
        let memory = NpcMemory::new(frozen.seed().run_id, level_id, region.clone())
            .map_err(anyhow::Error::msg)?;
        let mut run = Self {
            seed: frozen.seed().clone(),
            scene: frozen.scene_root().into(),
            combat: frozen.combat_root().into(),
            detector: frozen
                .npc_detector_root()
                .context("完整层流程缺少首次输入前冻结的导航模型")?
                .into(),
            level_id,
            region,
            start,
            plan,
            members,
            actual_members,
            memory,
            initial_options: options.clone(),
            retry_budget: None,
            recovery_reserved: false,
            attempt_limits: LostVoidAttemptLimits::default(),
            child_timeout: false,
            options,
            phase: LevelPhase::Roster,
            event_origin: LevelEventOrigin::Initial,
            resume_roster: false,
            timeout_ms,
            active_ms: 0,
            inputs_released: true,
            control,
            recognizer: None,
            reader: None,
            roster: None,
            navigation: None,
            battle: None,
            event: None,
            npc: None,
            transition: None,
            last_battle: None,
            receipt: None,
            completion_phase: None,
            child_started: false,
            end: None,
            steps: 0,
        };
        if screen == S::World {
            run.start_roster(frozen)?;
        } else {
            ensure!(
                od_core::lost_void_scene::level_content(screen)
                    && !matches!(screen, S::BattleResult | S::BattleFail),
                "准备交回了非局内起始页"
            );
            run.start_event(
                Uuid::new_v4(),
                NpcTarget::Anonymous,
                LevelEventOrigin::Initial,
                false,
            )?;
        }
        Ok(run)
    }
    pub fn snapshot(&self) -> LostVoidLevelSnapshot {
        LostVoidLevelSnapshot {
            run_id: self.seed.run_id,
            level_id: self.level_id,
            region: self.region.clone(),
            start: self.start.clone(),
            plan: self.plan.clone(),
            members: self.members,
            actual_members: self.actual_members.clone(),
            phase: self.phase,
            event_origin: self.event_origin,
            resume_roster: self.resume_roster,
            timeout_ms: self.timeout_ms,
            active_ms: self.active_ms,
            inputs_released: self.inputs_released,
            memory: self.memory.clone(),
            options: self.options.clone(),
            retry_budget: self.retry_budget.clone(),
            attempt_limits: self.attempt_limits,
            child_timeout: self.child_timeout,
            roster: self.roster.as_ref().map(|r| r.snapshot()),
            navigation: self.navigation.as_ref().map(|r| r.snapshot()),
            battle: self.battle.as_ref().map(|r| r.snapshot()),
            event: self.event.as_ref().map(|r| r.snapshot()),
            npc: self.npc.as_ref().map(|r| r.snapshot()),
            transition: self.transition.as_ref().map(|r| r.snapshot()),
            receipt: self.receipt.clone(),
        }
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn with_attempt_limits(mut self, limits: LostVoidAttemptLimits) -> Result<Self> {
        ensure!(
            self.active_ms == 0 && !self.child_started && self.end.is_none(),
            "层流程开始后不能更换步骤预算"
        );
        limits.validate()?;
        self.attempt_limits = limits;
        Ok(self)
    }
    pub fn receipt(&self) -> Option<&LostVoidLevelReceipt> {
        (self.end == Some(LostVoidLevelEnd::Ready)
            && !self.recovery_reserved
            && self.inputs_released
            && self.control.requested().is_none())
        .then_some(self.receipt.as_ref())
        .flatten()
    }
    pub fn next_level(
        &self,
        frozen: &FrozenLostVoidPreparation,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(
            matches!(
                self.receipt().map(|r| &r.outcome),
                Some(LevelOutcome::NextLevel { .. })
            ),
            "本层尚无实际下一层到达"
        );
        Self::after_transition(
            frozen,
            self.transition.as_ref().context("实际跨层执行器缺失")?,
            timeout_ms,
            control,
        )
    }
    pub(crate) fn verify_frozen(&self, frozen: &FrozenLostVoidPreparation) -> Result<()> {
        ensure!(
            frozen.seed() == &self.seed
                && frozen.scene_root() == self.scene
                && frozen.combat_root() == self.combat
                && frozen.npc_detector_root() == Some(self.detector.as_path()),
            "层流程冻结身份或路径已变化"
        );
        frozen.verify()
    }
    pub(crate) fn recovery_source(&self) -> Result<crate::lost_void_recovery::RecoverySource> {
        ensure!(
            !self.recovery_reserved && self.inputs_released && self.control.requested().is_none(),
            "本层失败已经交接或输入未释放"
        );
        let (id, reason, memory) = match self.end {
            Some(LostVoidLevelEnd::Ready) => {
                let LevelOutcome::AgentDead { battle } =
                    &self.receipt().context("本层未安全交回")?.outcome
                else {
                    anyhow::bail!("恢复只接受实际阵亡或子步骤自身超时");
                };
                (
                    battle.encounter_id,
                    RetryReason::AgentDead,
                    self.memory.clone(),
                )
            }
            Some(LostVoidLevelEnd::TimedOut)
                if self.child_timeout && self.active_ms < u64::from(self.timeout_ms) =>
            {
                match self.phase {
                    LevelPhase::Navigation => {
                        let nav = self.navigation.as_ref().context("超时导航缺失")?;
                        ensure!(nav.own_timeout(), "导航没有独立超时或仍有未决交互");
                        let s = nav.snapshot();
                        (
                            s.flow.request().interaction_id,
                            RetryReason::NavigationTimeout,
                            s.flow.memory().clone(),
                        )
                    }
                    LevelPhase::Battle => {
                        let battle = self.battle.as_ref().context("超时战斗缺失")?;
                        ensure!(battle.own_timeout(), "战斗没有独立超时");
                        (
                            battle.snapshot().flow.request().encounter.interaction_id,
                            RetryReason::BattleTimeout,
                            self.memory.clone(),
                        )
                    }
                    _ => anyhow::bail!("该步骤超时不允许重开"),
                }
            }
            _ => anyhow::bail!("总预算、暂停或异常不能触发重开"),
        };
        let failure = LayerFailure {
            id,
            run_id: self.seed.run_id,
            attempt_id: self.level_id,
            reason,
        };
        Ok(crate::lost_void_recovery::RecoverySource {
            failure,
            memory,
            plan: self.plan.clone(),
            members: self
                .actual_members
                .clone()
                .context("恢复缺少实际完整队伍")?,
            options: self.options.clone(),
            initial_options: self.initial_options.clone(),
            budget: self.retry_budget.clone().unwrap_or(
                LayerRetryBudget::new(self.seed.run_id, self.level_id, self.region.clone())
                    .map_err(anyhow::Error::msg)?,
            ),
            limits: self.attempt_limits,
        })
    }
    pub(crate) fn reserve_recovery(&mut self) {
        self.recovery_reserved = true;
    }
    pub(crate) fn after_find_exit(
        frozen: &FrozenLostVoidPreparation,
        source: &crate::lost_void_recovery::RecoverySource,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        let mut memory = source.memory.clone();
        memory
            .abandon_entry_exclamations_after_timeout()
            .map_err(anyhow::Error::msg)?;
        let mut run = Self::new(
            frozen,
            source.failure.attempt_id,
            memory.region().into(),
            LevelStart::FindExit {
                failure: source.failure.clone(),
            },
            source.plan.clone(),
            source.members.len() as u8,
            Some(source.members.clone()),
            source.options.clone(),
            S::World,
            timeout_ms,
            control,
        )?;
        run.memory = memory;
        run.initial_options = source.initial_options.clone();
        run.retry_budget = Some(source.budget.clone());
        run.attempt_limits = source.limits;
        Ok(run)
    }
    pub fn after_recovery(
        frozen: &FrozenLostVoidPreparation,
        recovery: &mut crate::lost_void_recovery::LostVoidRecoveryRun,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(recovery.uses_control(&control), "重开后须共用实际恢复控制");
        recovery.verify_frozen(frozen)?;
        let receipt = recovery
            .receipt()
            .context("恢复尚未安全交回或已被消费")?
            .clone();
        let od_core::lost_void_recovery_menu::RecoveryOutcome::Restarted {
            attempt_id,
            screen,
            members,
        } = &receipt.outcome
        else {
            anyhow::bail!("结果页面不是重开到达");
        };
        let source = recovery.source();
        let mut budget = source.budget.clone();
        let intent = budget
            .pending_restart()
            .context("实际重开缺少待确认身份")?
            .clone();
        ensure!(intent.next_attempt_id == *attempt_id, "实际重开身份不匹配");
        budget
            .confirm_restart(&intent)
            .map_err(anyhow::Error::msg)?;
        let mut run = Self::new(
            frozen,
            *attempt_id,
            source.memory.region().into(),
            LevelStart::Recovery(Box::new(receipt.clone())),
            source.plan.clone(),
            source.members.len() as u8,
            members.clone().or_else(|| Some(source.members.clone())),
            source.initial_options.clone(),
            *screen,
            timeout_ms,
            control,
        )?;
        run.retry_budget = Some(budget);
        run.attempt_limits = source.limits;
        recovery.consume_handoff();
        Ok(run)
    }
    fn start_roster(&mut self, frozen: &FrozenLostVoidPreparation) -> Result<()> {
        let request = frozen.roster_request(
            &self.plan,
            self.level_id,
            self.members,
            self.options.clone(),
            120_000,
        )?;
        let roster = frozen
            .create_roster_run(request, self.control.clone())?
            .with_page_handoff()?;
        self.roster = Some(roster);
        self.phase = LevelPhase::Roster;
        self.child_started = false;
        Ok(())
    }
    fn start_event(
        &mut self,
        id: Uuid,
        target: NpcTarget,
        origin: LevelEventOrigin,
        pending: bool,
    ) -> Result<()> {
        let mut memory = self.memory.clone();
        if origin != LevelEventOrigin::NpcPage && !pending {
            memory
                .begin_event(id, target.clone(), origin == LevelEventOrigin::Battle)
                .map_err(anyhow::Error::msg)?;
        }
        let event = LostVoidEventRun::new(
            LostVoidEventRequest {
                run_id: self.seed.run_id,
                interaction_id: id,
                challenge: self.seed.challenge.clone(),
                options: self.options.clone(),
                entry: self.region == "入口",
                completed_first: memory.completed_first().to_vec(),
                members: self.members,
                allow_challenge_confirm: matches!(target,NpcTarget::Entry(ref region) if region.starts_with("挑战-")),
                allow_gear_notice: origin == LevelEventOrigin::Battle,
                timeout_ms: 300_000,
            },
            self.control.clone(),
        )?;
        self.memory = memory;
        self.event = Some(event);
        self.event_origin = origin;
        self.phase = LevelPhase::Event;
        self.child_started = false;
        Ok(())
    }
    fn add_event_options(&self, receipt: &LostVoidEventReceipt) -> Result<ArtifactSelection> {
        let mut options = self.options.clone();
        for rule in &receipt.append_priority {
            if !options.battle_priority.contains(rule) {
                options.battle_priority.push(rule.clone());
            }
        }
        choose_artifacts(&self.seed.challenge, &[], 0, &options).map_err(anyhow::Error::msg)?;
        Ok(options)
    }
    fn complete(&mut self, outcome: LevelOutcome) {
        self.receipt = Some(LostVoidLevelReceipt {
            run_id: self.seed.run_id,
            level_id: self.level_id,
            region: self.region.clone(),
            options: self.options.clone(),
            outcome,
        });
        self.completion_phase = Some(self.phase);
        self.phase = LevelPhase::Done;
    }
    fn complete_page(&mut self, screen: S) {
        self.complete(LevelOutcome::ObservedResult {
            screen,
            battle: self.last_battle.clone(),
        });
    }
    fn consume(&mut self, frozen: &FrozenLostVoidPreparation, end: ChildEnd) -> Result<()> {
        if let ChildEnd::Page(screen) = end {
            if matches!(screen, S::BattleResult | S::BattleFail) {
                self.complete_page(screen);
                return Ok(());
            }
            let origin = if self.phase == LevelPhase::Roster {
                LevelEventOrigin::RosterPage
            } else {
                LevelEventOrigin::NpcPage
            };
            return self.start_event(Uuid::new_v4(), NpcTarget::Anonymous, origin, false);
        }
        match self.phase {
            LevelPhase::Roster => {
                let roster = self.roster.as_ref().context("实际队伍执行器缺失")?;
                let receipt = roster.receipt().context("实际队伍尚未安全交回")?;
                ensure!(
                    receipt.run_id == self.seed.run_id
                        && receipt.level_id == self.level_id
                        && receipt.plan == self.plan
                        && receipt.members.len() == usize::from(self.members),
                    "层内实际队伍身份不符"
                );
                if let Some(expected) = &self.actual_members {
                    ensure!(
                        expected.len() == receipt.members.len()
                            && receipt.members.iter().all(|m| expected.contains(m)),
                        "层内完整队伍发生变化"
                    );
                }
                let options = receipt.options.clone();
                let actual_members = receipt.members.clone();
                let mut navigation = LostVoidNavigationRun::new(
                    NavRequest {
                        run_id: self.seed.run_id,
                        level_id: self.level_id,
                        interaction_id: Uuid::new_v4(),
                        challenge: self.seed.challenge.clone(),
                        memory: self.memory.clone(),
                        ignored: self.memory.ignored_regions(),
                        timeout_ms: self.attempt_limits.navigation_ms,
                        turn_sensitivity_milli: 1000,
                    },
                    self.control.clone(),
                )?;
                let attack = self
                    .seed
                    .settings
                    .binding("普通攻击")
                    .map_err(anyhow::Error::msg)?;
                if attack.len() == 1 {
                    navigation = navigation.with_obstacle_attack(attack[0])?;
                }
                self.actual_members = Some(actual_members);
                self.options = options;
                self.navigation = Some(navigation);
                self.phase = LevelPhase::Navigation;
                self.child_started = false;
            }
            LevelPhase::Navigation => {
                let navigation = self.navigation.as_ref().context("导航执行器缺失")?;
                let receipt = navigation.receipt().context("导航尚未安全交回")?.clone();
                let memory = navigation.snapshot().flow.memory().clone();
                match receipt.outcome {
                    NavOutcome::EncounteredBattle => {
                        let battle = frozen.create_battle_after_navigation(
                            self.roster.as_ref().unwrap(),
                            navigation,
                            self.attempt_limits.battle_ms,
                            self.control.clone(),
                        )?;
                        self.memory = memory;
                        self.battle = Some(battle);
                        self.phase = LevelPhase::Battle;
                        self.child_started = false;
                    }
                    NavOutcome::InteractionSubmitted {
                        target: NpcTarget::Entry(_),
                    } => {
                        let transition = frozen.create_transition_after_navigation(
                            self.roster.as_ref().unwrap(),
                            navigation,
                            Uuid::new_v4(),
                            300_000,
                            self.control.clone(),
                        )?;
                        self.memory = memory;
                        self.transition = Some(transition);
                        self.phase = LevelPhase::Transition;
                        self.child_started = false;
                    }
                    NavOutcome::ObservedResult => {
                        self.memory = memory;
                        self.complete_page(receipt.screen);
                    }
                    _ => {
                        let target = memory
                            .pending_event()
                            .context("导航事件目标缺失")?
                            .1
                            .clone();
                        self.memory = memory;
                        self.start_event(
                            receipt.interaction_id,
                            target,
                            LevelEventOrigin::Navigation,
                            true,
                        )?;
                    }
                }
            }
            LevelPhase::Battle => {
                let receipt = self
                    .battle
                    .as_ref()
                    .context("战斗执行器缺失")?
                    .receipt()
                    .context("战斗尚未安全交回")?
                    .clone();
                self.last_battle = Some(receipt.clone());
                match receipt.outcome {
                    BattleOutcome::AgentDead => {
                        self.complete(LevelOutcome::AgentDead { battle: receipt })
                    }
                    BattleOutcome::ObservedPage { screen }
                        if matches!(screen, S::BattleResult | S::BattleFail) =>
                    {
                        self.complete_page(screen)
                    }
                    BattleOutcome::ObservedPage { .. } => self.start_event(
                        receipt.encounter_id,
                        NpcTarget::Anonymous,
                        LevelEventOrigin::Battle,
                        false,
                    )?,
                    BattleOutcome::Exploration { .. } => self.start_roster(frozen)?,
                }
            }
            LevelPhase::Event => {
                let event = self.event.as_ref().context("事件执行器缺失")?;
                let receipt = event.receipt().context("事件尚未安全交回")?;
                let options = self.add_event_options(receipt)?;
                if matches!(receipt.returned_to, S::BattleResult | S::BattleFail) {
                    self.options = options;
                    self.complete_page(receipt.returned_to);
                } else if self.event_origin == LevelEventOrigin::NpcPage {
                    let mut memory = self.memory.clone();
                    memory
                        .absorb_inline_event(receipt)
                        .map_err(anyhow::Error::msg)?;
                    self.npc
                        .as_mut()
                        .context("被打断的 NPC 移动缺失")?
                        .continue_after_page()?;
                    self.memory = memory;
                    self.options = options;
                    self.phase = LevelPhase::NpcMove;
                    self.child_started = false;
                } else {
                    let mut memory = self.memory.clone();
                    let npc = LostVoidNpcRun::after_event(
                        &mut memory,
                        event,
                        120_000,
                        self.control.clone(),
                    )?
                    .with_page_handoff()?;
                    self.memory = memory;
                    self.options = options;
                    self.npc = Some(npc);
                    self.resume_roster = self.event_origin == LevelEventOrigin::RosterPage;
                    self.phase = LevelPhase::NpcMove;
                    self.child_started = false;
                }
            }
            LevelPhase::NpcMove => {
                let receipt = self
                    .npc
                    .as_ref()
                    .context("NPC 移动缺失")?
                    .receipt()
                    .context("NPC 移动尚未安全交回")?;
                let mut memory = self.memory.clone();
                memory.accept_move(receipt).map_err(anyhow::Error::msg)?;
                if self.resume_roster {
                    self.roster
                        .as_mut()
                        .context("被打断的队伍执行器缺失")?
                        .continue_after_page()?;
                    self.resume_roster = false;
                    self.phase = LevelPhase::Roster;
                    self.child_started = false;
                } else {
                    self.start_roster(frozen)?;
                }
                self.memory = memory;
            }
            LevelPhase::Transition => {
                let transition = self.transition.as_ref().context("跨层执行器缺失")?;
                let receipt = transition.receipt().context("跨层尚未安全交回")?.clone();
                let options = transition.artifact_options()?;
                match receipt.outcome {
                    TransitionOutcome::Arrived(arrival) => {
                        self.options = options;
                        self.complete(LevelOutcome::NextLevel { arrival });
                    }
                    TransitionOutcome::Result { screen } => {
                        self.options = options;
                        self.complete_page(screen);
                    }
                    TransitionOutcome::SameLevelEvent => {
                        let mut memory = self.memory.clone();
                        let npc = LostVoidNpcRun::after_event(
                            &mut memory,
                            transition
                                .same_level_event()
                                .context("门交互的实际事件缺失")?,
                            120_000,
                            self.control.clone(),
                        )?
                        .with_page_handoff()?;
                        self.memory = memory;
                        self.options = options;
                        self.npc = Some(npc);
                        self.resume_roster = false;
                        self.phase = LevelPhase::NpcMove;
                        self.child_started = false;
                    }
                    TransitionOutcome::Event { .. } => anyhow::bail!("跨层内部事件不能交给层外"),
                }
            }
            LevelPhase::Done => anyhow::bail!("已完成层不能再消费子结果"),
        }
        Ok(())
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidLevelEnd::Paused),
            "层流程尚未安全暂停"
        );
        self.resume_after_parent_pause()
    }
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidLevelEnd::Paused | LostVoidLevelEnd::Ready)
            ) && self.inputs_released,
            "层流程尚未安全暂停"
        );
        if self.phase == LevelPhase::Done {
            self.phase = self.completion_phase.context("完成层缺少实际来源阶段")?;
        }
        if self.child_started && !self.child_timeout {
            match self.phase {
                LevelPhase::Roster => self
                    .roster
                    .as_mut()
                    .context("队伍暂停状态缺失")?
                    .resume_after_parent_pause()?,
                LevelPhase::Navigation => self
                    .navigation
                    .as_mut()
                    .context("导航暂停状态缺失")?
                    .resume_after_parent_pause()?,
                LevelPhase::Battle => self
                    .battle
                    .as_mut()
                    .context("战斗暂停状态缺失")?
                    .resume_after_parent_pause()?,
                LevelPhase::Event => self
                    .event
                    .as_mut()
                    .context("事件暂停状态缺失")?
                    .resume_after_parent_pause()?,
                LevelPhase::NpcMove => self
                    .npc
                    .as_mut()
                    .context("移动暂停状态缺失")?
                    .resume_after_parent_pause()?,
                LevelPhase::Transition => self
                    .transition
                    .as_mut()
                    .context("跨层暂停状态缺失")?
                    .resume_after_parent_pause()?,
                LevelPhase::Done => unreachable!(),
            }
        } else {
            ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        }
        self.receipt = None;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidLevelEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidLevelEnd::Paused,
            Interrupt::Stop => LostVoidLevelEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        frozen: &FrozenLostVoidPreparation,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidLevelSnapshot) -> Result<()>,
    ) -> Result<LostVoidLevelEnd> {
        use LostVoidLevelEnd as E;
        ensure!(
            self.end.is_none()
                || self.end == Some(E::Paused) && self.control.requested() == Some(Interrupt::Stop),
            "层流程不能直接重跑"
        );
        self.memory.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.active_ms;
        let budget = Duration::from_millis(u64::from(self.timeout_ms).saturating_sub(initial))
            .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<E> {
            if let Some(end) = self.interrupted() {
                return Ok(end);
            }
            if remaining().is_zero() {
                return Ok(E::TimedOut);
            }
            self.verify_frozen(frozen)?;
            session.check_ether_synthesis_target(game)?;
            session.release_inputs()?;
            if self.recognizer.is_none() {
                session.load_lost_void_scene_bundle(&self.scene)?;
                session.load_challenge_detector(&self.detector)?;
                self.recognizer = Some(session.load_combat_bundle(&self.combat)?);
            }
            if self.reader.is_none() {
                self.reader = Some(LostVoidHudReader::new(self.recognizer.as_ref().unwrap())?);
            }
            session.require_lost_void_battle_roots(&self.scene, &self.combat, &self.detector)?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                self.verify_frozen(frozen)?;
                if self.child_timeout {
                    return Ok(E::TimedOut);
                }
                if self.phase == LevelPhase::Done {
                    ensure!(self.receipt.is_some(), "完成层缺少实际回执");
                    return Ok(E::Ready);
                }
                ensure!(self.steps < 4096, "层流程交接次数达到上限");
                self.steps += 1;
                self.child_started = true;
                self.inputs_released = false;
                let mut saved = self.snapshot();
                macro_rules! save {
                    ($field:ident,$snapshot:expr) => {{
                        let snapshot = $snapshot;
                        saved.inputs_released = snapshot.inputs_released;
                        saved.$field = Some(snapshot);
                        saved.active_ms =
                            initial.saturating_add(started.elapsed().as_millis() as u64);
                        publish(saved.clone())
                    }};
                }
                macro_rules! end {
                    ($value:expr,$kind:ident) => {
                        match $value {
                            $kind::Ready => ChildEnd::Ready,
                            $kind::Paused => ChildEnd::Paused,
                            $kind::Stopped => ChildEnd::Stopped,
                            $kind::TimedOut => ChildEnd::TimedOut,
                            $kind::Failed => ChildEnd::Failed,
                        }
                    };
                }
                let child = match self.phase {
                    LevelPhase::Roster => self
                        .roster
                        .as_mut()
                        .context("队伍执行器缺失")?
                        .execute_with_budget(
                            session,
                            self.reader.as_mut().unwrap(),
                            game,
                            remaining(),
                            |s| save!(roster, s),
                        )
                        .map(|e| match e {
                            LostVoidRosterEnd::Ready => ChildEnd::Ready,
                            LostVoidRosterEnd::ObservedPage(s) => ChildEnd::Page(s),
                            LostVoidRosterEnd::Paused => ChildEnd::Paused,
                            LostVoidRosterEnd::Stopped => ChildEnd::Stopped,
                            LostVoidRosterEnd::TimedOut => ChildEnd::TimedOut,
                            LostVoidRosterEnd::Failed => ChildEnd::Failed,
                        }),
                    LevelPhase::Navigation => self
                        .navigation
                        .as_mut()
                        .context("导航执行器缺失")?
                        .execute_with_budget(
                            session,
                            self.recognizer.as_mut().unwrap(),
                            game,
                            remaining(),
                            |s| save!(navigation, s),
                        )
                        .map(|e| end!(e, LostVoidNavigationEnd)),
                    LevelPhase::Battle => self
                        .battle
                        .as_mut()
                        .context("战斗执行器缺失")?
                        .execute_with_budget(frozen, session, game, remaining(), |s| {
                            save!(battle, s)
                        })
                        .map(|e| end!(e, LostVoidBattleEnd)),
                    LevelPhase::Event => self
                        .event
                        .as_mut()
                        .context("事件执行器缺失")?
                        .execute_with_budget(session, game, remaining(), |s| save!(event, s))
                        .map(|e| end!(e, LostVoidEventEnd)),
                    LevelPhase::NpcMove => self
                        .npc
                        .as_mut()
                        .context("NPC 执行器缺失")?
                        .execute_with_budget(session, game, remaining(), |s| save!(npc, s))
                        .map(|e| match e {
                            LostVoidNpcEnd::Ready => ChildEnd::Ready,
                            LostVoidNpcEnd::ObservedPage(s) => ChildEnd::Page(s),
                            LostVoidNpcEnd::Paused => ChildEnd::Paused,
                            LostVoidNpcEnd::Stopped => ChildEnd::Stopped,
                            LostVoidNpcEnd::TimedOut => ChildEnd::TimedOut,
                            LostVoidNpcEnd::Failed => ChildEnd::Failed,
                        }),
                    LevelPhase::Transition => self
                        .transition
                        .as_mut()
                        .context("跨层执行器缺失")?
                        .execute_with_budget(
                            frozen,
                            session,
                            self.recognizer.as_mut().unwrap(),
                            game,
                            remaining(),
                            |s| save!(transition, s),
                        )
                        .map(|e| end!(e, LostVoidTransitionEnd)),
                    LevelPhase::Done => unreachable!(),
                };
                self.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
                self.inputs_released = session.inputs_released();
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(E::TimedOut);
                }
                match child? {
                    ChildEnd::Paused => return Ok(E::Paused),
                    ChildEnd::Stopped => return Ok(E::Stopped),
                    ChildEnd::TimedOut => {
                        self.child_timeout = match self.phase {
                            LevelPhase::Navigation => {
                                self.navigation.as_ref().is_some_and(|n| n.own_timeout())
                            }
                            LevelPhase::Battle => {
                                self.battle.as_ref().is_some_and(|b| b.own_timeout())
                            }
                            _ => false,
                        };
                        return Ok(E::TimedOut);
                    }
                    ChildEnd::Failed => return Ok(E::Failed),
                    done => {
                        if let Err(error) = self.consume(frozen, done) {
                            if let Some(end) = self.interrupted() {
                                return Ok(end);
                            }
                            return Err(error);
                        }
                    }
                }
                self.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot())?;
            }
        })();
        self.inputs_released = session.release_inputs().is_ok();
        self.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() || !self.inputs_released {
            self.end = Some(E::Failed);
        }
        ensure!(self.inputs_released, "层流程收尾无法确认输入释放");
        let mut end = result?;
        self.verify_frozen(frozen).inspect_err(|_| {
            self.end = Some(E::Failed);
        })?;
        if let Err(error) = publish(self.snapshot()) {
            self.end = Some(E::Failed);
            return Err(error);
        }
        self.verify_frozen(frozen).inspect_err(|_| {
            self.end = Some(E::Failed);
        })?;
        self.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            self.child_timeout = false;
            end = E::TimedOut;
        }
        if end == E::Paused && !self.control.acknowledge_pause() {
            end = E::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
