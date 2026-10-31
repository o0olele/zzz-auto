//! 真实 SDK 队列、模拟控制器；不向游戏发送输入。
use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct ReleaseState {
    down: AtomicBool,
    slow: AtomicBool,
    first_up: AtomicBool,
    fail: AtomicBool,
    events: Mutex<Vec<(String, Instant)>>,
}
impl ReleaseState {
    fn record(&self, event: String) {
        self.events.lock().unwrap().push((event, Instant::now()));
    }
}
struct ReleaseController {
    png: Vec<u8>,
    state: Arc<ReleaseState>,
}
impl maa_adapter::CustomControllerCallback for ReleaseController {
    fn connect(&self) -> bool {
        true
    }
    fn request_uuid(&self) -> Option<String> {
        Some("combat-release".into())
    }
    fn screencap(&self) -> Option<Vec<u8>> {
        Some(self.png.clone())
    }
    fn key_down(&self, key: i32) -> bool {
        self.state.record(format!("down:{key}"));
        self.state.down.store(true, Ordering::Release);
        true
    }
    fn key_up(&self, key: i32) -> bool {
        if !self.state.first_up.swap(true, Ordering::AcqRel) {
            self.state.slow.store(true, Ordering::Release);
            std::thread::sleep(Duration::from_millis(300));
            self.state.slow.store(false, Ordering::Release);
        }
        let ok = !self.state.fail.load(Ordering::Acquire) || key != 69;
        self.state
            .record(format!("{}:{key}", if ok { "up" } else { "failed" }));
        ok
    }
    fn touch_down(&self, _: i32, _: i32, _: i32, _: i32) -> bool {
        true
    }
    fn touch_up(&self, contact: i32) -> bool {
        self.state.record(format!("touch_up:{contact}"));
        true
    }
}
struct ReleaseReader {
    state: Arc<ReleaseState>,
    fired: AtomicBool,
    after_flash: AtomicBool,
}
impl maa_adapter::CombatVision for ReleaseReader {
    fn match_template(
        &self,
        _: &CombatFrame,
        _: [i32; 4],
        _: &maa_adapter::CombatTemplate,
    ) -> Result<Option<f64>> {
        if self.after_flash.swap(false, Ordering::AcqRel) {
            // 早期闪光已发布，完整识别等到主线程开始释放后才返回。
            let deadline = Instant::now() + Duration::from_secs(2);
            while !self.state.slow.load(Ordering::Acquire) {
                ensure!(Instant::now() < deadline, "抢占未开始释放");
                std::thread::sleep(Duration::from_millis(1));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(Some(1.0))
    }
    fn recognize_prepared_cancellable(
        &self,
        _: &maa_adapter::RecognitionImage,
        _: &str,
        _: &serde_json::Value,
        _: &maa_adapter::RawRecognitionControl,
    ) -> Result<maa_adapter::RecognitionMeasurement> {
        let flash =
            self.state.down.load(Ordering::Acquire) && !self.fired.swap(true, Ordering::AcqRel);
        self.after_flash.store(flash, Ordering::Release);
        Ok(maa_adapter::RecognitionMeasurement {
            detail: json!({"adaptation":{"class_index": if flash {1} else {0}}}),
            native_status: "test".into(),
            timing: maa_adapter::RecognitionTiming {
                session_setup_us: 0,
                prepare_us: 0,
                enqueue_us: 0,
                completion_us: 0,
                details_us: 0,
                total_us: 0,
            },
        })
    }
}
fn fixture(state: Arc<ReleaseState>) -> Result<(std::path::PathBuf, NativeSession)> {
    use base64::Engine;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/combat-release-native"),
    )?;
    let fixture = NativeSession::for_fixture(
        &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
        &root.join("resources/generated/combat"),
        maa_adapter::InferenceDevice::Cpu,
    )?;
    let png = base64::engine::general_purpose::STANDARD.decode(fixture.screenshot()?.png_base64)?;
    let native = NativeSession::for_custom_controller(
        ReleaseController { png, state },
        &root.join("resources/generated/combat"),
        od_core::InputMode::Foreground,
    )?;
    Ok((root, native))
}

#[test]
#[ignore = "需要固定 Maa SDK、生成后的战斗资源和固定截图"]
fn slow_release_keeps_loop_live_and_orders_replacement_and_switch() -> Result<()> {
    for (switch, pause) in [(false, false), (true, false), (false, true)] {
        let state = Arc::new(ReleaseState::default());
        let (root, mut native) = fixture(state.clone())?;
        let mut recognizer = native.load_combat_bundle(&root.join("resources/generated/combat"))?;
        let operations = if switch {
            "          - op_name: 按键-特殊攻击-按下\n          - op_name: 等待秒数\n            seconds: 0.1\n          - op_name: 按键-切换角色-下一个\n"
        } else {
            "          - op_name: 按键-特殊攻击-按下\n          - op_name: 等待秒数\n            seconds: 2\n"
        };
        let trigger = if switch {
            ""
        } else {
            "  - triggers: [闪避识别-红光]\n    priority: 99\n    handlers:\n      - states: '[闪避识别-红光]'\n        operations:\n          - op_name: 按键-终结技\n"
        };
        let script = Script::parse(
            "release",
            "release.yml",
            &format!(
                "auto_lock_interval: -1\nauto_turn_interval: -1\ncheck_agent_interval: 0\nscenes:\n{trigger}  - handlers:\n      - states: '[按键可用-普通攻击] | [前台-叶瞬光]'\n        operations:\n{operations}"
            ),
            &od_core::conditional::FileTemplates {
                root: root.join("resources/generated/combat/scripts"),
            },
        )?;
        let mut run = CombatRun::new(
            Uuid::new_v4(),
            CombatConfig {
                start_delay_ms: 0,
                timeout_ms: 5000,
                max_frame_age_ms: 1000,
                ..Default::default()
            },
            script,
        )?;
        let path = run.enable_performance(&root.join("artifacts/combat-release"), "fixture")?;
        let worker = crate::combat_worker::CombatWorker::new(
            ReleaseReader {
                state: state.clone(),
                fired: false.into(),
                after_flash: false.into(),
            },
            recognizer.clone(),
        )?;
        let control = run.control();
        let mut live_publishes = 0;
        run.execute_with_worker(
            &mut native,
            &mut recognizer,
            false,
            &mut |_| {
                if state.slow.load(Ordering::Acquire) {
                    live_publishes += 1;
                    if pause {
                        control.pause();
                    }
                }
                if state
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|e| e.0.starts_with("down:"))
                    .count()
                    >= 2
                {
                    control.stop();
                }
                Ok(())
            },
            &mut |_, _, _, _| Ok(CombatFrameGate::Proceed),
            worker,
        )?;
        assert!(
            live_publishes >= if pause { 1 } else { 2 },
            "慢释放期间主循环必须继续运行"
        );
        assert!(native.inputs_released());
        assert!(!state.slow.load(Ordering::Acquire));
        let events = state.events.lock().unwrap();
        let released = events
            .iter()
            .find(|e| e.0 == "up:69")
            .context("没有释放旧按键")?
            .1;
        let next_down = events.iter().filter(|e| e.0.starts_with("down:")).nth(1);
        if pause {
            assert!(next_down.is_none(), "暂停不能启动替代动作");
        } else {
            assert!(next_down.context("没有替代动作")?.1 >= released);
        }
        drop(events);
        drop(run);
        let events: Vec<serde_json::Value> = std::fs::read_to_string(path)?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let start = events
            .iter()
            .find(|e| e["event"] == "release_start")
            .context("没有释放诊断")?;
        assert_eq!(
            start["data"]["reason"],
            if switch { "clear_inputs" } else { "replace" }
        );
        if pause {
            assert!(events.iter().any(|e| e["event"] == "release_handoff"));
        } else {
            let end = events
                .iter()
                .find(|e| e["event"] == "release_complete")
                .context("释放未确认")?;
            assert!(end["data"]["duration_us"].as_u64().unwrap() >= 290_000);
            assert!(end["data"]["main_thread_us"].as_u64().unwrap() < 50_000);
            if !switch {
                assert!(events.iter().any(|e| e["event"] == "recognition_consumed"
                    && e["at_us"].as_u64() > start["at_us"].as_u64()
                    && e["at_us"].as_u64() < end["at_us"].as_u64()));
            }
        }
        assert_eq!(events.last().unwrap()["data"]["queue_dropped"], 0);
    }
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、生成后的战斗资源和固定截图"]
fn release_failure_cleans_other_channels_and_preserves_recovery() -> Result<()> {
    let state = Arc::new(ReleaseState::default());
    let (_, mut native) = fixture(state.clone())?;
    let cancel = AtomicBool::new(false);
    native.combat_frame(&cancel)?;
    for action in [
        od_core::InputProbeAction::Key { key: 69 },
        od_core::InputProbeAction::Key { key: 81 },
        od_core::InputProbeAction::MouseButton {
            button: od_core::MouseButton::Left,
            x: 960,
            y: 540,
        },
    ] {
        native.combat_input(action, true, &cancel)?;
    }
    state.fail.store(true, Ordering::Release);
    native.start_combat_release()?;
    assert!(!native.poll_combat_release()?);
    native.start_combat_release()?; // 重入复用原清理，不能重置或追加释放。
    let deadline = Instant::now() + Duration::from_secs(2);
    while !state.slow.load(Ordering::Acquire) {
        ensure!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let at = Instant::now();
    assert!(!native.poll_combat_release()?);
    assert!(at.elapsed() < Duration::from_millis(50));
    assert!(
        native
            .start_combat_input(od_core::InputProbeAction::Key { key: 70 }, true, &cancel)
            .is_err()
    );
    // 不再轮询的调用者仍须由交接排空，失败不能交出释放证明。
    assert!(native.quiesce().is_err());
    assert!(!native.inputs_released());
    assert!(!state.slow.load(Ordering::Acquire));
    let events = state.events.lock().unwrap();
    for expected in ["failed:69", "up:81", "touch_up:0"] {
        assert!(events.iter().any(|e| e.0 == expected), "未清理 {expected}");
    }
    drop(events);
    state.fail.store(false, Ordering::Release);
    native.quiesce()?;
    assert!(native.inputs_released());
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、生成后的战斗资源和固定截图"]
fn pause_during_release_resumes_with_new_actions_and_frames() -> Result<()> {
    let state = Arc::new(ReleaseState::default());
    let (root, mut native) = fixture(state.clone())?;
    let mut recognizer = native.load_combat_bundle(&root.join("resources/generated/combat"))?;
    let script = Script::parse(
        "release-resume",
        "release-resume.yml",
        r#"
auto_lock_interval: -1
auto_turn_interval: -1
scenes:
  - handlers:
      - states: '[按键可用-普通攻击]'
        operations:
          - op_name: 按键-特殊攻击-按下
          - op_name: 等待秒数
            seconds: 0.1
          - op_name: 按键-切换角色-下一个
"#,
        &od_core::conditional::FileTemplates {
            root: root.join("resources/generated/combat/scripts"),
        },
    )?;
    let mut run = CombatRun::new(
        Uuid::new_v4(),
        CombatConfig {
            start_delay_ms: 0,
            timeout_ms: 5000,
            ..Default::default()
        },
        script,
    )?;
    let path = run.enable_performance(&root.join("artifacts/combat-release"), "fixture")?;
    let control = run.control();
    run.execute(&mut native, &mut recognizer, false, |_| {
        if state.slow.load(Ordering::Acquire) {
            control.pause();
        }
    })?;
    assert_eq!(run.progress.state, RunState::Paused);
    assert!(run.progress.inputs_released && native.inputs_released());
    assert!(
        !state
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.0 == "down:32")
    );
    let previous_events = state.events.lock().unwrap().len();
    let previous_frames = run.progress.frames;
    run.resume()?;
    run.execute(&mut native, &mut recognizer, false, |_| {
        if state.events.lock().unwrap()[previous_events..]
            .iter()
            .any(|e| e.0.starts_with("down:"))
        {
            control.stop();
        }
    })?;
    assert_eq!(run.progress.state, RunState::Stopped);
    assert!(run.progress.inputs_released && native.inputs_released());
    assert!(run.progress.frames > previous_frames);
    assert_eq!(
        state.events.lock().unwrap()[previous_events..]
            .iter()
            .find(|e| e.0.starts_with("down:"))
            .context("没有新动作")?
            .0,
        "down:69"
    );
    drop(run);
    let events: Vec<serde_json::Value> = std::fs::read_to_string(path)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    let resumed = events
        .iter()
        .filter(|e| e["event"] == "loop_start")
        .nth(1)
        .context("没有继续记录")?["at_us"]
        .as_u64()
        .unwrap();
    for event in events
        .iter()
        .filter(|e| e["event"] == "recognition_start" && e["at_us"].as_u64().unwrap() >= resumed)
    {
        assert!(event["data"]["frame_id"].as_u64().unwrap() >= resumed);
    }
    assert!(events.iter().any(|e| e["event"] == "release_handoff"));
    Ok(())
}
