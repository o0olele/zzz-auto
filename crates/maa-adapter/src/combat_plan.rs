//! 从展开后的脚本和运行功能一次编译识别需求；每帧只查询布尔标记。
use od_core::{
    combat_vision::VisionSpec,
    conditional::{Action, ButtonWay, Handler, Script},
};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Serialize)]
pub struct CombatRecognitionPlan {
    pub(crate) flash: bool,
    pub(crate) agents: bool,
    pub(crate) quick: bool,
    pub(crate) chain: bool,
    pub(crate) target: bool,
    pub(crate) backup: bool,
    // agent / position / ENERGY, SPECIAL, ULTIMATE
    pub(crate) common: Vec<[[bool; 3]; 3]>,
    pub(crate) front_common: [bool; 3],
    pub(crate) agent_states: Vec<Vec<bool>>,
    pub(crate) shared: BTreeSet<String>,
}

impl CombatRecognitionPlan {
    /// 边界判断需要连携画面证明；自动终结技需要真实前台终结技按钮。
    pub fn for_script(
        spec: &VisionSpec,
        script: &Script,
        auto_ultimate: bool,
        boundary: bool,
    ) -> Self {
        let states = &script.states;
        let wants = |name: &str| states.contains(name);
        let front_common = ["能量", "特殊技可用", "终结技可用"].map(|suffix| {
            wants(&format!("前台-{suffix}"))
                || match suffix {
                    "特殊技可用" => wants("按键可用-特殊攻击"),
                    "终结技可用" => auto_ultimate || wants("按键可用-终结技"),
                    _ => false,
                }
        });
        let mut plan = Self {
            flash: wants("闪避识别-红光") || wants("闪避识别-黄光"),
            agents: false,
            quick: wants("按键可用-快速支援") || states.iter().any(|s| s.starts_with("快速支援-")),
            chain: boundary
                || wants("按键可用-连携技")
                || states.iter().any(|s| s.starts_with("连携技-")),
            target: wants("目标-近距离锁定"),
            backup: wants("按键可用-切换后援"),
            common: Vec::new(),
            front_common,
            agent_states: Vec::new(),
            shared: BTreeSet::new(),
        };
        for agent in &spec.agents {
            let common = std::array::from_fn(|position| {
                let prefix = if position == 0 {
                    "前台-".into()
                } else {
                    format!("后台-{position}-")
                };
                plan.agents |= [&agent.name, &agent.kind]
                    .iter()
                    .any(|suffix| wants(&format!("{prefix}{suffix}")));
                ["能量", "特殊技可用", "终结技可用"].map(|suffix| {
                    wants(&format!("{prefix}{suffix}"))
                        || wants(&format!("{}-{suffix}", agent.name))
                        || (position == 0
                            && front_common[match suffix {
                                "能量" => 0,
                                "特殊技可用" => 1,
                                _ => 2,
                            }])
                })
            });
            plan.agents |= wants(&format!("后台-{}", agent.name))
                || [&agent.name, &agent.kind]
                    .iter()
                    .any(|s| wants(&format!("切换角色-{s}")))
                || common.iter().flatten().any(|enabled| *enabled);
            plan.common.push(common);
            let selected: Vec<_> = agent
                .states
                .iter()
                .map(|def| wants(&def.state_name))
                .collect();
            plan.agents |= selected.iter().any(|enabled| *enabled);
            plan.agent_states.push(selected);
        }
        for key in [
            "GUARD_BREAK",
            "SWITCH_BAN",
            "LIFE_DEDUCTION_31",
            "LIFE_DEDUCTION_21",
        ] {
            if spec
                .common
                .get(key)
                .is_some_and(|def| wants(&def.state_name))
            {
                plan.shared.insert(key.into());
                plan.agents = true;
            }
        }
        fn actions(plan: &mut CombatRecognitionPlan, handlers: &[Handler]) {
            for handler in handlers {
                for action in &handler.actions {
                    match action {
                        Action::SwitchAgent { .. } => plan.agents = true,
                        Action::QuickAssist => {
                            plan.quick = true;
                            plan.agents = true;
                        }
                        Action::Button { name, way, .. } if *way != ButtonWay::Up => {
                            if name.starts_with("切换角色-") {
                                plan.agents = true;
                            }
                            if name.starts_with("连携技-") {
                                plan.chain = true;
                                plan.agents = true;
                            }
                        }
                        _ => {}
                    }
                }
                actions(plan, &handler.children);
            }
        }
        for scene in &script.scenes {
            actions(&mut plan, &scene.handlers);
        }
        // 支援 / 连携头像复用已确认队伍作为优先候选，不能裁掉其发现依赖。
        plan.agents |= plan.quick || plan.chain;
        plan
    }

    pub fn dodge_only(&self) -> bool {
        !(self.agents || self.quick || self.chain || self.target || self.backup)
    }
    pub fn flash_enabled(&self) -> bool {
        self.flash
    }
}

#[cfg(test)]
#[path = "combat_plan_tests.rs"]
mod tests;
