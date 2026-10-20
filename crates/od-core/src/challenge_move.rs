//! 代行挑战的战前移动。距离与画面都来自新帧，暂停后不重放上一段位移。
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MoveStage {
    Approach,
    Interact,
    Buff,
    Advance,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MoveAction {
    Forward { ms: u32, measured: bool },
    Search { key: i32 },
    Turn { dx: i32 },
    Interact,
    Buff { rect: [i32; 4] },
}

#[derive(Debug, Clone, Default)]
pub struct MoveObservation {
    pub attack: bool,
    pub interact: bool,
    pub boss: bool,
    pub target_x: Option<f64>,
    pub distance: Option<f64>,
    pub choices: Vec<[i32; 4]>,
    pub fresh: bool,
}

pub fn parse_distance(text: &str) -> Option<f64> {
    let normalized = text.trim().replace(['Ｍ', 'ｍ', 'M'], "m");
    let number = normalized.strip_suffix('m')?.trim();
    if number.is_empty() || number.chars().any(|c| !c.is_ascii_digit() && c != '.') {
        return None;
    }
    number
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n > 0.0 && *n <= 9999.0)
}

pub fn turn_distance(x: f64) -> Option<i32> {
    if !x.is_finite() || !(0.0..1920.0).contains(&x) {
        return None;
    }
    Some(if x < 760.0 {
        -100
    } else if x < 860.0 {
        -50
    } else if x < 910.0 {
        -25
    } else if x > 1160.0 {
        100
    } else if x > 1060.0 {
        50
    } else if x > 1010.0 {
        25
    } else {
        0
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChallengeMove {
    pub expert: bool,
    pub buff_index: u8,
    pub stage: MoveStage,
    pub moves: u8,
    pub blind_moves: u8,
    pub turns: u8,
    pub interactions: u8,
    pub search_steps: u8,
    pub interaction_attempted: bool,
    pub buff_attempted: bool,
    pub expert_move_attempted: bool,
    pub expert_remaining_ms: u32,
    pub last_distance: Option<f64>,
    pub boss_frames: u8,
    pub pending: Option<MoveAction>,
}
impl ChallengeMove {
    pub fn new(expert: bool, buff_index: u8) -> Result<Self, &'static str> {
        let result = Self {
            expert,
            buff_index,
            stage: MoveStage::Approach,
            moves: 0,
            blind_moves: 0,
            turns: 0,
            interactions: 0,
            search_steps: 0,
            interaction_attempted: false,
            buff_attempted: false,
            expert_move_attempted: false,
            expert_remaining_ms: if expert { 1500 } else { 0 },
            last_distance: None,
            boss_frames: 0,
            pending: None,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=3).contains(&self.buff_index)
            || self.moves > 10
            || self.blind_moves > 10
            || self.turns > 32
            || self.interactions > 10
            || self.search_steps > 18
            || self.boss_frames > 2
            || (self.buff_attempted && !self.interaction_attempted)
            || (matches!(
                self.stage,
                MoveStage::Interact | MoveStage::Buff | MoveStage::Advance
            ) && !self.interaction_attempted)
            || (self.expert
                && (self.interaction_attempted
                    || self.buff_attempted
                    || !matches!(self.stage, MoveStage::Approach | MoveStage::Done)))
            || (self.expert_move_attempted && !self.expert)
            || self.expert_remaining_ms > 1500
            || self
                .last_distance
                .is_some_and(|n| !n.is_finite() || n <= 0.0 || n > 9999.0)
            || (!self.expert && self.expert_remaining_ms != 0)
            || (self.expert && self.expert_remaining_ms < 1500 && !self.expert_move_attempted)
            || (self.stage == MoveStage::Done
                && if self.expert {
                    self.expert_remaining_ms != 0
                } else {
                    self.boss_frames != 2
                })
            || (self.stage == MoveStage::Done && self.pending.is_some())
        {
            return Err("战前移动检查点无效");
        }
        if let Some(action) = &self.pending {
            match action {
                MoveAction::Forward { ms, .. } if !(1..=5000).contains(ms) => {
                    return Err("前进时长无效");
                }
                MoveAction::Search { key } if ![87, 83, 65, 68].contains(key) => {
                    return Err("搜索方向无效");
                }
                MoveAction::Turn { dx } if ![-100, -50, -25, 25, 50, 100].contains(dx) => {
                    return Err("提示点转向无效");
                }
                MoveAction::Buff { rect } if !crate::intel_board::valid_rect(*rect) => {
                    return Err("鸣徽选择坐标无效");
                }
                _ => {}
            }
        }
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.stage == MoveStage::Done {
            return Ok(());
        }
        // 专业挑战只继续未执行的持有时间；恶名狩猎重新识别距离和朝向。
        self.pending = None;
        self.boss_frames = 0;
        self.validate()
    }
    pub fn next(
        &mut self,
        observation: &MoveObservation,
    ) -> Result<Option<MoveAction>, &'static str> {
        let mut next = self.clone();
        let action = next.next_inner(observation)?;
        *self = next;
        Ok(action)
    }
    fn next_inner(&mut self, o: &MoveObservation) -> Result<Option<MoveAction>, &'static str> {
        self.validate()?;
        if self.stage == MoveStage::Done {
            return Ok(None);
        }
        if !o.fresh {
            self.boss_frames = 0;
            return Ok(None);
        }
        if self.pending.is_some() {
            return Err("上一步战前输入尚未收尾");
        }
        if o.target_x.is_some_and(|x| turn_distance(x).is_none())
            || o.distance
                .is_some_and(|n| !n.is_finite() || n <= 0.0 || n > 9999.0)
            || o.choices.len() > 3
            || o.choices
                .iter()
                .any(|&rect| !crate::intel_board::valid_rect(rect))
        {
            return Err("战前提示坐标或距离无效");
        }
        if self.expert {
            if !o.attack && !o.interact && !o.boss {
                return Ok(None);
            }
            let action = MoveAction::Forward {
                ms: self.expert_remaining_ms,
                measured: false,
            };
            self.pending = Some(action.clone());
            return Ok(Some(action));
        }
        if o.distance.is_some() {
            self.last_distance = o.distance;
        }
        if !o.choices.is_empty() {
            if !self.interaction_attempted {
                return Err("没有交互上下文，不能选择鸣徽");
            }
            self.stage = MoveStage::Buff;
            let mut choices = o.choices.clone();
            choices.sort_by_key(|rect| rect[0]);
            for pair in choices.windows(2) {
                if pair[0][0] + pair[0][2] > pair[1][0] {
                    return Err("鸣徽选择按钮重叠，未点击");
                }
            }
            let index = if usize::from(self.buff_index) > choices.len() {
                0
            } else {
                usize::from(self.buff_index) - 1
            };
            self.buff_attempted = true;
            let action = MoveAction::Buff {
                rect: choices[index],
            };
            self.pending = Some(action.clone());
            return Ok(Some(action));
        }
        if o.interact {
            self.boss_frames = 0;
            if self.interactions >= 10 {
                return Err("战前交互达到十次上限");
            }
            self.interaction_attempted = true;
            self.pending = Some(MoveAction::Interact);
            return Ok(self.pending.clone());
        }
        if o.boss {
            self.boss_frames += 1;
            if self.boss_frames >= 2 {
                self.stage = MoveStage::Done;
            }
            return Ok(None);
        }
        self.boss_frames = 0;
        if self.stage == MoveStage::Interact {
            self.stage = MoveStage::Buff;
        }
        if self.stage == MoveStage::Buff {
            if o.attack {
                self.stage = MoveStage::Advance;
            } else {
                return Ok(None);
            }
        }
        // 黑屏、菜单等未知状态没有移动权限；提示或战斗 HUD 仍须来自本帧。
        if !o.attack && o.target_x.is_none() {
            return Ok(None);
        }
        let action = if let Some(x) = o.target_x {
            let dx = turn_distance(x).ok_or("距离提示位置无效")?;
            if dx != 0 {
                if self.turns >= 32 {
                    return Err("战前转向达到三十二次上限");
                }
                MoveAction::Turn { dx }
            } else {
                let Some(distance) = o.distance else {
                    return Ok(None);
                };
                if self.moves >= 10 {
                    return Err("战前前移达到十次上限");
                }
                MoveAction::Forward {
                    ms: ((distance / 7.2 * 1000.0).ceil() as u32).clamp(1, 5000),
                    measured: true,
                }
            }
        } else if self.stage == MoveStage::Advance {
            if self.search_steps >= 18 {
                return Err("鸣徽后搜索达到十八次上限");
            }
            const DIRECTIONS: [i32; 18] = [
                65, 68, 68, 65, 87, 83, 83, 87, 65, 87, 68, 68, 83, 83, 65, 65, 87, 68,
            ];
            MoveAction::Search {
                key: DIRECTIONS[usize::from(self.search_steps)],
            }
        } else {
            if self.blind_moves >= 10 {
                return Err("没有距离提示的前移达到十次上限");
            }
            MoveAction::Forward {
                ms: 1000,
                measured: false,
            }
        };
        self.pending = Some(action.clone());
        Ok(Some(action))
    }
    pub fn acknowledge(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        let action = self.pending.take().ok_or("没有可确认的战前输入")?;
        match action {
            MoveAction::Forward { .. } if self.expert => {
                self.expert_remaining_ms = 0;
                self.expert_move_attempted = true;
                self.stage = MoveStage::Done;
            }
            MoveAction::Forward { measured, .. } => {
                if measured {
                    self.moves += 1;
                } else {
                    self.blind_moves += 1;
                }
            }
            MoveAction::Search { .. } => self.search_steps += 1,
            MoveAction::Turn { .. } => self.turns += 1,
            MoveAction::Interact => {
                self.interactions += 1;
                self.stage = MoveStage::Interact;
            }
            MoveAction::Buff { .. } => {
                self.stage = MoveStage::Advance;
                self.search_steps = 0;
            }
        }
        self.validate()
    }
    pub fn record_expert_hold(&mut self, elapsed_ms: u32) -> Result<(), &'static str> {
        self.validate()?;
        if !self.expert || !matches!(self.pending, Some(MoveAction::Forward { .. })) {
            return Err("没有专业挑战前移上下文");
        }
        if elapsed_ms > 0 {
            self.expert_move_attempted = true;
        }
        self.expert_remaining_ms = self.expert_remaining_ms.saturating_sub(elapsed_ms);
        self.pending = None;
        if self.expert_remaining_ms == 0 {
            self.stage = MoveStage::Done;
        }
        self.validate()
    }
}
