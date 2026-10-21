//! 情报板完整执行器；同一控制句柄、运行 ID 和原生槽位贯穿界面、移动与战斗。
use crate::combat::{CombatRun, CombatSegmentEnd};
use anyhow::{Context, Result, ensure};
use maa_adapter::{
    GameSessionConfig, InferenceDevice, NativeSession, PipelineEnd, SessionHandoff,
    challenge_move::{MoveEnd, MoveOptions},
    combat_vision::{CombatBundle, CombatRecognizer},
    intel_board::{IntelBoardRead, IntelPickEnd},
    team_selection::SelectionEnd,
};
use od_core::{
    CaptureMethod, InputMode, WindowInfo,
    app_combat::AppCombatPlan,
    basic_daily::DailyProgress,
    conditional::Script,
    intel_board_flow::IntelOutcome,
    mail::MailState,
    pipeline::{Interrupt, PipelineControl, PipelineRequest},
    predefined_teams::TeamConfig,
};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub struct PreparedIntel {
    pub plan: AppCombatPlan,
    script: Script,
    root: PathBuf,
    manifest_hash: Vec<u8>,
}
impl PreparedIntel {
    /// 整组第一次输入前编译并固定脚本；其后的导入选择变化不改变已编译指令。
    pub fn load(plan: AppCombatPlan, root: &Path, data: &Path) -> Result<Self> {
        Self::from_bundle(plan, CombatBundle::load(root)?, data)
    }
    pub(crate) fn from_bundle(
        plan: AppCombatPlan,
        bundle: CombatBundle,
        data: &Path,
    ) -> Result<Self> {
        plan.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            crate::combat::script_catalog(&bundle, data)?.revision == plan.script_revision,
            "战斗脚本或资源已变化，请重新预览情报板计划"
        );
        let script_root = od_core::combat_scripts::select_root(
            data,
            &bundle.root.join("scripts"),
            "auto_battle",
            &plan.config.script,
        )?;
        let script = Script::load_entry(
            &script_root,
            "auto_battle",
            &plan.config.script,
            plan.config.use_merged,
        )?;
        bundle.spec.validate_script(&script)?;
        plan.config.validate_script(&script)?;
        ensure!(
            crate::combat::script_catalog(&bundle, data)?.revision == plan.script_revision,
            "编译期间战斗脚本或资源已变化，请重新预览"
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

pub struct IntelSessionOptions<'a> {
    pub window: &'a WindowInfo,
    pub capture: CaptureMethod,
    pub resources: &'a Path,
    pub fixture: bool,
}

pub struct IntelRun {
    pub id: Uuid,
    pub progress: DailyProgress,
    pub message: String,
    prepared: PreparedIntel,
    teams: TeamConfig,
    control: PipelineControl,
    combat: Option<CombatRun>,
    recognizer: Option<CombatRecognizer>,
    loaded: Option<bool>,
    steps: u32,
    deploy_waits: u8,
}
impl IntelRun {
    pub fn new(
        id: Uuid,
        progress: DailyProgress,
        prepared: PreparedIntel,
        teams: TeamConfig,
    ) -> Result<Self> {
        ensure!(
            progress.app_id == "intel_board"
                && progress.app_combat.as_ref() == Some(&prepared.plan),
            "情报板与冻结战斗设置不符"
        );
        let ledger = progress.intel_board.as_ref().context("情报板缺少周记录")?;
        progress
            .intel_flow
            .as_ref()
            .context("情报板缺少主流程")?
            .validate(ledger)
            .map_err(anyhow::Error::msg)?;
        teams.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            ledger.config.script(&teams).map_err(anyhow::Error::msg)?
                == prepared.plan.config.script,
            "冻结战斗脚本不是情报板选定的编队脚本"
        );
        Ok(Self {
            id,
            progress,
            message: "情报板准备中".into(),
            prepared,
            teams,
            control: PipelineControl::default(),
            combat: None,
            recognizer: None,
            loaded: None,
            steps: 0,
            deploy_waits: 0,
        })
    }
    pub fn control(&self) -> PipelineRequest {
        self.control.requester()
    }
    pub fn interrupt_requested(&self) -> Option<Interrupt> {
        self.control.requested()
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.progress.state == MailState::Paused && self.progress.inputs_released,
            "情报板尚未安全暂停"
        );
        let mut next = self.progress.clone();
        next.intel_flow
            .as_mut()
            .unwrap()
            .resume(next.intel_board.as_mut().unwrap())
            .map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "情报板已请求停止，不能继续");
        next.stage = next.intel_flow.as_ref().unwrap().stage.clone();
        next.state = MailState::Running;
        self.progress = next;
        Ok(())
    }
    fn sync(&mut self) {
        self.progress.stage = self.progress.intel_flow.as_ref().unwrap().stage.clone();
    }
    fn observe(&mut self, node: &str) -> Result<()> {
        self.progress
            .intel_flow
            .as_mut()
            .unwrap()
            .observe(self.progress.intel_board.as_mut().unwrap(), node)
            .map_err(anyhow::Error::msg)?;
        self.sync();
        Ok(())
    }
    fn publish(
        &mut self,
        started: Instant,
        initial_ms: u64,
        publish: &mut impl FnMut(DailyProgress) -> Result<()>,
    ) -> Result<()> {
        self.sync();
        self.progress.active_ms = initial_ms.saturating_add(started.elapsed().as_millis() as u64);
        publish(self.progress.clone())
    }
    fn interrupted(&mut self) -> bool {
        match self.control.requested() {
            Some(Interrupt::Stop) => {
                self.control.finish();
                self.progress.state = MailState::Stopped;
                self.message = "情报板已停止，保留已结算周记录".into();
                true
            }
            Some(Interrupt::Pause) => {
                if !self.control.acknowledge_pause() {
                    return self.interrupted();
                }
                self.progress.state = MailState::Paused;
                self.message = "情报板已暂停，输入已释放并保留当前委托".into();
                true
            }
            None => false,
        }
    }
    fn session<'a>(
        &mut self,
        slot: &'a mut Option<NativeSession>,
        options: &IntelSessionOptions<'_>,
        battle: bool,
    ) -> Result<Option<&'a mut NativeSession>> {
        let cancel = self.control.cancellation();
        if options.fixture {
            slot.as_mut().context("离线情报板控制器未准备")?.quiesce()?;
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(None);
            }
            slot.as_ref().unwrap().check_combat_input_mode(if battle {
                self.prepared.plan.config.mode
            } else {
                InputMode::Foreground
            })?;
        } else {
            let config = &self.prepared.plan.config;
            let handoff = NativeSession::handoff_game(
                slot,
                GameSessionConfig {
                    window: options.window,
                    resources: options.resources,
                    capture: options.capture,
                    input: if battle {
                        config.mode
                    } else {
                        InputMode::Foreground
                    },
                    device: if battle {
                        config
                            .directml_device
                            .map_or(InferenceDevice::Cpu, InferenceDevice::DirectMl)
                    } else {
                        InferenceDevice::Cpu
                    },
                },
                &cancel,
            )?;
            if handoff == SessionHandoff::Cancelled {
                return Ok(None);
            }
            if handoff == SessionHandoff::Created {
                self.loaded = None;
            }
        }
        let session = slot.as_mut().context("情报板控制器缺失")?;
        if self.loaded != Some(battle) {
            ensure!(
                Sha256::digest(std::fs::read(
                    self.prepared.root.join("combat-manifest.json")
                )?)
                .as_slice()
                    == self.prepared.manifest_hash,
                "运行期间战斗资源已变化"
            );
            self.recognizer = Some(session.load_combat_bundle(&self.prepared.root)?);
            if !battle {
                let daily = maa_adapter::validate_basic_daily_app_bundle(
                    &options.resources.join("generated/basic-daily"),
                    "intel_board",
                )?;
                session.load_basic_daily_bundle(&daily)?;
                session.load_challenge_detector(&daily)?;
            }
            self.loaded = Some(battle);
        }
        Ok(Some(session))
    }
    pub fn execute(
        &mut self,
        slot: &mut Option<NativeSession>,
        options: IntelSessionOptions<'_>,
        mut publish: impl FnMut(DailyProgress) -> Result<()>,
    ) -> Result<()> {
        ensure!(
            self.progress.state == MailState::Running
                || (self.progress.state == MailState::Paused
                    && self.control.requested() == Some(Interrupt::Stop)),
            "情报板不能直接重跑"
        );
        let started = Instant::now();
        let initial_ms = self.progress.active_ms;
        let result = (|| -> Result<()> {
            if let Some(session) = slot.as_mut() {
                session.quiesce()?;
            }
            self.progress.inputs_released = true;
            if self.interrupted() {
                return Ok(());
            }
            if !options.fixture {
                let Some(session) = self.session(slot, &options, false)? else {
                    self.interrupted();
                    return Ok(());
                };
                session.prepare_game_mail(
                    self.control.cancellation(),
                    self.progress.config.start_delay_ms,
                )?;
            }
            loop {
                if self.interrupted() {
                    return Ok(());
                }
                let used = initial_ms.saturating_add(started.elapsed().as_millis() as u64);
                let remaining = Duration::from_millis(
                    u64::from(self.progress.timeout_ms()).saturating_sub(used),
                );
                if remaining.is_zero() {
                    self.progress.state = MailState::TimedOut;
                    self.message = "情报板达到总运行超时，保留已结算周记录".into();
                    self.control.finish();
                    return Ok(());
                }
                ensure!(self.steps < 16384, "情报板步骤达到上限");
                if self.progress.stage == "Done" {
                    self.progress.completion_proven = true;
                    self.progress.returned_world = true;
                    ensure!(self.progress.completed(), "情报板完成证明不足");
                    self.progress.state = MailState::Succeeded;
                    self.message = match self.progress.intel_flow.as_ref().unwrap().outcome {
                        Some(IntelOutcome::WeeklyGoal) => {
                            "情报板周目标完成，已返回大世界并释放输入"
                        }
                        _ => "情报板当前无可代行委托，已返回大世界；周目标仍未完成",
                    }
                    .into();
                    if self.control.finish() == Some(Interrupt::Stop) {
                        self.progress.state = MailState::Stopped;
                    }
                    return Ok(());
                }
                let stage = self.progress.stage.clone();
                let before = self.progress.clone();
                self.progress
                    .intel_flow
                    .as_mut()
                    .unwrap()
                    .before_step(self.progress.intel_board.as_mut().unwrap())
                    .map_err(anyhow::Error::msg)?;
                // 在任何输入及模式切换前持久化；失败保留前一份有效检查点。
                if let Err(error) = self.publish(started, initial_ms, &mut publish) {
                    self.progress = before;
                    return Err(error);
                }
                let Some(session) = self.session(slot, &options, stage == "Combat")? else {
                    self.interrupted();
                    return Ok(());
                };
                if self.control.requested().is_some() {
                    self.interrupted();
                    return Ok(());
                }
                let remaining =
                    Duration::from_millis(u64::from(self.progress.timeout_ms()).saturating_sub(
                        initial_ms.saturating_add(started.elapsed().as_millis() as u64),
                    ));
                ensure!(
                    !remaining.is_zero(),
                    "情报板总运行预算已耗尽，未投递后续输入"
                );
                self.progress.inputs_released = false;
                self.steps += 1;
                match stage.as_str() {
                    "ReadBoard" | "FindCommission" => {
                        if let IntelBoardRead::Board {
                            contribution,
                            commission,
                        } = session.read_intel_board(&self.control.cancellation(), remaining)?
                        {
                            let flow = self.progress.intel_flow.as_mut().unwrap();
                            let ledger = self.progress.intel_board.as_mut().unwrap();
                            if stage == "ReadBoard" {
                                flow.read_board(ledger, contribution)
                            } else {
                                flow.scan_board(ledger, contribution, commission.map(|c| c.kind))
                            }
                            .map_err(anyhow::Error::msg)?;
                        } else {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    }
                    "OpenCommission" => {
                        let end = session.choose_intel_commission(
                            self.progress.intel_flow.as_ref().unwrap(),
                            self.progress.intel_board.as_ref().unwrap(),
                            self.control.cancellation(),
                            remaining,
                            !options.fixture,
                        )?;
                        match end {
                            IntelPickEnd::Opened => self.observe("Intel.CommissionSelected")?,
                            IntelPickEnd::Changed => self.observe("Intel.CandidateChanged")?,
                            IntelPickEnd::TimedOut => anyhow::bail!("情报板选帖超时"),
                            _ => {}
                        }
                    }
                    "SelectTeam" => {
                        let teams = self.teams.clone();
                        let mut selection = self
                            .progress
                            .intel_flow
                            .as_ref()
                            .unwrap()
                            .selection
                            .clone()
                            .context("缺少选队检查点")?;
                        let end = session.select_predefined_teams(
                            &teams,
                            &mut selection,
                            !options.fixture,
                            self.control.cancellation(),
                            remaining.min(Duration::from_secs(120)),
                            |selection| {
                                self.progress.intel_flow.as_mut().unwrap().selection =
                                    Some(selection);
                                self.publish(started, initial_ms, &mut publish)
                            },
                        );
                        self.progress.intel_flow.as_mut().unwrap().selection = Some(selection);
                        if end? == SelectionEnd::Completed {
                            self.progress
                                .intel_flow
                                .as_mut()
                                .unwrap()
                                .selection_completed(self.progress.intel_board.as_mut().unwrap())
                                .map_err(anyhow::Error::msg)?;
                        } else if self.control.requested().is_none() {
                            anyhow::bail!("情报板选队超时");
                        }
                    }
                    "Move" => {
                        let mut movement = self
                            .progress
                            .intel_flow
                            .as_ref()
                            .unwrap()
                            .movement
                            .clone()
                            .context("缺少战前移动检查点")?;
                        let recognizer = self.recognizer.as_ref().context("缺少战斗识别资源")?;
                        let mut checkpoint = self.progress.clone();
                        let end = session.execute_challenge_move(
                            recognizer,
                            &mut movement,
                            MoveOptions {
                                game: !options.fixture,
                                timeout: remaining.min(Duration::from_secs(120)),
                                turn_sensitivity_milli: self
                                    .prepared
                                    .plan
                                    .config
                                    .turn_sensitivity_milli,
                                max_frame_age_ms: 1000,
                            },
                            self.control.cancellation(),
                            |movement| {
                                checkpoint.intel_flow.as_mut().unwrap().movement = Some(movement);
                                checkpoint.active_ms =
                                    initial_ms.saturating_add(started.elapsed().as_millis() as u64);
                                publish(checkpoint.clone())
                            },
                        );
                        self.progress.intel_flow.as_mut().unwrap().movement = Some(movement);
                        if end? == MoveEnd::Completed {
                            self.progress
                                .intel_flow
                                .as_mut()
                                .unwrap()
                                .movement_completed(self.progress.intel_board.as_mut().unwrap())
                                .map_err(anyhow::Error::msg)?;
                        } else if self.control.requested().is_none() {
                            anyhow::bail!("情报板战前移动超时");
                        }
                    }
                    "Combat" => {
                        if self.combat.is_none() {
                            let mut combat = CombatRun::new_segment(
                                self.id,
                                self.prepared.plan.config.clone(),
                                self.prepared.script.clone(),
                                self.control.clone(),
                            )?;
                            combat.progress.script_revision =
                                self.prepared.plan.script_revision.clone();
                            self.combat = Some(combat);
                        }
                        let mut checkpoint = self.progress.clone();
                        let end = self.combat.as_mut().unwrap().execute_segment_with_budget(
                            session,
                            self.recognizer.as_mut().unwrap(),
                            !options.fixture,
                            remaining,
                            |battle| {
                                checkpoint.intel_battle = Some(battle);
                                checkpoint.active_ms =
                                    initial_ms.saturating_add(started.elapsed().as_millis() as u64);
                                publish(checkpoint.clone())
                            },
                        );
                        self.progress.intel_battle =
                            Some(self.combat.as_ref().unwrap().progress.clone());
                        if let CombatSegmentEnd::Result(result) = end? {
                            self.progress
                                .intel_flow
                                .as_mut()
                                .unwrap()
                                .battle_completed(
                                    self.progress.intel_board.as_mut().unwrap(),
                                    result,
                                )
                                .map_err(anyhow::Error::msg)?;
                            self.combat = None;
                        }
                    }
                    _ => {
                        if stage == "WaitDeploy"
                            && session.intel_battle_ready(
                                self.recognizer.as_ref().unwrap(),
                                &self.control.cancellation(),
                                remaining,
                            )?
                        {
                            self.observe("Intel.BattleReady")?;
                            self.deploy_waits = 0;
                            session.quiesce()?;
                            self.progress.inputs_released = true;
                            self.publish(started, initial_ms, &mut publish)?;
                            continue;
                        }
                        let step = session.intel_board_step(
                            self.progress.intel_flow.as_ref().unwrap(),
                            self.progress.intel_board.as_ref().unwrap(),
                            self.control.cancellation(),
                            remaining.min(Duration::from_secs(if stage == "WaitDeploy" {
                                3
                            } else {
                                60
                            })),
                            !options.fixture,
                        )?;
                        ensure!(step.report.released, "情报板 Pipeline 未释放输入");
                        match step.report.end {
                            PipelineEnd::Succeeded => self.observe(
                                &step
                                    .nodes
                                    .iter()
                                    .rev()
                                    .find(|n| n.completed)
                                    .context("缺少情报板完成节点")?
                                    .name,
                            )?,
                            PipelineEnd::Cancelled => {
                                ensure!(self.control.requested().is_some(), "未登记的情报板取消")
                            }
                            PipelineEnd::TimedOut
                                if stage == "WaitDeploy" && self.deploy_waits < 20 =>
                            {
                                self.deploy_waits += 1;
                            }
                            PipelineEnd::TimedOut => anyhow::bail!("情报板界面步骤超时"),
                            PipelineEnd::Failed => anyhow::bail!("情报板界面步骤失败"),
                        }
                    }
                }
                session.quiesce()?;
                self.progress.inputs_released = true;
                // 先保存已确认的结果，再响应暂停，避免重复结算或重新选帖。
                self.publish(started, initial_ms, &mut publish)?;
                if self.interrupted() {
                    return Ok(());
                }
            }
        })();
        let released = slot.as_mut().map_or(Ok(()), NativeSession::quiesce);
        self.progress.inputs_released = released.is_ok();
        if let Some(battle) = &mut self.progress.intel_battle {
            battle.inputs_released = released.is_ok();
        }
        self.sync();
        self.progress.active_ms = initial_ms.saturating_add(started.elapsed().as_millis() as u64);
        let result = result.and(released);
        if let Err(error) = &result {
            self.control.finish();
            self.progress.state =
                if self.progress.active_ms >= u64::from(self.progress.timeout_ms()) {
                    MailState::TimedOut
                } else {
                    MailState::Failed
                };
            self.message = format!("情报板执行失败：{error:#}");
        }
        result
    }
}
