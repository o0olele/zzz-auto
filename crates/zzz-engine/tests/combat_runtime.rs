//! 用真实 Maa 队列和模拟输入控制器复现慢帧，不操作游戏。
use base64::Engine;
use maa_adapter::{CustomControllerCallback, InputMode, NativeSession};
use od_core::{
    combat::CombatConfig,
    conditional::{FileTemplates, Script},
};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use zzz_engine::combat::CombatRun;

struct BattleController {
    png: Vec<u8>,
    slow_capture: Option<usize>,
    captures: Arc<AtomicUsize>,
    keys: Arc<Mutex<Vec<(i32, bool)>>>,
}
impl CustomControllerCallback for BattleController {
    fn connect(&self) -> bool {
        true
    }
    fn request_uuid(&self) -> Option<String> {
        Some("combat-stale-frame-regression".into())
    }
    fn screencap(&self) -> Option<Vec<u8>> {
        if Some(self.captures.fetch_add(1, Ordering::AcqRel)) == self.slow_capture {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        Some(self.png.clone())
    }
    fn key_down(&self, key: i32) -> bool {
        self.keys.lock().unwrap().push((key, true));
        true
    }
    fn key_up(&self, key: i32) -> bool {
        self.keys.lock().unwrap().push((key, false));
        true
    }
    fn touch_down(&self, _contact: i32, _x: i32, _y: i32, _pressure: i32) -> bool {
        true
    }
    fn touch_up(&self, _contact: i32) -> bool {
        true
    }
    fn relative_move(&self, _dx: i32, _dy: i32) -> bool {
        true
    }
}

#[test]
#[ignore = "需要固定 Maa SDK、生成后的战斗资源和战斗画面留档"]
fn stale_frame_releases_inputs_without_recasting_an_applied_buff() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/combat-runtime-logs"),
    )?;
    let png = {
        let fixture = NativeSession::for_fixture(
            &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
            &root.join("resources/generated/combat"),
            maa_adapter::InferenceDevice::Cpu,
        )?;
        base64::engine::general_purpose::STANDARD.decode(fixture.screenshot()?.png_base64)?
    };
    let keys = Arc::new(Mutex::new(vec![]));
    let mut native = NativeSession::for_custom_controller(
        BattleController {
            png,
            slow_capture: Some(1),
            captures: Arc::new(AtomicUsize::new(0)),
            keys: keys.clone(),
        },
        &root.join("resources/generated/combat"),
        InputMode::Foreground,
    )?;
    let mut recognizer = native.load_combat_bundle(&root.join("resources/generated/combat"))?;
    let script = Script::parse(
        "slow-frame",
        "slow-frame.yml",
        r#"
auto_lock_interval: -1
auto_turn_interval: -1
scenes:
  - interval: 0
    handlers:
      - states: "[按键可用-普通攻击] & ![自定义-露西BUFF, 0, 20]"
        operations:
          - op_name: 设置状态
            state: 自定义-露西BUFF
          - op_name: 按键-特殊攻击
      - states: "[按键可用-普通攻击]"
        operations:
          - op_name: 等待秒数
            seconds: 0.02
"#,
        &FileTemplates {
            root: root.join("resources/generated/combat/scripts"),
        },
    )?;
    let config = CombatConfig {
        start_delay_ms: 0,
        timeout_ms: 5_000,
        ..Default::default()
    };
    let mut run = CombatRun::new(uuid::Uuid::new_v4(), config, script)?;
    let control = run.control();
    run.execute(&mut native, &mut recognizer, false, |progress| {
        if progress.frames >= 5 {
            control.stop();
        }
    })?;
    // 截图阻塞也可能使先前在途的识别结果过期，两张旧图都必须被拒绝。
    assert!((1..=2).contains(&run.progress.stale_frames));
    assert!(run.progress.inputs_released);
    let keys = keys.lock().unwrap();
    assert_eq!(
        keys.iter().filter(|event| **event == (69, true)).count(),
        1,
        "慢帧不能清除已执行的 BUFF 状态并再次按 E：{keys:?}"
    );
    assert!(keys.contains(&(69, false)), "E 必须释放");
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、生成后的战斗资源和战斗画面留档"]
fn full_generic_script_keeps_acting_while_discovering_the_team() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/combat-runtime-logs"),
    )?;
    let png = {
        let fixture = NativeSession::for_fixture(
            &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
            &root.join("resources/generated/combat"),
            maa_adapter::InferenceDevice::Cpu,
        )?;
        base64::engine::general_purpose::STANDARD.decode(fixture.screenshot()?.png_base64)?
    };
    let captures = Arc::new(AtomicUsize::new(0));
    let mut native = NativeSession::for_custom_controller(
        BattleController {
            png,
            slow_capture: None,
            captures: captures.clone(),
            keys: Default::default(),
        },
        &root.join("resources/generated/combat"),
        InputMode::Foreground,
    )?;
    // 与真实独立战斗相同：构造控制器时已经载入战斗包，识别器另行校验创建。
    let mut recognizer = maa_adapter::combat_vision::CombatRecognizer::new(
        maa_adapter::combat_vision::CombatBundle::load(&root.join("resources/generated/combat"))?,
    );
    let script = Script::load(
        &root.join("resources/generated/combat/scripts"),
        "全配队通用",
    )?;
    let config = CombatConfig {
        start_delay_ms: 0,
        timeout_ms: 20_000,
        dodge_only: false,
        ..Default::default()
    };
    let mut run = CombatRun::new(uuid::Uuid::new_v4(), config, script)?;
    let control = run.control();
    let started = std::time::Instant::now();
    run.execute(&mut native, &mut recognizer, false, |progress| {
        if progress.frames >= 60 {
            control.stop();
        }
    })?;
    eprintln!(
        "combat_performance {}",
        serde_json::json!({
            "pid":std::process::id(),"seconds":started.elapsed().as_secs_f64(),
            "captures":captures.load(Ordering::Acquire), "observations":run.progress.frames,
            "stale":run.progress.stale_frames,"actions":run.progress.actions,
        })
    );
    eprintln!(
        "generic script: frames {}, stale {}, actions {}, team {:?}",
        run.progress.frames, run.progress.stale_frames, run.progress.actions, run.progress.team
    );
    assert_eq!(run.progress.team, ["叶瞬光", "千夏", "照"]);
    assert!(run.progress.stale_frames <= 2, "除模型冷启动外不能持续丢帧");
    assert!(run.progress.actions >= 10, "完整脚本必须持续出招");
    assert!(run.progress.inputs_released);
    Ok(())
}

#[test]
#[ignore = "需要固定 Maa SDK、生成后的战斗资源和战斗画面留档"]
fn named_switch_waits_for_discovery_and_does_not_attack_the_wrong_agent() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/combat-runtime-logs"),
    )?;
    let png = {
        let fixture = NativeSession::for_fixture(
            &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
            &root.join("resources/generated/combat"),
            maa_adapter::InferenceDevice::Cpu,
        )?;
        base64::engine::general_purpose::STANDARD.decode(fixture.screenshot()?.png_base64)?
    };
    let keys = Arc::new(Mutex::new(vec![]));
    let mut native = NativeSession::for_custom_controller(
        BattleController {
            png,
            slow_capture: None,
            captures: Arc::new(AtomicUsize::new(0)),
            keys: keys.clone(),
        },
        &root.join("resources/generated/combat"),
        InputMode::Foreground,
    )?;
    let mut recognizer = native.load_combat_bundle(&root.join("resources/generated/combat"))?;
    let script = Script::parse(
        "switch-discovery",
        "switch-discovery.yml",
        r#"
auto_lock_interval: -1
auto_turn_interval: -1
check_agent_interval: 0.02
scenes:
  - interval: 0
    handlers:
      - states: "[按键可用-普通攻击] & ![前台-露西]"
        operations:
          - op_name: 切换角色
            agent_name: 叶瞬光
          - op_name: 按键-特殊攻击
"#,
        &FileTemplates {
            root: root.join("resources/generated/combat/scripts"),
        },
    )?;
    let config = CombatConfig {
        start_delay_ms: 0,
        timeout_ms: 20_000,
        dodge_only: false,
        ..Default::default()
    };
    let mut run = CombatRun::new(uuid::Uuid::new_v4(), config, script)?;
    let control = run.control();
    run.execute(&mut native, &mut recognizer, false, |progress| {
        if keys.lock().unwrap().contains(&(69, true)) {
            assert!(progress.team.contains(&"叶瞬光".to_string()));
            assert_eq!(progress.team.first().map(String::as_str), Some("叶瞬光"));
            control.stop();
        }
    })?;
    assert!(run.progress.inputs_released);
    assert!(run.progress.frames > 1, "必须经过目标尚未发现的画面");
    assert!(
        keys.lock().unwrap().contains(&(69, true)),
        "重新识别后必须能恢复后续技能"
    );
    Ok(())
}
