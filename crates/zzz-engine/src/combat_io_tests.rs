use super::*;
use base64::Engine;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct CaptureProbe {
    png: Vec<u8>,
    captures: AtomicUsize,
    slow: Arc<AtomicBool>,
    keys: Arc<Mutex<Vec<(bool, Instant)>>>,
    fail: bool,
}
impl maa_adapter::CustomControllerCallback for CaptureProbe {
    fn connect(&self) -> bool {
        true
    }
    fn request_uuid(&self) -> Option<String> {
        Some("combat-async-io".into())
    }
    fn screencap(&self) -> Option<Vec<u8>> {
        if self.captures.fetch_add(1, Ordering::AcqRel) == 1 {
            self.slow.store(true, Ordering::Release);
            std::thread::sleep(Duration::from_millis(300));
            self.slow.store(false, Ordering::Release);
            if self.fail {
                return None;
            }
        }
        Some(self.png.clone())
    }
    fn key_down(&self, _: i32) -> bool {
        self.keys.lock().unwrap().push((true, Instant::now()));
        true
    }
    fn key_up(&self, _: i32) -> bool {
        self.keys.lock().unwrap().push((false, Instant::now()));
        true
    }
    fn touch_down(&self, _: i32, _: i32, _: i32, _: i32) -> bool {
        self.keys.lock().unwrap().push((true, Instant::now()));
        true
    }
    fn touch_up(&self, _: i32) -> bool {
        self.keys.lock().unwrap().push((false, Instant::now()));
        true
    }
}

fn fixture() -> Result<(std::path::PathBuf, Vec<u8>)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/combat-async-io-native"),
    )?;
    let fixture = NativeSession::for_fixture(
        &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
        &root.join("resources/generated/combat"),
        maa_adapter::InferenceDevice::Cpu,
    )?;
    let png = base64::engine::general_purpose::STANDARD.decode(fixture.screenshot()?.png_base64)?;
    Ok((root, png))
}

struct GatedFlash {
    slow: Arc<AtomicBool>,
    calls: AtomicUsize,
}
impl maa_adapter::CombatVision for GatedFlash {
    fn match_template(
        &self,
        _: &CombatFrame,
        _: [i32; 4],
        _: &maa_adapter::CombatTemplate,
    ) -> Result<Option<f64>> {
        Ok(Some(1.0))
    }
    fn recognize_prepared_cancellable(
        &self,
        _: &maa_adapter::RecognitionImage,
        _: &str,
        _: &serde_json::Value,
        _: &maa_adapter::RawRecognitionControl,
    ) -> Result<maa_adapter::RecognitionMeasurement> {
        let first = self.calls.fetch_add(1, Ordering::AcqRel) == 0;
        if first {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !self.slow.load(Ordering::Acquire) {
                ensure!(Instant::now() < deadline, "第二张慢截图没有启动");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(maa_adapter::RecognitionMeasurement {
            detail: json!({"adaptation":{"class_index": if first {1} else {0}}}),
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

#[test]
#[ignore = "需要固定 Maa SDK、战斗资源和固定截图；只使用模拟控制器"]
fn slow_capture_and_queued_input_do_not_block_flash_delivery_or_progress() -> Result<()> {
    let (root, png) = fixture()?;
    let slow = Arc::new(AtomicBool::new(false));
    let keys = Arc::new(Mutex::new(Vec::new()));
    let mut native = NativeSession::for_custom_controller(
        CaptureProbe {
            png,
            captures: 0.into(),
            slow: slow.clone(),
            keys: keys.clone(),
            fail: false,
        },
        &root.join("resources/generated/combat"),
        od_core::InputMode::Foreground,
    )?;
    let mut recognizer = native.load_combat_bundle(&root.join("resources/generated/combat"))?;
    let script = Script::parse(
        "async-flash",
        "async-flash.yml",
        r#"
auto_lock_interval: -1
auto_turn_interval: -1
scenes:
  - triggers: [闪避识别-红光]
    priority: 99
    handlers:
      - states: "[闪避识别-红光]"
        operations:
          - op_name: 按键-闪避
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
            max_frame_age_ms: 1000,
            ..Default::default()
        },
        script,
    )?;
    let path = run.enable_performance(&root.join("artifacts/combat-async-io"), "fixture")?;
    let worker = crate::combat_worker::CombatWorker::new(
        GatedFlash {
            slow: slow.clone(),
            calls: 0.into(),
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
            if slow.load(Ordering::Acquire) {
                live_publishes += 1;
            }
            if keys.lock().unwrap().iter().any(|(down, _)| !down) {
                control.stop();
            }
            Ok(())
        },
        &mut |_, _, _, _| Ok(CombatFrameGate::Proceed),
        worker,
    )?;
    assert!(live_publishes >= 2, "等待慢截图和排队按键时仍须发布进度");
    assert!(!native.combat_capture_pending());
    assert!(native.inputs_released());
    let keys = keys.lock().unwrap();
    let down = keys.iter().find(|e| e.0).context("没有按下")?.1;
    let up = keys.iter().find(|e| !e.0).context("没有松开")?.1;
    assert!(
        up.duration_since(down) >= Duration::from_millis(20),
        "点按从原生确认后计时"
    );
    assert!(up.duration_since(down) < Duration::from_millis(100));
    drop(run);
    let events: Vec<serde_json::Value> = std::fs::read_to_string(path)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    let ready = events
        .iter()
        .find(|e| e["event"] == "flash_ready")
        .context("没有早期闪光")?;
    let consumed = events
        .iter()
        .find(|e| e["event"] == "flash_consumed")
        .context("没有消费闪光")?;
    let wait = consumed["at_us"].as_u64().unwrap() - ready["at_us"].as_u64().unwrap();
    assert!(wait < 40_000, "闪光不应等 300ms 截图完成：{wait}us");
    let capture = events
        .iter()
        .find(|e| e["event"] == "capture" && e["data"]["capture_us"].as_u64().unwrap() > 290_000)
        .context("缺少慢截图记录")?;
    assert!(consumed["at_us"].as_u64().unwrap() < capture["at_us"].as_u64().unwrap());
    assert!(capture["data"]["main_thread_us"].as_u64().unwrap() < 50_000);
    assert_eq!(events.last().unwrap()["data"]["queue_dropped"], 0);
    eprintln!(
        "slow capture flash handoff {wait}us, live publishes {live_publishes}, tap {:?}",
        up.duration_since(down)
    );
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、战斗资源和固定截图；只使用模拟控制器"]
fn capture_poll_and_handoff_drain_pending_input_even_when_capture_fails() -> Result<()> {
    let (root, png) = fixture()?;
    for fail in [false, true] {
        let slow = Arc::new(AtomicBool::new(false));
        let keys = Arc::new(Mutex::new(Vec::new()));
        let mut native = NativeSession::for_custom_controller(
            CaptureProbe {
                png: png.clone(),
                captures: 0.into(),
                slow: slow.clone(),
                keys: keys.clone(),
                fail,
            },
            &root.join("resources/generated/combat"),
            od_core::InputMode::Foreground,
        )?;
        let cancel = std::sync::atomic::AtomicBool::new(false);
        native.combat_frame(&cancel)?;
        native.start_combat_frame(&cancel)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        while !slow.load(Ordering::Acquire) {
            ensure!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let at = Instant::now();
        assert!(native.poll_combat_frame(&cancel)?.is_none());
        let action = if fail {
            od_core::InputProbeAction::Key { key: 69 }
        } else {
            od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x: 960,
                y: 540,
            }
        };
        let mut input = native.start_combat_input(action, true, &cancel)?.unwrap();
        assert!(!native.poll_combat_input(&mut input)?);
        assert!(
            at.elapsed() < Duration::from_millis(50),
            "查询不能阻塞等待截图"
        );
        let result = native.quiesce();
        assert_eq!(result.is_err(), fail);
        assert!(native.inputs_released());
        assert!(!slow.load(Ordering::Acquire));
        assert_eq!(
            keys.lock().unwrap().iter().map(|e| e.0).collect::<Vec<_>>(),
            [true, false]
        );
        // 排空的旧图不能重新作为新结果；下一次读取必须来自新的截图请求。
        assert!(native.poll_combat_frame(&cancel)?.is_none());
        let resumed = Instant::now();
        let fresh = native.combat_frame(&cancel)?.unwrap();
        assert!(fresh.captured >= resumed);
    }
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、战斗资源和固定截图；只使用模拟控制器"]
fn pause_during_capture_drains_old_frame_before_resuming_same_session() -> Result<()> {
    let (root, png) = fixture()?;
    let slow = Arc::new(AtomicBool::new(false));
    let keys = Arc::new(Mutex::new(Vec::new()));
    let mut native = NativeSession::for_custom_controller(
        CaptureProbe {
            png,
            captures: 0.into(),
            slow: slow.clone(),
            keys: keys.clone(),
            fail: false,
        },
        &root.join("resources/generated/combat"),
        od_core::InputMode::Foreground,
    )?;
    let mut recognizer = native.load_combat_bundle(&root.join("resources/generated/combat"))?;
    let script = Script::parse(
        "pause-capture",
        "pause-capture.yml",
        r#"
auto_lock_interval: -1
auto_turn_interval: -1
scenes:
  - handlers:
      - states: "[按键可用-普通攻击]"
        operations:
          - op_name: 按键-特殊攻击
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
    let path = run.enable_performance(&root.join("artifacts/combat-async-io"), "fixture")?;
    let control = run.control();
    run.execute(&mut native, &mut recognizer, false, |_| {
        if slow.load(Ordering::Acquire) {
            control.pause();
        }
    })?;
    assert_eq!(run.progress.state, RunState::Paused);
    assert!(run.progress.inputs_released && native.inputs_released());
    assert!(!native.combat_capture_pending());
    assert!(!slow.load(Ordering::Acquire));
    let frames = run.progress.frames;
    run.resume()?;
    run.execute(&mut native, &mut recognizer, false, |progress| {
        if progress.frames >= frames + 3 {
            control.stop();
        }
    })?;
    assert_eq!(run.progress.state, RunState::Stopped);
    assert!(run.progress.inputs_released && native.inputs_released());
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
        assert!(
            event["data"]["frame_id"].as_u64().unwrap() >= resumed,
            "继续后不能识别暂停前的截图"
        );
    }
    assert_eq!(events.last().unwrap()["data"]["queue_dropped"], 0);
    Ok(())
}
