//! 使用真实 Maa 队列及模拟控制器，验证地图入口和终态保存，不连接游戏。
use maa_adapter::{CustomControllerCallback, InputMode, NativeSession};
use od_core::{basic_daily::DailyProgress, mail::MailState};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use zzz_engine::{
    intel_board::{IntelSessionOptions, PreparedIntel},
    map_exploration::ExplorationRun,
};

struct MapController {
    recover: bool,
    recover_navigation: bool,
    transition: bool,
    transition_captures: AtomicUsize,
    page: Arc<AtomicUsize>,
    keys: Arc<Mutex<Vec<i32>>>,
    key_events: Arc<Mutex<Vec<(i32, bool)>>>,
    attacks: Arc<AtomicUsize>,
    clicks: Arc<Mutex<Vec<[i32; 2]>>>,
    swipes: Arc<Mutex<Vec<[i32; 4]>>>,
}

struct SlowCleanupController {
    cleanups: Arc<AtomicUsize>,
    keys: Arc<Mutex<Vec<(i32, bool)>>>,
}
impl CustomControllerCallback for SlowCleanupController {
    fn connect(&self) -> bool {
        true
    }
    fn request_uuid(&self) -> Option<String> {
        Some("slow-win32-cleanup-regression".into())
    }
    fn screencap(&self) -> Option<Vec<u8>> {
        Some(
            include_bytes!("../../maa-adapter/tests/fixtures/map-exploration/follow-world.png")
                .to_vec(),
        )
    }
    fn key_down(&self, key: i32) -> bool {
        self.keys.lock().unwrap().push((key, true));
        true
    }
    fn key_up(&self, key: i32) -> bool {
        self.keys.lock().unwrap().push((key, false));
        true
    }
    fn inactive(&self) -> bool {
        self.cleanups.fetch_add(1, Ordering::AcqRel);
        std::thread::sleep(std::time::Duration::from_millis(40));
        true
    }
}

#[test]
#[ignore = "需要固定 Maa SDK 和生成后的 world-patrol 资源"]
fn read_only_recognition_does_not_inactivate_the_game_controller() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/exploration-runtime-logs"),
    )?;
    let cleanups = Arc::new(AtomicUsize::new(0));
    let keys = Arc::new(Mutex::new(vec![]));
    let mut native = NativeSession::for_custom_controller(
        SlowCleanupController {
            cleanups: cleanups.clone(),
            keys: keys.clone(),
        },
        &root.join("resources/generated/world-patrol"),
        InputMode::Foreground,
    )?;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let image = maa_adapter::RecognitionImage::load(
        &root.join("crates/maa-adapter/tests/fixtures/map-exploration/follow-world.png"),
    )?;
    native.combat_input(od_core::InputProbeAction::Key { key: 87 }, true, &cancel)?;
    let before = cleanups.load(Ordering::Acquire);
    for _ in 0..8 {
        let result = native.recognize_prepared(&image, "TemplateMatch", &serde_json::json!({
            "template":"world-patrol/normal_world_investigation/function_menu.png", "roi":[1755,20,150,145],
            "threshold":0.8,"method":5,"green_mask":true
        }))?;
        assert_eq!(result.detail["hit"], true);
    }
    assert_eq!(
        cleanups.load(Ordering::Acquire),
        before,
        "只读识别不能逐次恢复真实窗口/输入"
    );
    assert_eq!(*keys.lock().unwrap(), vec![(87, true)]);
    assert!(native.patrol_is_walking());
    native.quiesce()?;
    assert!(
        cleanups.load(Ordering::Acquire) > before,
        "交接仍须收尾真实控制器"
    );
    assert_eq!(*keys.lock().unwrap(), vec![(87, true), (87, false)]);
    Ok(())
}
impl CustomControllerCallback for MapController {
    fn connect(&self) -> bool {
        true
    }
    fn request_uuid(&self) -> Option<String> {
        Some("exploration-map-entry-regression".into())
    }
    fn screencap(&self) -> Option<Vec<u8>> {
        if self.page.load(Ordering::Acquire) == 11
            && self.transition_captures.fetch_add(1, Ordering::AcqRel) >= 2
        {
            self.page.store(2, Ordering::Release);
        }
        let bytes = match self.page.load(Ordering::Acquire) {
            0 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/navigation-overview.png"
            )
            .as_slice(),
            1 | 12 => {
                include_bytes!("../../maa-adapter/tests/fixtures/map-exploration/world-hud.png")
                    .as_slice()
            }
            11 | 13 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/map-opening-transition.png"
            )
            .as_slice(),
            2 => include_bytes!("../../maa-adapter/tests/fixtures/map-exploration/zoomed-map.png")
                .as_slice(),
            3 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/reset-minimum-map.png"
            )
            .as_slice(),
            5 | 7 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/nearby-transport.png"
            )
            .as_slice(),
            6 => {
                include_bytes!("../../maa-adapter/tests/fixtures/map-exploration/follow-world.png")
                    .as_slice()
            }
            14 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/world-without-player.png"
            )
            .as_slice(),
            15 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/post-battle-no-target.png"
            )
            .as_slice(),
            17 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/road-aligned-world.png"
            )
            .as_slice(),
            18 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/observation-arrival-edge.png"
            )
            .as_slice(),
            19 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/observation-arrival-facing.png"
            )
            .as_slice(),
            20 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/observation-arrival-f-ready.png"
            )
            .as_slice(),
            21 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/post-battle-no-target.png"
            )
            .as_slice(),
            22 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/map-observation-unlocked.png"
            )
            .as_slice(),
            23 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/minimap-ambiguous-route.png"
            )
            .as_slice(),
            24 => include_bytes!(
                "../../maa-adapter/tests/fixtures/map-exploration/minimap-ambiguous-centered.png"
            )
            .as_slice(),
            8 | 9 | 16 => {
                include_bytes!("../../maa-adapter/tests/fixtures/map-exploration/map.png")
                    .as_slice()
            }
            10 => {
                include_bytes!("../../maa-adapter/tests/fixtures/map-exploration/map-untracked.png")
                    .as_slice()
            }
            _ => {
                include_bytes!("../../maa-adapter/tests/fixtures/map-exploration/live-panned-0.png")
                    .as_slice()
            }
        };
        Some(bytes.to_vec())
    }
    fn key_down(&self, key: i32) -> bool {
        self.keys.lock().unwrap().push(key);
        self.key_events.lock().unwrap().push((key, true));
        if key == 27 && self.page.load(Ordering::Acquire) == 0 {
            self.page.store(1, Ordering::Release);
        } else if key == 27 && self.page.load(Ordering::Acquire) == 16 {
            self.page.store(6, Ordering::Release);
        } else if key == 87 && self.page.load(Ordering::Acquire) == 19 {
            self.page.store(20, Ordering::Release);
        } else if key == 70 && self.page.load(Ordering::Acquire) == 20 {
            self.page.store(21, Ordering::Release);
        }
        key != 78
    }
    fn key_up(&self, key: i32) -> bool {
        self.key_events.lock().unwrap().push((key, false));
        true
    }
    fn relative_move(&self, _: i32, _: i32) -> bool {
        if self.page.load(Ordering::Acquire) == 18 {
            self.page.store(19, Ordering::Release);
            true
        } else if self.page.load(Ordering::Acquire) == 23 {
            self.page.store(24, Ordering::Release);
            true
        } else {
            false
        }
    }
    fn touch_down(&self, contact: i32, x: i32, y: i32, _: i32) -> bool {
        if contact == 0 && self.page.load(Ordering::Acquire) == 13 {
            self.attacks.fetch_add(1, Ordering::AcqRel);
        }
        self.clicks.lock().unwrap().push([x, y]);
        match self.page.load(Ordering::Acquire) {
            21 if (1600..1700).contains(&x) && (260..350).contains(&y) => {
                self.page.store(22, Ordering::Release);
            }
            15 if (1600..1700).contains(&x) && (260..350).contains(&y) => {
                self.page.store(9, Ordering::Release);
            }
            5 if self.recover_navigation && (650..950).contains(&x) && (570..710).contains(&y) => {
                self.page.store(16, Ordering::Release);
            }
            5 if (980..1280).contains(&x) && (570..710).contains(&y) => {
                self.page.store(6, Ordering::Release);
            }
            7 if (650..950).contains(&x) && (570..710).contains(&y) => {
                self.page.store(8, Ordering::Release);
            }
            8 if (1760..1870).contains(&x) && (100..220).contains(&y) => {
                self.page.store(2, Ordering::Release);
            }
            4 if self.recover && (900..980).contains(&x) && (410..480).contains(&y) => {
                self.page.store(9, Ordering::Release);
            }
            9 if (1400..1830).contains(&x) && (850..990).contains(&y) => {
                self.page.store(10, Ordering::Release);
            }
            10 if (1400..1830).contains(&x) && (850..990).contains(&y) => {
                self.page.store(5, Ordering::Release);
            }
            _ => {}
        }
        if (1600..1700).contains(&x)
            && (260..350).contains(&y)
            && matches!(self.page.load(Ordering::Acquire), 1 | 12)
        {
            self.page
                .store(if self.transition { 11 } else { 2 }, Ordering::Release);
        }
        true
    }
    fn touch_up(&self, _: i32) -> bool {
        true
    }
    fn swipe(&self, x1: i32, y1: i32, x2: i32, y2: i32, _: i32) -> bool {
        self.swipes.lock().unwrap().push([x1, y1, x2, y2]);
        let page = self.page.load(Ordering::Acquire);
        if page == 2 && y1 > 1000 && x2 == 834 {
            self.page.store(3, Ordering::Release);
        } else if page == 3 && [x1, y1, x2, y2] == [720, 540, 200, 540] {
            self.page.store(4, Ordering::Release);
        }
        true
    }
}

#[test]
#[ignore = "需要固定 Maa SDK、战斗模型和生成后的 world-patrol 资源"]
fn ambiguous_minimap_uses_world_marker_and_releases_when_stuck() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/exploration-runtime-logs"),
    )?;
    let combat_root = root.join("resources/generated/combat");
    let data = root.join("artifacts/exploration-runtime-scripts");
    let bundle = maa_adapter::combat_vision::CombatBundle::load(&combat_root)?;
    let revision = zzz_engine::combat::script_catalog(&bundle, &data)?.revision;
    let mut config = od_core::combat::CombatConfig::default();
    config.start_delay_ms = 0;
    config.dodge_only = false;
    let frozen = od_core::app_combat::AppCombatPlan {
        config,
        script_revision: revision,
    };
    let prepared = PreparedIntel::load(frozen.clone(), &combat_root, &data)?;
    let mut plan = od_core::run_plan::plan_standalone_at(
        &od_core::config::AccountConfig::default(),
        1,
        "one_dragon",
        "map_exploration",
        chrono::Utc::now(),
        None,
    )
    .map_err(anyhow::Error::msg)?;
    plan.exploration_combat = Some(frozen);
    let mut slot = od_core::RunSlot::default();
    let record = slot
        .begin_group_plan(plan.mail_config, &plan)
        .map_err(anyhow::Error::msg)?;
    let mut run = ExplorationRun::new(record.id, record.daily[0].clone(), prepared, plan.game_day)?;
    run.progress.stage = "Follow".into();
    let control = run.control();
    let page = Arc::new(AtomicUsize::new(23));
    let events = Arc::new(Mutex::new(vec![]));
    let mut native = Some(NativeSession::for_custom_controller(
        MapController {
            recover: false,
            recover_navigation: false,
            transition: false,
            transition_captures: AtomicUsize::new(0),
            page: page.clone(),
            keys: Arc::default(),
            key_events: events.clone(),
            attacks: Arc::default(),
            clicks: Arc::default(),
            swipes: Arc::default(),
        },
        &root.join("resources/generated/world-patrol"),
        InputMode::Foreground,
    )?);
    let window = od_core::WindowInfo {
        hwnd: "fixture".into(),
        pid: 0,
        title: "小地图异常世界标记回归".into(),
        executable: String::new(),
        width: 1920,
        height: 1080,
        keyboard_permission: None,
    };
    let mut moving_frames = 0;
    let mut released = false;
    run.execute(
        &mut native,
        IntelSessionOptions {
            window: &window,
            capture: Default::default(),
            resources: &root.join("resources"),
            fixture: true,
        },
        |progress| {
            assert!(
                progress.active_ms < 25_000,
                "世界标记距离不变时必须有界停止"
            );
            assert_ne!(progress.stage, "Combat");
            if progress.stage == "FollowMarker" && !progress.inputs_released {
                moving_frames += 1;
                let w: Vec<_> = events
                    .lock()
                    .unwrap()
                    .iter()
                    .copied()
                    .filter(|e| e.0 == 87)
                    .collect();
                assert_eq!(w, vec![(87, true)], "对准世界目标后应持续按住 W");
                assert_eq!(page.load(Ordering::Acquire), 24, "先调整镜头再前进");
            }
            if moving_frames >= 3 && progress.inputs_released {
                released = true;
                assert!(control.stop());
            }
            Ok(())
        },
    )?;
    assert!(
        moving_frames >= 3 && released,
        "必须验证连续行走后因距离无进展松键"
    );
    assert!(run.progress.inputs_released);
    let w: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .copied()
        .filter(|e| e.0 == 87)
        .collect();
    assert_eq!(w, vec![(87, true), (87, false)]);
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、战斗模型和生成后的 world-patrol 资源"]
fn arrives_faces_then_interacts_and_verifies_unlock() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/exploration-runtime-logs"),
    )?;
    let combat_root = root.join("resources/generated/combat");
    let data = root.join("artifacts/exploration-runtime-scripts");
    let bundle = maa_adapter::combat_vision::CombatBundle::load(&combat_root)?;
    let revision = zzz_engine::combat::script_catalog(&bundle, &data)?.revision;
    let mut config = od_core::combat::CombatConfig::default();
    config.start_delay_ms = 0;
    config.dodge_only = false;
    let frozen = od_core::app_combat::AppCombatPlan {
        config,
        script_revision: revision,
    };
    let prepared = PreparedIntel::load(frozen.clone(), &combat_root, &data)?;
    let mut plan = od_core::run_plan::plan_standalone_at(
        &od_core::config::AccountConfig::default(),
        1,
        "one_dragon",
        "map_exploration",
        chrono::Utc::now(),
        None,
    )
    .map_err(anyhow::Error::msg)?;
    plan.exploration_combat = Some(frozen);
    let mut slot = od_core::RunSlot::default();
    let record = slot
        .begin_group_plan(plan.mail_config, &plan)
        .map_err(anyhow::Error::msg)?;
    let mut run = ExplorationRun::new(record.id, record.daily[0].clone(), prepared, plan.game_day)?;
    run.progress.stage = "Follow".into();
    let control = run.control();
    let page = Arc::new(AtomicUsize::new(18));
    let events = Arc::new(Mutex::new(vec![]));
    let mut native = Some(NativeSession::for_custom_controller(
        MapController {
            recover: false,
            recover_navigation: false,
            transition: false,
            transition_captures: AtomicUsize::new(0),
            page: page.clone(),
            keys: Arc::default(),
            key_events: events.clone(),
            attacks: Arc::default(),
            clicks: Arc::default(),
            swipes: Arc::default(),
        },
        &root.join("resources/generated/world-patrol"),
        InputMode::Foreground,
    )?);
    let window = od_core::WindowInfo {
        hwnd: "fixture".into(),
        pid: 0,
        title: "到点转向交互回归".into(),
        executable: String::new(),
        width: 1920,
        height: 1080,
        keyboard_permission: None,
    };
    let mut approached = false;
    let mut confirmed = false;
    run.execute(
        &mut native,
        IntelSessionOptions {
            window: &window,
            capture: Default::default(),
            resources: &root.join("resources"),
            fixture: true,
        },
        |progress| {
            assert_ne!(
                progress.stage, "RecoverTrack",
                "到点不能继续按导航丢失反复追踪"
            );
            assert!(progress.active_ms < 30_000, "到点必须转向、按 F 并验证解锁");
            if progress.stage == "ApproachTarget" {
                approached = true;
            }
            if progress.exploration.as_ref().unwrap().observations == 1 {
                assert!(approached);
                assert_eq!(page.load(Ordering::Acquire), 22, "必须回读地图的已解锁状态");
                assert!(!progress.exploration.as_ref().unwrap().interaction_attempted);
                assert!(
                    !progress.completion_proven,
                    "单个奇点解锁不代表全区域卡格车完成"
                );
                confirmed = true;
                assert!(control.stop());
            }
            Ok(())
        },
    )?;
    assert!(confirmed);
    assert!(run.progress.inputs_released);
    let relevant: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .copied()
        .filter(|e| matches!(e.0, 87 | 70))
        .collect();
    assert_eq!(
        relevant,
        vec![(87, true), (87, false), (70, true), (70, false)],
        "先短按前进转人物，再按一次 F"
    );
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、战斗模型和生成后的 world-patrol 资源"]
fn patrol_combat_recognition_fits_the_configured_freshness_budget() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/exploration-runtime-logs"),
    )?;
    let bundle =
        maa_adapter::combat_vision::CombatBundle::load(&root.join("resources/generated/combat"))?;
    let native = NativeSession::for_fixture(
        &root.join("crates/maa-adapter/tests/fixtures/map-exploration/follow-world.png"),
        &bundle.root,
        maa_adapter::InferenceDevice::Cpu,
    )?;
    let mut recognizer = maa_adapter::combat_vision::CombatRecognizer::new(bundle);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    recognizer.warm_patrol(&native, true, &cancel)?;
    for now in 0..3 {
        let mut frame = native.combat_frame(&cancel)?.unwrap();
        assert!(recognizer.align_patrol_frame(&native, &mut frame, &cancel)?);
        let observation = recognizer.observe(
            &native,
            &frame,
            f64::from(now),
            &Default::default(),
            false,
            &cancel,
        )?;
        eprintln!(
            "frame {now}: {}ms, team {:?}, {} states",
            frame.age_ms(),
            observation.team,
            observation.states.len()
        );
        assert!(observation.in_battle);
        assert!(
            observation
                .states
                .iter()
                .any(|state| state.name == "前台-能量"),
            "尚未命中角色头像时，通用脚本也需要真实读取的前台能量状态才能出招"
        );
        assert!(frame.age_ms() <= 250, "识别不能让每帧超期而一直不出招");
    }
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、战斗模型和生成后的 world-patrol 资源"]
fn follow_holds_w_and_yields_to_stop_pause_lost_guide_or_combat() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/exploration-runtime-logs"),
    )?;
    let combat_root = root.join("resources/generated/combat");
    let data = root.join("artifacts/exploration-runtime-scripts");
    let window = od_core::WindowInfo {
        hwnd: "fixture".into(),
        pid: 0,
        title: "模拟连续导航".into(),
        executable: String::new(),
        width: 1920,
        height: 1080,
        keyboard_permission: None,
    };
    for end in [
        "stop",
        "pause",
        "lost",
        "combat",
        "combat_road",
        "road_stuck",
    ] {
        let bundle = maa_adapter::combat_vision::CombatBundle::load(&combat_root)?;
        let revision = zzz_engine::combat::script_catalog(&bundle, &data)?.revision;
        let mut config = od_core::combat::CombatConfig::default();
        config.start_delay_ms = 0;
        config.dodge_only = false;
        let frozen = od_core::app_combat::AppCombatPlan {
            config,
            script_revision: revision,
        };
        let prepared = PreparedIntel::load(frozen.clone(), &combat_root, &data)?;
        let mut plan = od_core::run_plan::plan_standalone_at(
            &od_core::config::AccountConfig::default(),
            1,
            "one_dragon",
            "map_exploration",
            chrono::Utc::now(),
            None,
        )
        .map_err(anyhow::Error::msg)?;
        plan.exploration_combat = Some(frozen);
        let mut slot = od_core::RunSlot::default();
        let record = slot
            .begin_group_plan(plan.mail_config, &plan)
            .map_err(anyhow::Error::msg)?;
        let mut run =
            ExplorationRun::new(record.id, record.daily[0].clone(), prepared, plan.game_day)?;
        run.progress.stage = "Follow".into();
        let control = run.control();
        let page = Arc::new(AtomicUsize::new(6));
        let events = Arc::new(Mutex::new(vec![]));
        let clicks = Arc::new(Mutex::new(vec![]));
        let attacks = Arc::new(AtomicUsize::new(0));
        let mut native = Some(NativeSession::for_custom_controller(
            MapController {
                recover: false,
                recover_navigation: end == "combat",
                transition: false,
                transition_captures: AtomicUsize::new(0),
                page: page.clone(),
                keys: Arc::default(),
                key_events: events.clone(),
                attacks: attacks.clone(),
                clicks: clicks.clone(),
                swipes: Arc::default(),
            },
            &root.join("resources/generated/world-patrol"),
            InputMode::Foreground,
        )?);
        let mut moving_frames = 0;
        let mut lost_released = false;
        let mut resumed_after_battle = false;
        let mut road_frames = 0;
        let mut stuck_released = false;
        run.execute(
            &mut native,
            IntelSessionOptions {
                window: &window,
                capture: Default::default(),
                resources: &root.join("resources"),
                fixture: true,
            },
            |progress| {
                if matches!(end, "combat" | "combat_road" | "road_stuck")
                    && progress.stage == "Combat"
                {
                    let w: Vec<_> = events
                        .lock()
                        .unwrap()
                        .iter()
                        .copied()
                        .filter(|event| event.0 == 87)
                        .collect();
                    assert_eq!(w, vec![(87, true), (87, false)], "切战斗前必须松开移动键");
                    if let Some(battle) = &progress.intel_battle {
                        assert!(battle.frames < 30, "不能进入战斗后一直不出招：{battle:?}");
                        if attacks.load(Ordering::Acquire) > 0 {
                            assert!(battle.actions > 0);
                            page.store(
                                if matches!(end, "combat_road" | "road_stuck") {
                                    17
                                } else {
                                    15
                                },
                                Ordering::Release,
                            );
                        }
                    }
                    return Ok(());
                }
                if end == "road_stuck" && progress.intel_battle.is_some() {
                    assert!(progress.active_ms < 40_000, "道路无进展时不能一直按住前进");
                    if progress.stage == "RecoverTrack" {
                        assert!(progress.inputs_released);
                        let w: Vec<_> = events
                            .lock()
                            .unwrap()
                            .iter()
                            .copied()
                            .filter(|event| event.0 == 87)
                            .collect();
                        assert_eq!(w, vec![(87, true), (87, false), (87, true), (87, false)]);
                        stuck_released = true;
                        assert!(control.stop());
                    }
                    return Ok(());
                }
                if end == "combat_road" && progress.intel_battle.is_some() {
                    assert_ne!(
                        progress.stage, "RecoverTrack",
                        "战斗区域无绿点但有连通道路时不能反复重追踪"
                    );
                    assert!(progress.active_ms < 40_000);
                    if progress.stage == "CrossZone" && !progress.inputs_released {
                        road_frames += 1;
                        let w: Vec<_> = events
                            .lock()
                            .unwrap()
                            .iter()
                            .copied()
                            .filter(|event| event.0 == 87)
                            .collect();
                        assert_eq!(
                            w,
                            vec![(87, true), (87, false), (87, true)],
                            "无绿点时也应沿道路连续移动"
                        );
                        if road_frames == 3 {
                            page.store(6, Ordering::Release);
                        }
                    } else if progress.stage == "Follow"
                        && road_frames == 3
                        && !progress.inputs_released
                    {
                        resumed_after_battle = true;
                        assert!(control.stop());
                    }
                    return Ok(());
                }
                if end == "combat" && progress.intel_battle.is_some() {
                    assert_eq!(
                        progress.intel_battle.as_ref().unwrap().state,
                        od_core::RunState::Succeeded
                    );
                    assert!(
                        progress.active_ms < 40_000,
                        "战斗后必须恢复导航而非停在交接点"
                    );
                    if progress.stage == "Follow" && !progress.inputs_released {
                        let navigation_keys: Vec<_> = events
                            .lock()
                            .unwrap()
                            .iter()
                            .copied()
                            .filter(|event| matches!(event.0, 87 | 27))
                            .collect();
                        assert_eq!(
                            navigation_keys,
                            vec![(87, true), (87, false), (27, true), (27, false), (87, true)]
                        );
                        resumed_after_battle = true;
                        assert!(control.stop());
                    }
                    return Ok(());
                }
                assert_eq!(progress.stage, "Follow");
                assert!(moving_frames < 5, "导航必须及时响应结束条件");
                if !progress.inputs_released {
                    moving_frames += 1;
                    let w: Vec<_> = events
                        .lock()
                        .unwrap()
                        .iter()
                        .copied()
                        .filter(|event| event.0 == 87)
                        .collect();
                    assert_eq!(
                        w,
                        vec![(87, true)],
                        "{end}: 第 {moving_frames} 次有效导航更新不能松开或重复按下 W（{}ms）",
                        progress.active_ms
                    );
                    if moving_frames == 3 {
                        match end {
                            "pause" => {
                                assert!(control.pause());
                            }
                            "lost" => page.store(1, Ordering::Release),
                            "combat" | "combat_road" | "road_stuck" => {
                                page.store(13, Ordering::Release)
                            }
                            _ => {
                                assert!(control.stop());
                            }
                        }
                    }
                } else if moving_frames >= 3 && end == "lost" {
                    lost_released = true;
                    assert!(control.stop());
                }
                Ok(())
            },
        )?;
        assert_eq!(moving_frames, 3, "必须验证至少三次连续导航读帧");
        assert_eq!(lost_released, end == "lost");
        assert_eq!(stuck_released, end == "road_stuck");
        if matches!(end, "combat" | "combat_road") {
            assert!(
                resumed_after_battle,
                "必须验证重建追踪后再次按住 W，而非刚结束战斗就停止回归"
            );
            assert!(
                attacks.load(Ordering::Acquire) > 0,
                "通用脚本必须真正发送鼠标左键攻击输入"
            );
        }
        if end == "combat" {
            let clicks = clicks.lock().unwrap();
            assert_eq!(
                clicks
                    .iter()
                    .filter(|p| (650..950).contains(&p[0]) && (570..710).contains(&p[1]))
                    .count(),
                1,
                "恢复追踪只取消附近传送，保留战斗后位置"
            );
            assert!(
                !clicks
                    .iter()
                    .any(|p| (980..1280).contains(&p[0]) && (570..710).contains(&p[1])),
                "战斗后不能重复确认附近传送"
            );
        }
        assert_eq!(
            run.progress.state,
            if end == "pause" {
                MailState::Paused
            } else {
                MailState::Stopped
            }
        );
        assert!(run.progress.inputs_released);
        assert!(!native.as_ref().unwrap().patrol_is_walking());
        let w: Vec<_> = events
            .lock()
            .unwrap()
            .iter()
            .copied()
            .filter(|event| event.0 == 87)
            .collect();
        assert_eq!(
            w,
            if matches!(end, "combat" | "combat_road" | "road_stuck") {
                vec![(87, true), (87, false), (87, true), (87, false)]
            } else {
                vec![(87, true), (87, false)]
            },
            "{end}: 必须释放连续移动键"
        );
    }
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、战斗模型和生成后的 world-patrol 资源"]
fn opens_local_map_without_n_and_preserves_failure_reason() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/exploration-runtime-logs"),
    )?;
    let combat_root = root.join("resources/generated/combat");
    let data = root.join("artifacts/exploration-runtime-scripts");
    let window = od_core::WindowInfo {
        hwnd: "fixture".into(),
        pid: 0,
        title: "模拟控制器".into(),
        executable: String::new(),
        width: 1920,
        height: 1080,
        keyboard_permission: None,
    };
    for initial_page in [1, 0, 5, 7, 12, 13] {
        let bundle = maa_adapter::combat_vision::CombatBundle::load(&combat_root)?;
        let revision = zzz_engine::combat::script_catalog(&bundle, &data)?.revision;
        let mut combat = od_core::combat::CombatConfig::default();
        combat.start_delay_ms = 0;
        combat.dodge_only = false;
        let frozen = od_core::app_combat::AppCombatPlan {
            config: combat,
            script_revision: revision,
        };
        let prepared = PreparedIntel::load(frozen.clone(), &combat_root, &data)?;
        let config = od_core::config::AccountConfig::default();
        let mut plan = od_core::run_plan::plan_standalone_at(
            &config,
            1,
            "one_dragon",
            "map_exploration",
            chrono::Utc::now(),
            None,
        )
        .map_err(anyhow::Error::msg)?;
        plan.exploration_combat = Some(frozen);
        let mut slot = od_core::RunSlot::default();
        let record = slot
            .begin_group_plan(plan.mail_config, &plan)
            .map_err(anyhow::Error::msg)?;
        slot.confirm_group_saved(&record)
            .map_err(anyhow::Error::msg)?;
        let mut run =
            ExplorationRun::new(record.id, record.daily[0].clone(), prepared, plan.game_day)?;
        if initial_page == 5 {
            run.progress.stage = "Track".into();
        }
        let page = Arc::new(AtomicUsize::new(initial_page));
        let keys = Arc::new(Mutex::new(vec![]));
        let clicks = Arc::new(Mutex::new(vec![]));
        let swipes = Arc::new(Mutex::new(vec![]));
        let controller = MapController {
            recover: initial_page == 7,
            recover_navigation: false,
            transition: initial_page == 12,
            transition_captures: AtomicUsize::new(0),
            page: page.clone(),
            keys: keys.clone(),
            key_events: Arc::default(),
            attacks: Arc::default(),
            clicks: clicks.clone(),
            swipes: swipes.clone(),
        };
        let mut native = Some(NativeSession::for_custom_controller(
            controller,
            &root.join("resources/generated/world-patrol"),
            InputMode::Foreground,
        )?);
        let mut injected = false;
        let mut published = vec![];
        let result = run.execute(
            &mut native,
            IntelSessionOptions {
                window: &window,
                capture: Default::default(),
                resources: &root.join("resources"),
                fixture: true,
            },
            |progress: DailyProgress| {
                if initial_page != 13 {
                    assert_ne!(
                        progress.stage, "Combat",
                        "探索 HUD、地图与地图过渡帧不能触发战斗"
                    );
                }
                published.push(progress.state);
                if progress.stage
                    == if initial_page == 13 {
                        "Combat"
                    } else if matches!(initial_page, 5 | 7) {
                        "Follow"
                    } else {
                        "Detail"
                    }
                    && !injected
                {
                    injected = true;
                    anyhow::bail!("模拟检查点故障，必须保留这条原因");
                }
                slot.update_daily(record.id, progress, "空洞探索执行中".into())
                    .map_err(anyhow::Error::msg)?;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(injected, "未识别并点击拖图后的观测奇点：{}", run.message);
        if initial_page == 13 {
            assert_eq!(page.load(Ordering::Acquire), 13);
            assert!(clicks.lock().unwrap().is_empty());
            assert!(swipes.lock().unwrap().is_empty());
            assert!(keys.lock().unwrap().is_empty());
        } else if matches!(initial_page, 5 | 7) {
            let frame = native
                .as_ref()
                .unwrap()
                .combat_frame(&std::sync::atomic::AtomicBool::new(false))?
                .unwrap();
            assert!(
                native.as_ref().unwrap().patrol_world_in_combat(&frame)?,
                "传送后的实际探索 HUD 必须也能结束战斗分段"
            );
            let world = native.as_ref().unwrap().inspect_map_exploration(
                od_core::map_exploration::ExplorationTarget::Observation,
            )?;
            assert_eq!(world["non_combat_ui"], true);
            assert!(world["guide"].is_object(), "实际引导线必须可用：{world}");
            assert_eq!(
                page.load(Ordering::Acquire),
                6,
                "必须确认附近传送并读取新的世界画面"
            );
            let clicks = clicks.lock().unwrap();
            if initial_page == 5 {
                assert!(swipes.lock().unwrap().is_empty());
                assert_eq!(clicks.len(), 1, "只确认一次传送弹窗");
            } else {
                assert_eq!(swipes.lock().unwrap().len(), 2);
                assert_eq!(
                    clicks
                        .iter()
                        .filter(|p| (1400..1830).contains(&p[0]) && (850..990).contains(&p[1]))
                        .count(),
                    2,
                    "必须先取消旧追踪再重建追踪"
                );
            }
            let confirmed = clicks.last().unwrap();
            assert!(
                (confirmed[0] - 1120).abs() < 50 && (confirmed[1] - 628).abs() < 30,
                "{clicks:?}"
            );
        } else {
            assert_eq!(
                page.load(Ordering::Acquire),
                4,
                "必须先缩放再拖图才进入扫描"
            );
            let drags = swipes.lock().unwrap();
            assert_eq!(drags.len(), 2, "{drags:?}");
            assert!(drags[0][1] > 1000 && drags[0][2] == 834, "{drags:?}");
            assert_eq!(drags[1], [720, 540, 200, 540]);
            assert!(!keys.lock().unwrap().contains(&78));
            assert!(!clicks.lock().unwrap().is_empty());
            let selected = *clicks.lock().unwrap().last().unwrap();
            assert!(
                (selected[0] - 938).abs() < 20 && (selected[1] - 444).abs() < 20,
                "{selected:?}"
            );
            let sight = native.as_ref().unwrap().inspect_map_exploration(
                od_core::map_exploration::ExplorationTarget::Observation,
            )?;
            assert_eq!(sight["cargo_count"], serde_json::json!([9, 17]));
            assert_eq!(sight["zoom"]["at_minimum"], true);
        }
        assert!(
            published.iter().all(|state| !state.is_terminal()),
            "终态应由分项包装器保存一次"
        );
        assert_eq!(run.progress.state, MailState::Failed);
        assert!(run.progress.inputs_released);
        let saved = slot
            .update_daily(record.id, run.progress.clone(), run.message.clone())
            .map_err(anyhow::Error::msg)?;
        assert!(
            saved.message.contains("模拟检查点故障，必须保留这条原因"),
            "{}",
            saved.message
        );
    }
    Ok(())
}
