//! 锄大地完整执行器。UI、步行和战斗共用运行槽、取消句柄与持久化检查点。
use crate::{
    combat::{CombatRun, CombatSegmentEnd},
    intel_board::{IntelSessionOptions, PreparedIntel},
};
use anyhow::{Context, Result, bail, ensure};
use maa_adapter::{
    GameSessionConfig, InferenceDevice, NativeSession, SessionHandoff,
    combat_vision::CombatRecognizer,
    world_patrol::{
        PatrolAreaStep, PatrolFilterPhase, PatrolFilterStep, PatrolSearchState, PatrolSearchStep,
    },
};
use od_core::{
    InputMode,
    basic_daily::DailyProgress,
    mail::MailState,
    pipeline::{Interrupt, PipelineControl, PipelineRequest},
    world_patrol::{self, PatrolPoint, PatrolRetryAction},
};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub struct PatrolRun {
    pub id: Uuid,
    pub progress: DailyProgress,
    pub message: String,
    prepared: PreparedIntel,
    control: PipelineControl,
    loaded: Option<bool>,
    recognizer: Option<CombatRecognizer>,
    combat: Option<CombatRun>,
    position: Option<PatrolPoint>,
    no_pos: Option<Instant>,
    stuck: Option<(PatrolPoint, Instant)>,
    backtrack: Option<(PatrolPoint, Instant)>,
    unstuck: u8,
    search_index: usize,
    search: PatrolSearchState,
    stage_since: Instant,
    tracked_stage: String,
    day: od_core::daily::GameDay,
}
impl PatrolRun {
    pub fn new(
        id: Uuid,
        progress: DailyProgress,
        prepared: PreparedIntel,
        day: od_core::daily::GameDay,
    ) -> Result<Self> {
        ensure!(
            progress.app_id == "world_patrol"
                && progress.app_combat.as_ref() == Some(&prepared.plan),
            "锄大地冻结战斗设置不符"
        );
        progress
            .world_patrol
            .as_ref()
            .context("缺少锄大地检查点")?
            .validate()
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            id,
            progress,
            message: "锄大地准备中".into(),
            prepared,
            control: PipelineControl::default(),
            loaded: None,
            recognizer: None,
            combat: None,
            position: None,
            no_pos: None,
            stuck: None,
            backtrack: None,
            unstuck: 0,
            search_index: 0,
            search: PatrolSearchState::default(),
            stage_since: Instant::now(),
            tracked_stage: String::new(),
            day,
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
            "锄大地尚未安全暂停"
        );
        ensure!(self.control.resume(), "锄大地已请求停止");
        if !matches!(
            self.progress.stage.as_str(),
            "Wait"
                | "Done"
                | "Leave"
                | "LeaveArea"
                | "LeavePoint"
                | "LeaveConfirm"
                | "LeaveArrival"
        ) {
            self.progress.stage = "Inspect".into();
            self.progress.world_patrol.as_mut().unwrap().point_index = 0;
        }
        self.combat = None;
        self.position = None;
        self.no_pos = None;
        self.stuck = None;
        self.backtrack = None;
        self.tracked_stage.clear();
        self.progress.state = MailState::Running;
        Ok(())
    }
    fn interrupted(&mut self) -> bool {
        match self.control.requested() {
            Some(Interrupt::Stop) => {
                self.control.finish();
                self.progress.state = MailState::Stopped;
                self.message = "锄大地已停止，保留已完成路线".into();
                true
            }
            Some(Interrupt::Pause) => {
                if !self.control.acknowledge_pause() {
                    return self.interrupted();
                }
                self.progress.state = MailState::Paused;
                self.message = "锄大地已暂停；继续后重新传送并定位当前路线".into();
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
            slot.as_mut().context("锄大地回放控制器未准备")?.quiesce()?;
            if cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
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
        let session = slot.as_mut().context("锄大地缺少控制器")?;
        session.check_combat_input_mode(if battle {
            self.prepared.plan.config.mode
        } else {
            InputMode::Foreground
        })?;
        if self.loaded != Some(battle) {
            use sha2::{Digest, Sha256};
            ensure!(
                Sha256::digest(std::fs::read(
                    self.prepared.root.join("combat-manifest.json")
                )?)
                .as_slice()
                    == self.prepared.manifest_hash,
                "运行中战斗资源已变化"
            );
            self.recognizer = Some(session.load_combat_bundle(&self.prepared.root)?);
            session.load_world_patrol_bundle(&options.resources.join("generated/world-patrol"))?;
            self.loaded = Some(battle);
        }
        Ok(Some(session))
    }
    fn retry_route(&mut self) -> Result<()> {
        let patrol = self.progress.world_patrol.as_mut().unwrap();
        if patrol.attempt < patrol.config.route_retry_times {
            patrol.attempt += 1;
            patrol.point_index = 0;
            self.progress.stage = "Inspect".into();
        } else {
            patrol.complete_route(false).map_err(anyhow::Error::msg)?;
            self.progress.stage = "Inspect".into();
        }
        self.position = None;
        self.no_pos = None;
        self.stuck = None;
        self.backtrack = None;
        self.unstuck = 0;
        Ok(())
    }
    pub fn execute(
        &mut self,
        slot: &mut Option<NativeSession>,
        options: IntelSessionOptions<'_>,
        mut publish: impl FnMut(DailyProgress) -> Result<()>,
    ) -> Result<()> {
        let started = Instant::now();
        let initial = self.progress.active_ms;
        let root = options.resources.join("generated/world-patrol");
        let result = (|| -> Result<()> {
            if let Some(s) = slot.as_mut() {
                s.quiesce()?;
            }
            self.progress.inputs_released = true;
            if self.interrupted() {
                return Ok(());
            }
            maa_adapter::world_patrol::validate_bundle(&root)?;
            if !options.fixture {
                let cancel = self.control.cancellation();
                let delay = self.progress.config.start_delay_ms;
                if let Some(session) = self.session(slot, &options, false)? {
                    session.prepare_game_mail(cancel, delay)?;
                }
            }
            loop {
                if self.interrupted() {
                    return Ok(());
                }
                ensure!(
                    options.fixture || self.day.contains(chrono::Utc::now()),
                    "锄大地已跨游戏日，请重新预览"
                );
                self.progress.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                let used = self.progress.active_ms;
                let timeout = self.progress.timeout_ms();
                ensure!(used < u64::from(timeout), "锄大地达到总运行超时");
                let remaining = Duration::from_millis(u64::from(timeout) - used);
                if self.progress.stage != self.tracked_stage {
                    self.tracked_stage = self.progress.stage.clone();
                    self.stage_since = Instant::now();
                    self.search_index = 0;
                    if self.progress.stage == "Search" {
                        self.search = PatrolSearchState::default();
                    }
                }
                ensure!(
                    matches!(self.progress.stage.as_str(), "Move" | "Combat" | "Wait")
                        || self.stage_since.elapsed() < Duration::from_secs(90),
                    "锄大地界面步骤超时：{}",
                    self.progress.stage
                );
                if !matches!(self.progress.stage.as_str(), "Wait" | "Done") {
                    self.progress.inputs_released = false;
                }
                publish(self.progress.clone())?;
                if self.progress.stage == "Done" {
                    self.progress.completion_proven = true;
                    self.progress.returned_world = true;
                    ensure!(self.progress.completed(), "锄大地完成证明不足");
                    self.progress.state = MailState::Succeeded;
                    self.message = "锄大地全部路线与循环完成，已返回录像店并释放输入".into();
                    if self.control.finish() == Some(Interrupt::Stop) {
                        self.progress.state = MailState::Stopped;
                        self.message = "锄大地已停止，输入已释放".into();
                    }
                    return Ok(());
                }
                if self.progress.stage == "Wait" {
                    let patrol = self.progress.world_patrol.as_mut().unwrap();
                    if patrol
                        .next_round_ms
                        .is_some_and(|deadline| used >= deadline)
                    {
                        patrol.next_round(used).map_err(anyhow::Error::msg)?;
                        self.progress.stage = "Inspect".into();
                    } else {
                        let waiting = Instant::now();
                        let cancel = self.control.cancellation();
                        while waiting.elapsed() < Duration::from_secs(1)
                            && !cancel.load(Ordering::Acquire)
                        {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                    }
                    continue;
                }
                let stage = self.progress.stage.clone();
                let Some(session) = self.session(slot, &options, stage == "Combat")? else {
                    self.interrupted();
                    return Ok(());
                };
                if stage == "Combat" {
                    if self.combat.is_none() {
                        let mut combat = CombatRun::new_segment(
                            self.id,
                            self.prepared.plan.config.clone(),
                            self.prepared.script.clone(),
                            self.control.clone(),
                        )?
                        .for_world_patrol(
                            self.progress
                                .world_patrol
                                .as_ref()
                                .unwrap()
                                .config
                                .ui_disappear_seconds,
                        );
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
                            checkpoint.inputs_released = battle.inputs_released;
                            checkpoint.intel_battle = Some(battle);
                            checkpoint.active_ms =
                                initial.saturating_add(started.elapsed().as_millis() as u64);
                            publish(checkpoint.clone())
                        },
                    );
                    self.progress.intel_battle =
                        Some(self.combat.as_ref().unwrap().progress.clone());
                    self.progress.inputs_released =
                        self.combat.as_ref().unwrap().progress.inputs_released;
                    if let CombatSegmentEnd::Result(result) = end? {
                        ensure!(
                            result == od_core::battle_result::NormalBattleResult::Completed,
                            "锄大地战斗撤退"
                        );
                        self.combat = None;
                        self.progress.stage = "Move".into();
                        self.no_pos = None;
                        self.stuck = None;
                    }
                    continue;
                }
                let cancel = self.control.cancellation();
                let catalog = world_patrol::catalog().map_err(anyhow::Error::msg)?;
                let patrol = self.progress.world_patrol.as_ref().unwrap();
                let route = patrol.routes.get(patrol.route_index).cloned();
                if route.is_none() && !stage.starts_with("Leave") {
                    self.progress.stage = "Leave".into();
                    continue;
                }
                let area = route
                    .as_ref()
                    .and_then(|r| catalog.areas.iter().find(|a| a.id == r.area_id));
                if stage == "Move" {
                    let route = route.as_ref().context("锄大地缺少当前路线")?;
                    let area = area.unwrap();
                    if self.position.is_none() {
                        self.position = Some(catalog.start(route).map_err(anyhow::Error::msg)?);
                    }
                    let previous = self.position.unwrap();
                    let missing = self.no_pos.map_or(0.0, |at| at.elapsed().as_secs_f64());
                    let radius = ((missing + 1.0) * 50.0).min(800.0);
                    let Some(sight) = session.read_world_patrol(
                        Some(area),
                        Some(previous),
                        radius,
                        &root,
                        &cancel,
                    )?
                    else {
                        session.release_inputs()?;
                        continue;
                    };
                    if sight.mini_map.is_none() {
                        session.release_inputs()?;
                        if session.intel_battle_ready(
                            self.recognizer.as_ref().unwrap(),
                            &cancel,
                            remaining.min(Duration::from_secs(2)),
                        )? {
                            self.progress.stage = "Combat".into();
                        } else {
                            let at = self.no_pos.get_or_insert_with(Instant::now);
                            ensure!(
                                at.elapsed()
                                    < Duration::from_secs(u64::from(
                                        self.progress
                                            .world_patrol
                                            .as_ref()
                                            .unwrap()
                                            .config
                                            .ui_disappear_seconds
                                    )),
                                "锄大地界面消失，已停止输入"
                            );
                        }
                        continue;
                    }
                    let Some(position) = sight.position else {
                        session.release_inputs()?;
                        let since = self
                            .no_pos
                            .get_or_insert_with(Instant::now)
                            .elapsed()
                            .as_secs_f64();
                        let cfg = &self.progress.world_patrol.as_ref().unwrap().config;
                        let skip = self.progress.world_patrol.as_ref().unwrap().attempt > 0
                            && cfg.route_retry_action == PatrolRetryAction::SkipOnStuckAgain;
                        if since > 13.5 || (skip && since > 4.5) {
                            self.retry_route()?;
                        } else if since > 4.5
                            && self.unstuck < 3
                            && since > 4.5 + f64::from(self.unstuck) * 2.5
                        {
                            let key = if self.unstuck.is_multiple_of(2) {
                                65
                            } else {
                                68
                            };
                            session.patrol_key(key, &cancel)?;
                            self.unstuck += 1;
                        }
                        continue;
                    };
                    if position.distance(previous) > radius {
                        session.release_inputs()?;
                        self.no_pos.get_or_insert_with(Instant::now);
                        continue;
                    }
                    self.no_pos = None;
                    self.position = Some(position);
                    let patrol = self.progress.world_patrol.as_mut().unwrap();
                    if patrol.point_index >= route.points.len() {
                        session.release_inputs()?;
                        patrol.complete_route(true).map_err(anyhow::Error::msg)?;
                        self.progress.stage = "Inspect".into();
                        self.position = None;
                        self.stuck = None;
                        self.backtrack = None;
                        continue;
                    }
                    let target = route.points[patrol.point_index];
                    if position.distance(target) < 10.0 {
                        session.release_inputs()?;
                        patrol.point_index += 1;
                        self.stuck = None;
                        self.backtrack = None;
                        self.unstuck = 0;
                        continue;
                    }
                    if let Some((origin, at)) = self.stuck {
                        if position.distance(origin) >= 10.0 {
                            self.stuck = Some((position, Instant::now()));
                        } else if at.elapsed() > Duration::from_secs(2) {
                            session.release_inputs()?;
                            if patrol.attempt > 0
                                && patrol.config.route_retry_action
                                    == PatrolRetryAction::SkipOnStuckAgain
                            {
                                self.retry_route()?;
                                continue;
                            }
                            if self.backtrack.is_none() && self.unstuck == 0 {
                                let back = if patrol.point_index == 0 {
                                    catalog.start(route).map_err(anyhow::Error::msg)?
                                } else {
                                    route.points[patrol.point_index - 1]
                                };
                                self.backtrack = Some((back, Instant::now()));
                                self.unstuck = 1;
                            } else if self.backtrack.is_none() {
                                if self.unstuck >= 6 {
                                    self.retry_route()?;
                                    continue;
                                }
                                session.patrol_key(
                                    if self.unstuck.is_multiple_of(2) {
                                        65
                                    } else {
                                        68
                                    },
                                    &cancel,
                                )?;
                                self.unstuck += 1;
                            }
                            self.stuck = Some((position, Instant::now()));
                        }
                    } else {
                        self.stuck = Some((position, Instant::now()));
                    }
                    if let Some((back, at)) = self.backtrack
                        && (position.distance(back) < 10.0
                            || at.elapsed() > Duration::from_secs(15))
                    {
                        self.backtrack = None;
                    }
                    let target = self.backtrack.map_or(target, |(back, _)| back);
                    session.patrol_move_step(&sight, target, &cancel)?;
                    continue;
                }
                let Some(sight) = session.read_world_patrol(None, None, 0.0, &root, &cancel)?
                else {
                    session.release_inputs()?;
                    continue;
                };
                let world = sight.mini_map.is_some() || sight.normal_world;
                match stage.as_str() {
                    "Inspect" => {
                        if world {
                            if sight
                                .point("按自己的步调度过这一天", [0, 100, 380, 320])
                                .is_some()
                            {
                                self.progress.stage = "OpenMap".into();
                            } else {
                                session.patrol_key(27, &cancel)?;
                                self.progress.stage = "TrackSelect".into();
                            }
                        } else if sight.map_open {
                            self.progress.stage = "Area".into();
                        } else if sight.back {
                            session.patrol_key(27, &cancel)?;
                        }
                    }
                    "TrackSelect" => {
                        if let Some(p) = sight.point("绳网", [200, 924, 1728, 1058]) {
                            session.patrol_click(p, &cancel)?;
                            self.progress.stage = "TrackStop".into();
                        } else if world {
                            self.progress.stage = "Inspect".into();
                        }
                    }
                    "TrackStop" => {
                        if let Some(p) = sight.point("停止追踪", [410, 900, 546, 988]) {
                            session.patrol_click(p, &cancel)?;
                            self.progress.stage = "TrackReturn".into();
                        } else if sight.point("追踪", [410, 900, 546, 988]).is_some() {
                            self.progress.stage = "TrackReturn".into();
                        }
                    }
                    "TrackReturn" => {
                        if world {
                            self.progress.stage = "OpenMap".into();
                        } else if sight.back {
                            session.patrol_key(27, &cancel)?;
                        }
                    }
                    "OpenMap" => {
                        if sight.map_open {
                            self.progress.stage = "Area".into();
                        } else if world {
                            session.patrol_click([1639, 306], &cancel)?;
                        }
                    }
                    "Area" => {
                        let area = area.unwrap();
                        match session.patrol_area_step(
                            &sight,
                            area.parent_name.as_deref().unwrap_or(&area.name),
                            self.search_index,
                            cancel.clone(),
                        )? {
                            PatrolAreaStep::Selected => {
                                self.progress.stage = if area.parent_name.is_some() {
                                    "SubOpen"
                                } else {
                                    "Filter"
                                }
                                .into();
                            }
                            PatrolAreaStep::DetailsClosed | PatrolAreaStep::Scrolled => {
                                self.search_index += 1;
                            }
                            PatrolAreaStep::Waiting => {}
                        }
                    }
                    "SubOpen" => {
                        if sight.map_open {
                            session.patrol_click([214, 1021], &cancel)?;
                            self.progress.stage = "SubArea".into();
                        }
                    }
                    "SubArea" => {
                        if let Some(p) = sight.point(&area.unwrap().name, [66, 340, 406, 976]) {
                            session.patrol_click(p, &cancel)?;
                            self.progress.stage = "Filter".into();
                        }
                    }
                    "Filter" | "FilterOption" | "CloseFilter" => {
                        let phase = match stage.as_str() {
                            "Filter" => PatrolFilterPhase::Open,
                            "FilterOption" => PatrolFilterPhase::Option,
                            _ => PatrolFilterPhase::Close,
                        };
                        match session.patrol_filter_step(
                            &sight,
                            phase,
                            area.unwrap().hollow,
                            self.search_index,
                            cancel.clone(),
                        )? {
                            PatrolFilterStep::Opened => self.progress.stage = "FilterOption".into(),
                            PatrolFilterStep::Selected => {
                                self.progress.stage = "CloseFilter".into()
                            }
                            PatrolFilterStep::Closed => self.progress.stage = "Zoom".into(),
                            PatrolFilterStep::Opening | PatrolFilterStep::Closing => {
                                self.search_index += 1
                            }
                            PatrolFilterStep::Waiting => {}
                        }
                    }
                    "Zoom" => {
                        if sight.map_open {
                            session.patrol_drag([833, 1017], [533, 1017], cancel.clone())?;
                            self.progress.stage = "Search".into();
                        }
                    }
                    "Search" => {
                        if session.patrol_search_step(
                            &sight,
                            area.unwrap(),
                            &route.as_ref().unwrap().tp_name,
                            &mut self.search,
                            cancel.clone(),
                        )? == PatrolSearchStep::Found
                        {
                            self.progress.stage = "Confirm".into();
                        }
                    }
                    "Confirm" => {
                        let route = route.as_ref().unwrap();
                        if sight
                            .point(&route.tp_name, [1442, 118, 1794, 180])
                            .is_some()
                        {
                            if let Some(p) = sight.point("前往", [1542, 874, 1746, 930]) {
                                session.patrol_click(p, &cancel)?;
                                self.progress.stage = "Arrival".into();
                            }
                        } else {
                            self.progress.stage = "Search".into();
                        }
                    }
                    "Arrival" => {
                        let battle = !world
                            && session.intel_battle_ready(
                                self.recognizer.as_ref().unwrap(),
                                &cancel,
                                remaining.min(Duration::from_secs(2)),
                            )?;
                        if world || battle {
                            let route = route.as_ref().unwrap();
                            self.position = Some(catalog.start(route).map_err(anyhow::Error::msg)?);
                            self.progress.stage = if battle { "Combat" } else { "Move" }.into();
                            self.no_pos = None;
                            self.stuck = None;
                            self.unstuck = 0;
                        }
                    }
                    "Leave" => {
                        if sight.normal_world {
                            self.progress.stage = "LeaveArrival".into();
                        } else if world {
                            session.patrol_key(78, &cancel)?;
                            self.progress.stage = "LeaveArea".into();
                        }
                    }
                    "LeaveArea" => {
                        if sight.back
                            && let Some(p) = sight.point("录像店", [0, 90, 1920, 930])
                        {
                            session.patrol_click(p, &cancel)?;
                            self.progress.stage = "LeavePoint".into();
                        }
                    }
                    "LeavePoint" => {
                        if sight.back
                            && let Some(p) = sight.point("房间", [70, 1000, 1848, 1058])
                        {
                            session.patrol_click(p, &cancel)?;
                            self.progress.stage = "LeaveConfirm".into();
                        }
                    }
                    "LeaveConfirm" => {
                        if (sight
                            .point("是否传送至该地点？", [700, 480, 1220, 570])
                            .is_some()
                            || sight
                                .point("是否传送至该地点?", [700, 480, 1220, 570])
                                .is_some())
                            && let Some(p) = sight.point("确认", [1024, 600, 1232, 708])
                        {
                            session.patrol_click(p, &cancel)?;
                            self.progress.stage = "LeaveArrival".into();
                        }
                    }
                    "LeaveArrival" => {
                        if sight.normal_world {
                            let patrol = self.progress.world_patrol.as_mut().unwrap();
                            ensure!(
                                patrol.failed.is_empty(),
                                "锄大地部分路线失败，本轮不计入完成"
                            );
                            patrol.finish_round(used).map_err(anyhow::Error::msg)?;
                            self.progress.stage =
                                if patrol.completed() { "Done" } else { "Wait" }.into();
                        }
                    }
                    _ => bail!("未知锄大地阶段：{stage}"),
                }
                session.release_inputs()?;
                self.progress.inputs_released = true;
                let until = Instant::now();
                while until.elapsed() < Duration::from_millis(250)
                    && !cancel.load(Ordering::Acquire)
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        })();
        let release = slot.as_mut().map_or(Ok(()), |s| s.quiesce());
        self.progress.inputs_released = release.is_ok();
        self.progress.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Err(error) = result.and(release) {
            self.control.finish();
            self.progress.state = MailState::Failed;
            self.message = format!("锄大地失败：{error:#}");
            let _ = publish(self.progress.clone());
            return Err(error);
        }
        if !self.progress.state.is_terminal() && self.control.requested().is_some() {
            self.interrupted();
        }
        publish(self.progress.clone())?;
        Ok(())
    }
}
