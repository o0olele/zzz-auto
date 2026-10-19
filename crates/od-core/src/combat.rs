//! 战斗公开配置与有界运行快照；输入映射独立于脚本内容。
use crate::{InputMode, InputProbeAction, InputProbeConfig, InputSide, MouseButton, RunState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

pub const BUTTONS: [&str; 16] = ["闪避", "切换角色-下一个", "切换角色-上一个", "切换后援", "普通攻击", "特殊攻击", "终结技", "连携技-左", "连携技-右", "连携技-取消", "移动-前", "移动-后", "移动-左", "移动-右", "锁定敌人", "快速支援"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct CombatConfig {
    pub script: String,
    pub dodge_script:String,
    pub use_merged:bool,
    pub auto_ultimate:bool,
    pub frame_interval_ms:u32,
    pub directml_device:Option<i32>,
    pub mode: InputMode,
    pub start_delay_ms: u32,
    pub timeout_ms: u32,
    pub max_frame_age_ms: u32,
    pub tap_ms: u32,
    pub audio: bool,
    pub audio_threshold_milli: u32,
    pub dodge_only: bool,
    /// 每 1000 单位旧版 turn_by_distance 对应的相对位移；首版默认 1:1。
    pub turn_sensitivity_milli: u32,
    pub gamepad_turn_speed: i32,
    pub bindings: BTreeMap<String, Vec<InputProbeAction>>,
}
impl Default for CombatConfig {
    fn default() -> Self {
        Self { script: "全配队通用".into(), dodge_script:"闪避".into(),use_merged:true,auto_ultimate:false,frame_interval_ms:20,directml_device:None, mode: InputMode::Foreground, start_delay_ms: 3000, timeout_ms: 3_600_000, max_frame_age_ms: 250, tap_ms: 20, audio: false, audio_threshold_milli: 100, dodge_only: true, turn_sensitivity_milli: 1000, gamepad_turn_speed: 16000, bindings: BTreeMap::new() }
    }
}
impl CombatConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        for name in [&self.script,&self.dodge_script] {if name.is_empty() || name.len() > 240 || name.contains(['/', '\\', ':']) || matches!(name.as_str(), "." | "..") { return Err("战斗脚本名称无效"); }}
        if !(1..=1000).contains(&self.frame_interval_ms) {return Err("截图间隔超出 1..1000ms");}
        if self.directml_device.is_some_and(|device|!(0..=15).contains(&device)) {return Err("DirectML 设备编号超出 0..15");}
        if self.start_delay_ms > 10000 || self.timeout_ms == 0 || self.timeout_ms > 21_600_000 || !(20..=1000).contains(&self.max_frame_age_ms) || !(1..=1000).contains(&self.tap_ms) || self.audio_threshold_milli > 10000 || !(1..=100000).contains(&self.turn_sensitivity_milli) || !(1..=32767).contains(&self.gamepad_turn_speed) { return Err("战斗时长、帧龄、声音阈值或转向参数超出范围"); }
        for (name, actions) in &self.bindings {
            if !BUTTONS.contains(&name.as_str()) || actions.is_empty() || actions.len() > 8 { return Err("战斗按键映射无效"); }
            for action in actions { InputProbeConfig { mode: self.mode, action: *action, hold_ms: 1, start_delay_ms: 0 }.validate()?; }
        }
        Ok(())
    }
    pub fn binding(&self, name: &str) -> Result<Vec<InputProbeAction>, &'static str> {
        if let Some(value) = self.bindings.get(name) { return Ok(value.clone()); }
        let index = BUTTONS.iter().position(|n| *n == name).ok_or("未知战斗按键")?;
        if self.mode.is_gamepad() {
            if (10..=13).contains(&index) {
                let (x, y) = [(0, 32767), (0, -32768), (-32768, 0), (32767, 0)][index - 10];
                return Ok(vec![InputProbeAction::Stick { stick: InputSide::Left, x, y }]);
            }
            if index == 6 { return Ok(vec![InputProbeAction::Trigger { trigger: InputSide::Right, value: 255 }]); }
            // Maa 的 Xbox 与 DS4 控制器都接收 XUSB 掩码，DS4 的转换由 SDK 完成。
            let key = [4096, 512, 256, 8192, 16384, 32768, 0, 256, 512, 4096, 0, 0, 0, 0, 128, 512][index];
            return Ok(vec![InputProbeAction::Key { key }]);
        }
        if index == 4 || index == 9 || index == 14 { return Ok(vec![InputProbeAction::MouseButton { button: if index == 4 { MouseButton::Left } else { MouseButton::Middle }, x: 960, y: 540 }]); }
        Ok(vec![InputProbeAction::Key { key: [16, 32, 67, 82, 0, 69, 81, 81, 69, 0, 87, 83, 65, 68, 0, 32][index] }])
    }
    pub fn recovery_inputs(&self) -> Result<Vec<InputProbeConfig>, &'static str> {
        self.validate()?;
        let mut actions = Vec::new();
        for name in BUTTONS { for action in self.binding(name)? { if !actions.contains(&action) { actions.push(action); } } }
        if self.mode.is_gamepad() { actions.push(InputProbeAction::Stick { stick: InputSide::Right, x: self.gamepad_turn_speed, y: 0 }); }
        let result: Vec<_> = actions.into_iter().map(|action| InputProbeConfig { mode: self.mode, action, hold_ms: 1, start_delay_ms: 0 }).collect();
        for input in &result { input.validate()?; }
        Ok(result)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CombatProgress {
    pub config: CombatConfig,
    pub state: RunState,
    pub inputs_released: bool,
    pub in_battle: bool,
    pub handler: String,
    pub action: String,
    #[ts(type = "number")]
    pub frames: u64,
    #[ts(type = "number")]
    pub stale_frames: u64,
    #[ts(type = "number")]
    pub actions: u64,
    #[ts(type = "number")]
    pub preemptions: u64,
    pub frame_age_ms: u32,
    pub audio_correlation: Option<f64>,
    pub team: Vec<String>,
    pub states: BTreeMap<String, f64>,
}
impl CombatProgress {
    pub fn new(config: CombatConfig) -> Self {
        Self { config, state: RunState::Running, inputs_released: true, in_battle: false, handler: String::new(), action: String::new(), frames: 0, stale_frames: 0, actions: 0, preemptions: 0, frame_age_ms: 0, audio_correlation: None, team: Vec::new(), states: BTreeMap::new() }
    }
}

#[derive(Debug,Clone,Serialize,Deserialize,TS)]
pub struct CombatScriptCatalog {pub revision:String,pub imported:Option<String>,pub battle:Vec<String>,pub dodge:Vec<String>}

impl crate::RunSlot {
    pub fn begin_combat(&mut self, config: CombatConfig, context: crate::run_plan::RunContext) -> Result<crate::RunRecord, &'static str> {
        config.validate()?;
        context.validate()?;
        let app_id = if config.dodge_only { "dodge_assistant" } else { "auto_battle" };
        if context.app_ids != [app_id] || !context.standalone { return Err("战斗账号上下文无效"); }
        self.begin(app_id)?;
        let run = self.current.as_mut().ok_or("战斗运行缺失")?;
        run.context = Some(context);
        run.combat = Some(CombatProgress::new(config));
        Ok(run.clone())
    }
    pub fn update_combat(&mut self, id: uuid::Uuid, mut progress: CombatProgress, message: String) -> Result<crate::RunRecord, &'static str> {
        let run = self.current.as_mut().ok_or("战斗运行缺失")?;
        if run.id != id || run.state.is_terminal() { return Err("战斗运行 ID 已过期"); }
        let previous = run.combat.as_ref().ok_or("当前不是战斗任务")?;
        if previous.config != progress.config || previous.frames > progress.frames || previous.actions > progress.actions || previous.preemptions > progress.preemptions { return Err("战斗冻结配置或计数无效"); }
        if (progress.state == RunState::Paused || progress.state == RunState::Succeeded) && !progress.inputs_released { return Err("战斗输入尚未释放"); }
        if run.state == RunState::Stopping && progress.state == RunState::Succeeded { progress.state = RunState::Stopped; }
        run.state = if progress.state == RunState::Running { run.state } else if run.state == RunState::Stopping && progress.state == RunState::Paused { RunState::Stopping } else { progress.state };
        run.combat = Some(progress);
        run.message = message;
        if run.state.is_terminal() { run.finished_at = Some(chrono::Utc::now()); }
        Ok(run.clone())
    }
}
