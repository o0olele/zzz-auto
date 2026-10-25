//! 从当前区域地图追踪目标；沿引导点移动，遇战时交给同一运行槽的战斗执行器。
use crate::{
    combat::{CombatRun, CombatSegmentEnd},
    intel_board::{IntelSessionOptions, PreparedIntel},
};
use anyhow::{Context, Result, bail, ensure};
use maa_adapter::{
    GameSessionConfig, InferenceDevice, NativeSession, SessionHandoff,
    combat_vision::CombatRecognizer,
};
use od_core::{
    InputMode,
    basic_daily::DailyProgress,
    mail::MailState,
    map_exploration::{ExplorationTarget, MapSweep},
    pipeline::{Interrupt, PipelineControl, PipelineRequest},
};
use sha2::{Digest, Sha256};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub struct ExplorationRun {
    pub id: Uuid,
    pub progress: DailyProgress,
    pub message: String,
    prepared: PreparedIntel,
    control: PipelineControl,
    loaded: bool,
    following: bool,
    recognizer: Option<CombatRecognizer>,
    combat: Option<CombatRun>,
    tried: Vec<[i32; 2]>,
    stage_at: Instant,
    tracked_stage: String,
    target_at: Instant,
    lost_at: Option<Instant>,
    after_combat: String,
    sweep: MapSweep,
    zoom_stable: u8,
    zoom_attempts: u8,
    pan_inputs: usize,
    last_cargo_count: Option<u16>,
    cargo_total: Option<u16>,
    map_reset: bool,
    retrack_pending: bool,
    tracking_recovery: bool,
    tracking_recoveries: u8,
    road_started: Option<Instant>,
    road_progress: Option<(f64, Instant)>,
    road_blocked: bool,
    marker_started: Option<Instant>,
    marker_progress: Option<(f64, Instant)>,
    marker_blocked: bool,
    marker_turns: u8,
    guide_untrusted: bool,
    arrival_turns: u8,
    arrival_steps: u8,
    battle_frames: u8,
    day: od_core::daily::GameDay,
}
impl ExplorationRun {
    pub fn new(
        id: Uuid,
        progress: DailyProgress,
        prepared: PreparedIntel,
        day: od_core::daily::GameDay,
    ) -> Result<Self> {
        ensure!(
            progress.app_id == "map_exploration"
                && progress.app_combat.as_ref() == Some(&prepared.plan)
                && progress.exploration.is_some(),
            "空洞探索检查点或冻结战斗配置不符"
        );
        Ok(Self {
            id,
            progress,
            message: "空洞探索准备中".into(),
            prepared,
            control: PipelineControl::default(),
            loaded: false,
            following: false,
            recognizer: None,
            combat: None,
            tried: vec![],
            stage_at: Instant::now(),
            tracked_stage: String::new(),
            target_at: Instant::now(),
            lost_at: None,
            after_combat: "Follow".into(),
            sweep: MapSweep::default(),
            zoom_stable: 0,
            zoom_attempts: 0,
            pan_inputs: 0,
            last_cargo_count: None,
            cargo_total: None,
            map_reset: false,
            retrack_pending: false,
            tracking_recovery: false,
            tracking_recoveries: 0,
            road_started: None,
            road_progress: None,
            road_blocked: false,
            marker_started: None,
            marker_progress: None,
            marker_blocked: false,
            marker_turns: 0,
            guide_untrusted: false,
            arrival_turns: 0,
            arrival_steps: 0,
            battle_frames: 0,
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
            "空洞探索尚未安全暂停"
        );
        ensure!(self.control.resume(), "空洞探索已请求停止");
        self.progress.stage = if self.progress.exploration.as_ref().unwrap().scan_finished {
            "Leave"
        } else if self
            .progress
            .exploration
            .as_ref()
            .unwrap()
            .interaction_attempted
        {
            "Verify"
        } else {
            "Inspect"
        }
        .into();
        self.combat = None;
        self.following = false;
        self.tried.clear();
        self.tracked_stage.clear();
        self.target_at = Instant::now();
        self.lost_at = None;
        self.sweep = MapSweep::default();
        self.zoom_stable = 0;
        self.zoom_attempts = 0;
        self.pan_inputs = 0;
        self.map_reset = false;
        self.retrack_pending = false;
        self.tracking_recovery = false;
        self.tracking_recoveries = 0;
        self.guide_untrusted = false;
        self.reset_road_navigation();
        self.arrival_turns = 0;
        self.arrival_steps = 0;
        self.battle_frames = 0;
        self.progress.state = MailState::Running;
        Ok(())
    }
    fn prepare_scan(&mut self) {
        self.sweep = MapSweep::default();
        self.zoom_stable = 0;
        self.zoom_attempts = 0;
        self.pan_inputs = 0;
        self.map_reset = false;
        self.tried.clear();
        self.progress.stage = "Zoom".into();
    }
    fn reset_road_navigation(&mut self) {
        self.road_started = None;
        self.road_progress = None;
        self.road_blocked = false;
        self.reset_marker_navigation();
    }
    fn reset_marker_navigation(&mut self) {
        self.marker_started = None;
        self.marker_progress = None;
        self.marker_blocked = false;
        self.marker_turns = 0;
    }
    fn navigation_lost(
        &mut self,
        session: &mut NativeSession,
        sight: &maa_adapter::map_exploration::ExplorationSight,
    ) -> Result<()> {
        session.reset_patrol_steering();
        session.release_inputs()?;
        self.progress.inputs_released = true;
        let lost = self.lost_at.get_or_insert_with(Instant::now).elapsed();
        if lost >= Duration::from_secs(3)
            && self.tracking_recoveries < 2
            && sight.world_controls
            && sight.map_button.is_some()
        {
            self.tracking_recoveries += 1;
            self.tracking_recovery = true;
            self.retrack_pending = true;
            self.road_progress = None;
            self.road_blocked = false;
            self.reset_marker_navigation();
            self.lost_at = None;
            self.progress.stage = "RecoverTrack".into();
        } else {
            ensure!(
                lost < Duration::from_secs(8),
                "空洞探索重建追踪 {} 次后仍无法取得可用的引导线或小地图道路路径",
                self.tracking_recoveries
            );
        }
        Ok(())
    }
    fn trace_map(
        &self,
        sight: &maa_adapter::map_exploration::ExplorationSight,
        resources: &std::path::Path,
        phase: &str,
    ) {
        if self.progress.exploration.as_ref().unwrap().target != ExplorationTarget::Observation
            || self.sweep.index > 5
        {
            return;
        }
        let Some(frame) = &sight.map_frame else {
            return;
        };
        let Some(parent) = resources.parent() else {
            return;
        };
        let directory = parent
            .join("artifacts/map-exploration")
            .join(self.id.to_string());
        if std::fs::create_dir_all(&directory).is_ok() {
            let stem = format!("{phase}-{:02}", self.sweep.index);
            let _ = frame.save(directory.join(format!("{stem}.png")));
            let data = serde_json::json!({"view": self.sweep.index, "offset": self.sweep.offset,
                "icons": sight.icons, "zoom": sight.zoom, "cargo_count": sight.cargo_count});
            let _ = std::fs::write(directory.join(format!("{stem}.json")), data.to_string());
        }
    }
    fn interrupted(&mut self) -> bool {
        match self.control.requested() {
            Some(Interrupt::Stop) => {
                self.control.finish();
                self.progress.state = MailState::Stopped;
                self.message = "空洞探索已停止".into();
                true
            }
            Some(Interrupt::Pause) => {
                if !self.control.acknowledge_pause() {
                    return self.interrupted();
                }
                self.progress.state = MailState::Paused;
                self.message = "空洞探索已暂停；继续后重新读取地图与追踪状态".into();
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
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        // Follow 连续占用当前前台控制器；逐帧 handoff/quiesce 会松开 W。
        // 战斗、暂停恢复和其他阶段仍走完整交接与资源校验。
        if !battle
            && matches!(
                self.progress.stage.as_str(),
                "Follow" | "CrossZone" | "FollowMarker"
            )
            && self.following
            && self.loaded
            && slot.is_some()
        {
            let session = slot.as_mut().unwrap();
            session.check_combat_input_mode(InputMode::Foreground)?;
            return Ok(Some(session));
        }
        self.following = false;
        if options.fixture {
            slot.as_mut()
                .context("空洞探索回放控制器未准备")?
                .quiesce()?;
        } else {
            let config = &self.prepared.plan.config;
            match NativeSession::handoff_game(
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
            )? {
                SessionHandoff::Cancelled => return Ok(None),
                SessionHandoff::Created => self.loaded = false,
                _ => {}
            }
        }
        let session = slot.as_mut().context("空洞探索缺少控制器")?;
        session.check_combat_input_mode(if battle {
            self.prepared.plan.config.mode
        } else {
            InputMode::Foreground
        })?;
        if !self.loaded {
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
            let flash = self
                .prepared
                .script
                .states
                .iter()
                .any(|s| s == "闪避识别-红光" || s == "闪避识别-黄光");
            self.recognizer
                .as_mut()
                .unwrap()
                .warm_patrol(session, flash, &cancel)?;
            self.loaded = true;
        }
        self.following = !battle
            && matches!(
                self.progress.stage.as_str(),
                "Follow" | "CrossZone" | "FollowMarker"
            );
        Ok(Some(session))
    }
    pub fn execute(
        &mut self,
        slot: &mut Option<NativeSession>,
        options: IntelSessionOptions<'_>,
        mut publish: impl FnMut(DailyProgress) -> Result<()>,
    ) -> Result<()> {
        let started = Instant::now();
        let initial = self.progress.active_ms;
        let result = (|| -> Result<()> {
            if let Some(session) = slot.as_mut() {
                session.quiesce()?;
            }
            self.loaded = false;
            self.following = false;
            self.progress.inputs_released = true;
            if self.interrupted() {
                return Ok(());
            }
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
                    "空洞探索已跨游戏日，请重新预览"
                );
                self.progress.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                ensure!(
                    self.progress.active_ms < u64::from(self.progress.timeout_ms()),
                    "空洞探索达到总超时"
                );
                if self.tracked_stage != self.progress.stage {
                    self.tracked_stage = self.progress.stage.clone();
                    self.stage_at = Instant::now();
                }
                ensure!(
                    matches!(
                        self.progress.stage.as_str(),
                        "Follow" | "CrossZone" | "FollowMarker" | "Combat" | "Pan" | "Scan"
                    ) || self.stage_at.elapsed()
                        < Duration::from_secs(if self.progress.stage == "Transport" {
                            45
                        } else {
                            20
                        }),
                    "空洞探索界面步骤超时：{}",
                    self.progress.stage
                );
                publish(self.progress.clone())?;
                let stage = self.progress.stage.clone();
                ensure!(
                    !matches!(stage.as_str(), "Follow" | "CrossZone" | "FollowMarker")
                        || self.target_at.elapsed() < Duration::from_secs(180),
                    "空洞探索当前目标长时间未完成，已停止移动"
                );
                let cancel = self.control.cancellation();
                let Some(session) = self.session(slot, &options, stage == "Combat")? else {
                    self.interrupted();
                    return Ok(());
                };
                if stage == "Combat" {
                    if self.combat.is_none() {
                        let mut run = CombatRun::new_segment(
                            self.id,
                            self.prepared.plan.config.clone(),
                            self.prepared.script.clone(),
                            self.control.clone(),
                        )?
                        .for_world_patrol(10);
                        run.progress.script_revision = self.prepared.plan.script_revision.clone();
                        self.combat = Some(run);
                    }
                    let mut checkpoint = self.progress.clone();
                    let remaining = Duration::from_millis(
                        u64::from(self.progress.timeout_ms()) - self.progress.active_ms,
                    );
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
                    let end = end?;
                    if let CombatSegmentEnd::Result(result) = end {
                        ensure!(
                            result == od_core::battle_result::NormalBattleResult::Completed,
                            "空洞探索战斗撤退"
                        );
                        self.combat = None;
                        self.progress.stage = self.after_combat.clone();
                        self.target_at = Instant::now();
                        self.lost_at = None;
                        self.reset_road_navigation();
                    }
                    continue;
                }
                let target = self.progress.exploration.as_ref().unwrap().target;
                let Some(sight) = session.read_map_exploration_with_guide_policy(
                    target,
                    !self.guide_untrusted,
                    &cancel,
                )?
                else {
                    self.battle_frames = 0;
                    session.reset_patrol_steering();
                    session.release_inputs()?;
                    self.progress.inputs_released = true;
                    continue;
                };
                self.guide_untrusted |= sight.guide_ambiguous;
                if !sight.fresh() {
                    self.battle_frames = 0;
                    session.reset_patrol_steering();
                    session.release_inputs()?;
                    self.progress.inputs_released = true;
                    continue;
                }
                // 攻击按钮在探索和地图开合过渡帧中也会出现。先排除明确的
                // 非战斗界面，再连续确认两帧；确认期间不发送导航或交互输入。
                if !sight.non_combat_ui()
                    && session
                        .exploration_battle_ready(self.recognizer.as_mut().unwrap(), &cancel)?
                {
                    session.release_inputs()?;
                    self.progress.inputs_released = true;
                    self.battle_frames += 1;
                    if self.battle_frames >= 2 {
                        self.battle_frames = 0;
                        session.reset_patrol_steering();
                        self.after_combat = if self
                            .progress
                            .exploration
                            .as_ref()
                            .unwrap()
                            .interaction_attempted
                        {
                            "Verify".into()
                        } else if matches!(stage.as_str(), "Follow" | "CrossZone" | "FollowMarker")
                        {
                            "Follow".into()
                        } else {
                            "Inspect".into()
                        };
                        self.progress.stage = "Combat".into();
                    }
                    continue;
                }
                self.battle_frames = 0;
                if !sight.fresh() {
                    session.release_inputs()?;
                    continue;
                }
                if let Some([current, total]) = sight.cargo_count {
                    self.cargo_total = Some(total);
                    let checkpoint = self.progress.exploration.as_mut().unwrap();
                    let mut confirmed_count = false;
                    if checkpoint.interaction_attempted
                        && checkpoint.target == ExplorationTarget::Cargo
                        && self.last_cargo_count.is_some_and(|before| current > before)
                    {
                        checkpoint.confirm().map_err(anyhow::Error::msg)?;
                        confirmed_count = true;
                    }
                    if !checkpoint.interaction_attempted {
                        self.last_cargo_count = Some(current);
                    }
                    if current == total && !checkpoint.scan_finished {
                        checkpoint.target = ExplorationTarget::Cargo;
                        checkpoint.interaction_attempted = false;
                        checkpoint.scan_finished = true;
                        self.progress.stage = "Leave".into();
                        continue;
                    }
                    if confirmed_count {
                        self.tried.clear();
                        self.progress.stage = "Inspect".into();
                        continue;
                    }
                }
                self.progress.inputs_released = false;
                match stage.as_str() {
                    "Inspect" => {
                        if let Some(cancel_button) = sight.nearby_transport_button(false) {
                            // 新运行不能继承上次未确认的目标，先返回地图重新核对。
                            session.patrol_click(cancel_button, &cancel)?;
                            self.retrack_pending = true;
                        } else if sight.map_open {
                            self.prepare_scan();
                        } else if sight.navigation_overview {
                            session.patrol_key(27, &cancel)?;
                        } else if let Some(button) = sight.map_button {
                            session.patrol_click(button, &cancel)?;
                        }
                    }
                    "RecoverTrack" => {
                        if sight.navigation_overview {
                            session.patrol_key(27, &cancel)?;
                        } else if sight.map_open {
                            if sight.selected(target) {
                                self.progress.stage = "Detail".into();
                            } else if let Some(close) = sight.close_details {
                                session.patrol_click(close, &cancel)?;
                            } else {
                                let mut highlighted =
                                    sight.icons.iter().filter(|icon| icon.highlighted);
                                if let Some(icon) = highlighted.next()
                                    && highlighted.next().is_none()
                                {
                                    session.patrol_click(icon.center, &cancel)?;
                                    self.progress.stage = "Detail".into();
                                } else {
                                    // 没有唯一追踪图标时，重新缩放拖图核对目标名称。
                                    self.prepare_scan();
                                }
                            }
                        } else if let Some(button) = sight.map_button {
                            session.patrol_click(button, &cancel)?;
                        }
                    }
                    "Zoom" => {
                        ensure!(sight.map_open, "空洞探索缩放时区域地图已关闭");
                        if let Some(close) = sight.close_details {
                            session.patrol_click(close, &cancel)?;
                        } else if !self.map_reset
                            && let Some(zoom) = sight.zoom
                        {
                            // 每轮从重置视角开始，避免上轮拖图造成扫描原点漂移。
                            session.patrol_click([1825, zoom.knob[1]], &cancel)?;
                            self.map_reset = true;
                        } else if let Some(zoom) = sight.zoom {
                            if zoom.at_minimum {
                                self.zoom_stable += 1;
                                if self.zoom_stable >= 2 {
                                    if !options.fixture {
                                        self.trace_map(&sight, options.resources, "minimum");
                                    }
                                    self.progress.stage = "Pan".into();
                                }
                            } else {
                                ensure!(self.zoom_attempts < 3, "空洞探索无法将地图缩放到最小");
                                self.zoom_attempts += 1;
                                self.zoom_stable = 0;
                                session.patrol_drag(zoom.knob, zoom.minimum, cancel.clone())?;
                            }
                        }
                    }
                    "Pan" => {
                        ensure!(sight.map_open, "空洞探索拖图时区域地图已关闭");
                        if let Some(close) = sight.close_details {
                            session.patrol_click(close, &cancel)?;
                        } else if !sight.zoom.is_some_and(|zoom| zoom.at_minimum) {
                            self.prepare_scan();
                        } else if let Some((start, end)) = self.sweep.drag() {
                            ensure!(self.pan_inputs < 256, "空洞探索拖图达到本轮上限");
                            session.patrol_drag(start, end, cancel.clone())?;
                            if !cancel.load(Ordering::Acquire) {
                                self.sweep.applied(start, end);
                                self.pan_inputs += 1;
                                self.tried.clear();
                            }
                        } else {
                            self.progress.stage = "Scan".into();
                        }
                    }
                    "Scan" => {
                        if !options.fixture {
                            self.trace_map(&sight, options.resources, "scan");
                        }
                        ensure!(sight.map_open, "空洞探索大地图已关闭，请重新运行");
                        if !sight.zoom.is_some_and(|zoom| zoom.at_minimum) {
                            self.prepare_scan();
                        } else if let Some(close) = sight.close_details {
                            session.patrol_click(close, &cancel)?;
                        } else if let Some(icon) = sight.icons.iter().find(|i| {
                            !self.tried.iter().any(|p| {
                                (p[0] - i.center[0]).pow(2) + (p[1] - i.center[1]).pow(2) < 32 * 32
                            })
                        }) {
                            if !self.tracking_recovery {
                                self.tracking_recoveries = 0;
                                self.guide_untrusted = false;
                                self.reset_road_navigation();
                                self.arrival_turns = 0;
                                self.arrival_steps = 0;
                            }
                            self.tried.push(icon.center);
                            session.patrol_click(icon.center, &cancel)?;
                            self.progress.stage = "Detail".into();
                        } else {
                            self.tried.clear();
                            if self.sweep.advance() {
                                self.progress.stage = "Pan".into();
                            } else if target == ExplorationTarget::Observation {
                                self.progress.exploration.as_mut().unwrap().exhausted();
                                self.sweep.restart();
                                self.progress.stage = "Pan".into();
                            } else {
                                bail!(
                                    "最小缩放下已拖图扫描 {} 屏，卡格车进度 {}/{}，尚未证明全部收集",
                                    self.sweep.total(),
                                    self.last_cargo_count
                                        .map_or("未知".into(), |n| n.to_string()),
                                    self.cargo_total.map_or("未知".into(), |n| n.to_string())
                                );
                            }
                        }
                    }
                    "Detail" => {
                        if sight.selected(target) {
                            if sight.unlocked() {
                                self.progress.stage = "Scan".into();
                            } else if sight.tracking() {
                                if self.retrack_pending {
                                    // 核对候选名称后，先取消旧追踪，再重新追踪。
                                    let button =
                                        sight.stop_tracking_button().context("缺少停止追踪按钮")?;
                                    session.patrol_click(button, &cancel)?;
                                    self.retrack_pending = false;
                                } else {
                                    self.progress.stage = "CloseMap".into();
                                }
                            } else if let Some(button) = sight.track_button() {
                                self.retrack_pending = false;
                                session.patrol_click(button, &cancel)?;
                                self.progress.stage = "Track".into();
                            }
                        } else if sight.close_details.is_some() {
                            self.progress.stage = "Scan".into();
                        }
                    }
                    "Track" => {
                        if self.tracking_recovery
                            && let Some(cancel_button) = sight.nearby_transport_button(false)
                        {
                            // 已开始追踪；只取消附近传送，保留战斗后的当前位置。
                            session.patrol_click(cancel_button, &cancel)?;
                            self.progress.stage = "CloseMap".into();
                        } else if let Some(confirm_button) = sight.nearby_transport_button(true) {
                            session.patrol_click(confirm_button, &cancel)?;
                            self.progress.stage = "Transport".into();
                        } else if sight.selected(target) && sight.tracking() {
                            self.progress.stage = "CloseMap".into();
                        } else if sight.mini_map {
                            self.tracking_recovery = false;
                            self.target_at = Instant::now();
                            self.lost_at = None;
                            session.reset_patrol_steering();
                            self.progress.stage = "Follow".into();
                        }
                    }
                    "Transport" => {
                        if sight.mini_map {
                            self.target_at = Instant::now();
                            self.lost_at = None;
                            session.reset_patrol_steering();
                            self.progress.stage = "Follow".into();
                        }
                    }
                    "CloseMap" => {
                        if sight.map_open {
                            session.patrol_key(27, &cancel)?;
                        } else if sight.mini_map {
                            self.tracking_recovery = false;
                            self.target_at = Instant::now();
                            self.lost_at = None;
                            session.reset_patrol_steering();
                            self.progress.stage = "Follow".into();
                        }
                    }
                    "ApproachTarget" => {
                        if sight.interact(target) {
                            session.release_inputs()?;
                            self.progress
                                .exploration
                                .as_mut()
                                .unwrap()
                                .interaction_attempted = true;
                            self.progress.stage = "AwaitUnlock".into();
                            self.progress.inputs_released = true;
                            publish(self.progress.clone())?;
                            if sight.fresh() {
                                session.patrol_key(70, &cancel)?;
                            } else {
                                self.progress
                                    .exploration
                                    .as_mut()
                                    .unwrap()
                                    .interaction_attempted = false;
                                self.progress.stage = "ApproachTarget".into();
                            }
                        } else {
                            ensure!(
                                self.arrival_turns < 8 && self.arrival_steps < 3,
                                "已到目标附近并调整人物朝向，但仍未出现该目标的 F 交互提示"
                            );
                            match session.exploration_face_target(&sight, target, &cancel)? {
                                maa_adapter::map_exploration::ArrivalStep::Turned => {
                                    self.arrival_turns += 1
                                }
                                maa_adapter::map_exploration::ArrivalStep::Faced => {
                                    self.arrival_steps += 1
                                }
                                maa_adapter::map_exploration::ArrivalStep::Waiting => {}
                            }
                        }
                    }
                    "Follow" | "CrossZone" | "FollowMarker" | "AwaitUnlock" => {
                        ensure!(
                            self.target_at.elapsed() < Duration::from_secs(180),
                            "空洞探索当前目标长时间未完成，已停止移动"
                        );
                        let attempted = self
                            .progress
                            .exploration
                            .as_ref()
                            .unwrap()
                            .interaction_attempted;
                        if attempted && sight.reward(target) {
                            self.progress
                                .exploration
                                .as_mut()
                                .unwrap()
                                .confirm()
                                .map_err(anyhow::Error::msg)?;
                            self.tried.clear();
                            self.progress.stage = "Inspect".into();
                        } else if attempted {
                            session.release_inputs()?;
                            if self.stage_at.elapsed() > Duration::from_secs(5) {
                                self.progress.stage = "Verify".into();
                            }
                        } else if sight.interact(target) {
                            session.release_inputs()?;
                            self.progress
                                .exploration
                                .as_mut()
                                .unwrap()
                                .interaction_attempted = true;
                            self.progress.stage = "AwaitUnlock".into();
                            self.progress.inputs_released = true;
                            publish(self.progress.clone())?;
                            if sight.fresh() {
                                session.patrol_key(70, &cancel)?;
                            } else {
                                self.progress
                                    .exploration
                                    .as_mut()
                                    .unwrap()
                                    .interaction_attempted = false;
                                self.progress.stage = "Follow".into();
                            }
                        } else if sight.arrival_point(target).is_some() {
                            session.release_inputs()?;
                            session.reset_patrol_steering();
                            self.arrival_turns = 0;
                            self.arrival_steps = 0;
                            self.progress.stage = "ApproachTarget".into();
                        } else if sight.guide.is_some() && sight.heading.is_some() {
                            self.lost_at = None;
                            self.reset_road_navigation();
                            self.progress.stage = "Follow".into();
                            session.exploration_walk(&sight, &cancel)?;
                        } else if let Some(route) = &sight.road_route
                            && sight.heading.is_some()
                        {
                            self.reset_marker_navigation();
                            ensure!(
                                self.road_started.get_or_insert_with(Instant::now).elapsed()
                                    < Duration::from_secs(60),
                                "无引导线的小地图道路导航达到 60 秒，已停止移动"
                            );
                            if !session.patrol_is_walking() && !self.road_blocked {
                                self.road_progress = Some((route.remaining, Instant::now()));
                            } else if let Some((best, at)) = self.road_progress {
                                if route.remaining < best - 2.0 {
                                    self.road_progress = Some((route.remaining, Instant::now()));
                                } else if at.elapsed() > Duration::from_secs(4) {
                                    self.road_blocked = true;
                                }
                            }
                            if self.road_blocked {
                                self.navigation_lost(session, &sight)?;
                            } else {
                                self.lost_at = None;
                                self.progress.stage = "CrossZone".into();
                                session.exploration_walk(&sight, &cancel)?;
                            }
                        } else if let Some(marker) = sight.world_marker {
                            let elapsed = self
                                .marker_started
                                .get_or_insert_with(Instant::now)
                                .elapsed();
                            let (best, at) = *self
                                .marker_progress
                                .get_or_insert((marker.meters, Instant::now()));
                            if marker.meters < best - 0.5 {
                                self.marker_progress = Some((marker.meters, Instant::now()));
                            } else if at.elapsed() > Duration::from_secs(4) {
                                self.marker_blocked = true;
                            }
                            self.marker_blocked |=
                                elapsed > Duration::from_secs(30) || self.marker_turns >= 8;
                            if self.marker_blocked {
                                self.navigation_lost(session, &sight)?;
                            } else {
                                self.lost_at = None;
                                self.progress.stage = "FollowMarker".into();
                                if session.exploration_walk_marker(&sight, &cancel)? {
                                    self.marker_turns = 0;
                                } else {
                                    self.marker_turns += 1;
                                }
                            }
                        } else {
                            self.navigation_lost(session, &sight)?;
                        }
                    }
                    "Verify" => {
                        if sight.navigation_overview {
                            session.patrol_key(27, &cancel)?;
                        } else if !sight.map_open
                            && let Some(button) = sight.map_button
                        {
                            session.patrol_click(button, &cancel)?;
                        } else if sight.map_open
                            && target == ExplorationTarget::Cargo
                            && sight.close_details.is_some()
                        {
                            session.patrol_click(sight.close_details.unwrap(), &cancel)?;
                        } else if sight.map_open && sight.selected(target) && sight.unlocked() {
                            self.progress
                                .exploration
                                .as_mut()
                                .unwrap()
                                .confirm()
                                .map_err(anyhow::Error::msg)?;
                            self.tried.clear();
                            self.prepare_scan();
                        } else if sight.map_open
                            && !sight.selected(target)
                            && target == ExplorationTarget::Observation
                        {
                            // 地图重开后详情可能收起，只重选唯一的黄色追踪奇点。
                            let mut highlighted =
                                sight.icons.iter().filter(|icon| icon.highlighted);
                            if let Some(icon) = highlighted.next()
                                && highlighted.next().is_none()
                            {
                                session.patrol_click(icon.center, &cancel)?;
                            }
                        }
                    }
                    "Leave" => {
                        if sight.map_open {
                            session.patrol_key(27, &cancel)?;
                        } else if sight.mini_map {
                            session.release_inputs()?;
                            self.progress.inputs_released = true;
                            self.progress.completion_proven = true;
                            self.progress.returned_world = true;
                            self.progress.stage = "Done".into();
                            ensure!(self.progress.completed(), "空洞探索完成证明不足");
                            self.progress.state = MailState::Succeeded;
                            if self.control.finish() == Some(Interrupt::Stop) {
                                self.progress.state = MailState::Stopped;
                                self.message = "空洞探索已停止".into();
                                return Ok(());
                            }
                            let p = self.progress.exploration.as_ref().unwrap();
                            self.message = format!(
                                "本区域小卡格车已全部解锁（{0}/{0}）；本次解锁 {1} 个观测奇点，开启 {2} 辆小卡格车",
                                self.cargo_total.unwrap_or(0),
                                p.observations,
                                p.cargo
                            );
                            return Ok(());
                        }
                    }
                    _ => bail!("未知空洞探索阶段：{stage}"),
                }
                if matches!(
                    self.progress.stage.as_str(),
                    "Follow" | "CrossZone" | "FollowMarker"
                ) && session.patrol_is_walking()
                {
                    self.progress.inputs_released = false;
                } else {
                    session.release_inputs()?;
                    self.progress.inputs_released = true;
                }
                let wait = Instant::now();
                while wait.elapsed() < Duration::from_millis(180) && !cancel.load(Ordering::Acquire)
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
            self.message = format!("空洞探索失败：{error:#}");
            return Err(error);
        }
        if !self.progress.state.is_terminal() && self.control.requested().is_some() {
            self.interrupted();
        }
        // 终态由分项包装器连同 message 保存一次；提前用通用消息发布会丢失失败原因。
        Ok(())
    }
}
