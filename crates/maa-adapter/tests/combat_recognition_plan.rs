//! 同图对照裁剪前的完整识别，验证脚本实际使用的状态保持一致。
use anyhow::Result;
use maa_adapter::{
    InferenceDevice, NativeSession,
    combat_vision::{CombatBundle, CombatRecognitionPlan, CombatRecognizer},
};
use od_core::conditional::{FileTemplates, Script};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

#[test]
#[ignore = "需要固定 Maa SDK、战斗资源和固定截图；只读，不向游戏输入"]
fn planned_recognition_preserves_used_states_and_omits_unused_stages() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    maa_adapter::load_sdk(
        &root.join("sdk/maa/bin"),
        &root.join("artifacts/combat-plan-native"),
    )?;
    let bundle = CombatBundle::load(&root.join("resources/generated/combat"))?;
    let native = NativeSession::for_fixture(
        &root.join("zzz-od-test/screens/战斗画面/默认.webp"),
        &bundle.root,
        InferenceDevice::Cpu,
    )?;
    let base = CombatRecognizer::new(bundle);
    let scripts = base.bundle.root.join("scripts");
    let mut cases = Vec::new();
    for (label, condition, auto) in [
        ("attack", "[按键可用-普通攻击]", false),
        ("front_energy", "[前台-能量]", false),
        ("agent_alias", "[叶瞬光-特殊技可用]", false),
        ("auto_ultimate", "[按键可用-普通攻击]", true),
    ] {
        cases.push((Script::parse(label, "plan.yml", &format!(
            "scenes:\n- handlers:\n  - states: '{condition}'\n    operations:\n    - op_name: 等待秒数\n      seconds: 1\n"), &FileTemplates {root:scripts.clone()})?,auto));
    }
    let agent = base
        .bundle
        .spec
        .agents
        .iter()
        .find(|a| a.name == "叶瞬光")
        .unwrap();
    assert!(!agent.states.is_empty());
    let condition = agent
        .states
        .iter()
        .map(|def| format!("[{}]", def.state_name))
        .collect::<Vec<_>>()
        .join(" | ");
    cases.push((Script::parse("agent_state", "plan.yml", &format!(
        "scenes:\n- handlers:\n  - states: '{condition}'\n    operations:\n    - op_name: 等待秒数\n      seconds: 1\n"), &FileTemplates {root:scripts.clone()})?,false));
    for name in &base.bundle.scripts {
        cases.push((Script::load(&scripts, name)?, false));
    }
    let mut report = Vec::new();
    for (script, auto) in cases {
        let mut full = base.clone();
        let mut planned = base.clone();
        let plan = Arc::new(CombatRecognitionPlan::for_script(
            &base.bundle.spec,
            &script,
            auto,
            false,
        ));
        let dodge_only = plan.dodge_only();
        planned.set_recognition_plan(plan);
        let mut full_calls = 0;
        let mut planned_calls = 0;
        let mut full_us = 0;
        let mut planned_us = 0;
        for index in 0..20 {
            let mut frame = native.combat_frame(&AtomicBool::new(false))?.unwrap();
            let intervals = BTreeMap::from([("flash_enabled".into(), 0.0)]);
            frame.enable_performance();
            let start = std::time::Instant::now();
            let all = full.observe(
                &native,
                &frame,
                index as f64,
                &intervals,
                false,
                &AtomicBool::new(false),
            )?;
            full_us += start.elapsed().as_micros();
            let stages = frame.performance().unwrap();
            full_calls += stages["template"].calls;
            frame.enable_performance();
            let start = std::time::Instant::now();
            let selected = planned.observe(
                &native,
                &frame,
                index as f64,
                &intervals,
                dodge_only,
                &AtomicBool::new(false),
            )?;
            planned_us += start.elapsed().as_micros();
            let stages = frame.performance().unwrap();
            planned_calls += stages["template"].calls;
            if script.name == "attack" {
                for unused in [
                    "agents",
                    "agent_state",
                    "avatar",
                    "quick_check",
                    "backup",
                    "target",
                    "chain",
                ] {
                    assert!(!stages.contains_key(unused), "多余阶段 {unused}");
                }
            }
            if [
                "front_energy",
                "agent_alias",
                "auto_ultimate",
                "agent_state",
            ]
            .contains(&script.name.as_str())
            {
                assert!(stages.contains_key("agents"));
                for unused in ["quick_check", "backup", "target"] {
                    assert!(
                        !stages.contains_key(unused),
                        "{}: 多余阶段 {unused}",
                        script.name
                    );
                }
            }
            let project = |states: Vec<od_core::conditional::StateUpdate>| -> Result<_> {
                Ok(serde_json::to_value(
                    states
                        .into_iter()
                        .filter(|s| {
                            script.states.contains(&s.name) || (auto && s.name == "按键可用-终结技")
                        })
                        .collect::<Vec<_>>(),
                )?)
            };
            assert_eq!(all.in_battle, selected.in_battle);
            assert_eq!(
                project(all.states)?,
                project(selected.states)?,
                "{} frame {index}",
                script.name
            );
        }
        if script.name == "attack" {
            assert!(planned_calls < full_calls);
        }
        report.push(
            serde_json::json!({"script":script.name,"frames":20,"full_template_calls":full_calls,
            "planned_template_calls":planned_calls,"full_us":full_us,"planned_us":planned_us}),
        );
    }
    std::fs::write(
        root.join("artifacts/combat-plan-comparison.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    eprintln!("combat_plan_comparison {}", serde_json::to_string(&report)?);
    Ok(())
}
