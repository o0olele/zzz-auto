//! 参考角色数据转换后的显式识别定义；无 Python 动态加载。
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStateDef {
    pub state_name: String,
    pub check_way: String,
    pub template_id: String,
    pub lower_color: Option<serde_json::Value>,
    pub upper_color: Option<serde_json::Value>,
    pub hsv_color: Option<[f64; 3]>,
    pub hsv_color_diff: Option<[f64; 3]>,
    pub connect_cnt: Option<u32>,
    pub split_color_range: Option<[f64; 2]>,
    #[serde(default = "hundred")]
    pub max_length: u32,
    #[serde(default)]
    pub min_value_trigger_state: i64,
    pub template_threshold: Option<f64>,
    #[serde(default)]
    pub clear_on_zero: bool,
}
fn hundred() -> u32 {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub templates: Vec<String>,
    pub states: Vec<AgentStateDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub image: String,
    pub mask: Option<String>,
    pub rect: Option<[i32; 4]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisionSpec {
    pub schema: u32,
    pub agents: Vec<Agent>,
    pub common: BTreeMap<String, AgentStateDef>,
    pub areas: BTreeMap<String, [i32; 4]>,
    pub templates: BTreeMap<String, Template>,
}

impl VisionSpec {
    pub fn validate_script(&self, script: &crate::conditional::Script) -> anyhow::Result<()> {
        use crate::conditional::{Action, Handler};
        let mut names = std::collections::BTreeSet::new();
        for name in [
            "闪避识别-黄光",
            "闪避识别-红光",
            "闪避识别-声音",
            "连携技-准备",
            "目标-近距离锁定",
            "格挡-破碎",
            "切人-冷却",
            "连携技-1-邦布",
            "连携技-2-邦布",
        ] {
            names.insert(name.to_string());
        }
        for name in crate::combat::BUTTONS
            .iter()
            .map(|name| format!("按键-{name}"))
            .chain(
                [
                    "特殊攻击",
                    "终结技",
                    "连携技",
                    "快速支援",
                    "切换后援",
                    "普通攻击",
                ]
                .map(|name| format!("按键可用-{name}")),
            )
        {
            for suffix in ["", "-按下", "-松开"] {
                names.insert(format!("{name}{suffix}"));
            }
        }
        for agent in &self.agents {
            for prefix in [
                "前台-",
                "后台-1-",
                "后台-2-",
                "连携技-1-",
                "连携技-2-",
                "快速支援-",
                "切换角色-",
            ] {
                for suffix in [&agent.name, &agent.kind] {
                    names.insert(format!("{prefix}{suffix}"));
                }
            }
            names.insert(format!("后台-{}", agent.name));
            for suffix in ["能量", "特殊技可用", "终结技可用"] {
                names.insert(format!("{}-{suffix}", agent.name));
            }
            names.extend(agent.states.iter().map(|state| state.state_name.clone()));
        }
        names.extend(self.common.values().map(|state| state.state_name.clone()));
        for name in &script.states {
            anyhow::ensure!(
                name.starts_with("自定义-") || names.contains(name),
                "{}: 未知战斗状态 {name}",
                script.source
            );
        }
        fn handlers(spec: &VisionSpec, values: &[Handler]) -> anyhow::Result<()> {
            for handler in values {
                for action in &handler.actions {
                    if let Action::SwitchAgent { name } = action {
                        anyhow::ensure!(
                            spec.agents.iter().any(|agent| &agent.name == name),
                            "{}: 未知切换角色 {name}",
                            handler.source
                        );
                    }
                }
                handlers(spec, &handler.children)?;
            }
            Ok(())
        }
        for scene in &script.scenes {
            handlers(self, &scene.handlers)?;
        }
        Ok(())
    }
}
