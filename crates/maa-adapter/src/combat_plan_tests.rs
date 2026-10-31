use super::*;
use od_core::combat_vision::{Agent, AgentStateDef};

fn spec() -> VisionSpec {
    let state = |name: &str| -> AgentStateDef {
        serde_json::from_value(serde_json::json!({
            "state_name":name, "check_way":"RGB", "template_id":"test"
        }))
        .unwrap()
    };
    VisionSpec {
        schema: 1,
        agents: vec![Agent {
            id: "anby".into(),
            name: "安比".into(),
            kind: "击破".into(),
            templates: vec![],
            states: vec![state("安比-电压")],
        }],
        common: BTreeSet::from([
            "GUARD_BREAK",
            "SWITCH_BAN",
            "LIFE_DEDUCTION_31",
            "LIFE_DEDUCTION_21",
        ])
        .into_iter()
        .map(|k| (k.into(), state(k)))
        .collect(),
        areas: Default::default(),
        templates: Default::default(),
    }
}
fn script(text: &str) -> Script {
    Script::parse(
        "plan",
        "plan.yml",
        text,
        &od_core::conditional::FileTemplates {
            root: Default::default(),
        },
    )
    .unwrap()
}
#[test]
fn simple_attack_omits_unreferenced_vision_but_auto_ultimate_keeps_its_button() {
    let spec = spec();
    let script = script(
        "scenes:\n- handlers:\n  - states: '[按键可用-普通攻击]'\n    operations:\n    - op_name: 按键-普通攻击\n",
    );
    let plan = CombatRecognitionPlan::for_script(&spec, &script, false, false);
    assert!(plan.dodge_only());
    assert!(!plan.flash_enabled());
    let plan = CombatRecognitionPlan::for_script(&spec, &script, true, false);
    assert!(!plan.dodge_only());
    assert!(plan.agents && plan.front_common[2] && plan.common[0][0][2]);
    assert!(!plan.common[0][1][2]);
    assert!(!plan.quick && !plan.chain && !plan.target && !plan.backup);
    let plan = CombatRecognitionPlan::for_script(&spec, &script, false, true);
    assert!(plan.chain && plan.agents);
    assert!(!plan.dodge_only());
}
#[test]
fn nested_conditions_interrupts_and_action_dependencies_are_preserved() {
    let spec = spec();
    let script = script(
        r#"
scenes:
- triggers: [闪避识别-红光]
  handlers:
  - states: '[按键可用-普通攻击]'
    interrupt_states: '[安比-电压]'
    sub_handlers:
    - states: '[后台-1-能量]'
      operations:
      - op_name: 按键-快速支援
      - op_name: 按键-连携技-左
      - op_name: 切换角色
        agent_name: 安比
"#,
    );
    let plan = CombatRecognitionPlan::for_script(&spec, &script, false, false);
    assert!(plan.flash && plan.agents && plan.quick && plan.chain);
    assert!(plan.agent_states[0][0] && plan.common[0][1][0]);
    assert!(!plan.common[0][0][0] && !plan.common[0][1][1]);
    assert!(!plan.target && !plan.backup);
}
#[test]
fn aliases_and_shared_states_select_only_the_needed_checks() {
    let spec = spec();
    let script = script(
        r#"
scenes:
- handlers:
  - states: '[安比-特殊技可用] & [目标-近距离锁定] & [按键可用-切换后援] & [GUARD_BREAK]'
    operations:
    - op_name: 等待秒数
      seconds: 1
"#,
    );
    let plan = CombatRecognitionPlan::for_script(&spec, &script, false, false);
    assert!(plan.agents && plan.target && plan.backup);
    assert_eq!(plan.shared, BTreeSet::from(["GUARD_BREAK".into()]));
    for position in 0..3 {
        assert_eq!(plan.common[0][position], [false, true, false]);
    }
    assert_eq!(plan.front_common, [false; 3]);
    assert!(!plan.quick && !plan.chain && !plan.flash);
}
