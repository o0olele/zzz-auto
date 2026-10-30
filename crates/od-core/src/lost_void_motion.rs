//! 固定镜头背景采样及有界脱困。像素样本不进入检查点，未知不等于受阻。
use crate::{InputMode, InputProbeAction, InputProbeConfig};
use serde::{Deserialize, Serialize};

/// 参考大世界 ROI，xywh；避开角色、名牌和右下角按键。
pub const MOTION_AREAS: [[usize; 4]; 4] = [
    [320, 320, 400, 200],
    [1200, 320, 400, 200],
    [320, 560, 400, 200],
    [1200, 560, 400, 200],
];
const WIDTH: usize = 96;
const HEIGHT: usize = 64;
pub const MOTION_SAMPLE_MS: u64 = 300;
pub const MOTION_CONFIRM_MS: u64 = 1500;
pub const MOTION_UNKNOWN_MS: u64 = 3900;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MotionResult {
    #[default]
    Unknown,
    Still,
    Moving,
}
#[derive(Debug, Default)]
pub struct MotionDetector {
    last_ms: Option<u64>,
    previous: Vec<Vec<u8>>,
    moving: u32,
    still: u32,
}
// OpenCV RGB2GRAY 系数；Maa 的输入是连续 CV_8UC3 BGR。
fn gray(bgr: &[u8], x: usize, y: usize) -> u32 {
    let i = (y * 1920 + x) * 3;
    (1868 * u32::from(bgr[i]) + 9617 * u32::from(bgr[i + 1]) + 4899 * u32::from(bgr[i + 2]) + 8192)
        >> 14
}
fn region(bgr: &[u8], [left, top, w, h]: [usize; 4]) -> Vec<u8> {
    let mut small = vec![0u8; WIDTH * HEIGHT];
    // 整数面积权重，处理 400/96、200/64 的非整数缩放，不能用最近邻代替。
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (x1, x2) = (x * w, (x + 1) * w);
            let (y1, y2) = (y * h, (y + 1) * h);
            let mut sum = 0u64;
            for sy in y1 / HEIGHT..y2.div_ceil(HEIGHT) {
                let wy = y2.min((sy + 1) * HEIGHT) - y1.max(sy * HEIGHT);
                for sx in x1 / WIDTH..x2.div_ceil(WIDTH) {
                    let wx = x2.min((sx + 1) * WIDTH) - x1.max(sx * WIDTH);
                    sum += u64::from(gray(bgr, left + sx, top + sy)) * (wx * wy) as u64;
                }
            }
            small[y * WIDTH + x] = ((sum + (w * h / 2) as u64) / (w * h) as u64) as u8;
        }
    }
    let reflect = |i: isize, size: usize| {
        if i < 0 {
            (-i) as usize
        } else if i as usize >= size {
            2 * size - i as usize - 2
        } else {
            i as usize
        }
    };
    let mut blurred = vec![0; small.len()];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let mut sum = 0u32;
            for (dy, wy) in [(-1, 1), (0, 2), (1, 1)] {
                for (dx, wx) in [(-1, 1), (0, 2), (1, 1)] {
                    sum += u32::from(
                        small[reflect(y as isize + dy, HEIGHT) * WIDTH
                            + reflect(x as isize + dx, WIDTH)],
                    ) * wx
                        * wy;
                }
            }
            blurred[y * WIDTH + x] = ((sum + 8) / 16) as u8;
        }
    }
    blurred
}
fn textured(pixels: &[u8]) -> bool {
    let mean = pixels.iter().map(|p| f64::from(*p)).sum::<f64>() / pixels.len() as f64;
    pixels
        .iter()
        .map(|p| (f64::from(*p) - mean).powi(2))
        .sum::<f64>()
        / pixels.len() as f64
        >= 9.0
}
impl MotionDetector {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn counts(&self) -> (u32, u32) {
        (self.moving, self.still)
    }
    pub fn result(&self) -> MotionResult {
        let n = u64::from(self.moving) + u64::from(self.still);
        if n < 2 {
            MotionResult::Unknown
        } else if u64::from(self.moving) * 3 >= n * 2 {
            MotionResult::Moving
        } else if u64::from(self.still) * 3 >= n * 2 {
            MotionResult::Still
        } else {
            MotionResult::Unknown
        }
    }
    pub fn observe_bgr(
        &mut self,
        bgr: &[u8],
        sample_ms: u64,
    ) -> Result<MotionResult, &'static str> {
        if bgr.len() != 1920 * 1080 * 3 {
            self.reset();
            return Err("背景运动截图必须是连续 1080p BGR");
        }
        if let Some(last) = self.last_ms {
            if sample_ms < last || sample_ms - last > 2400 {
                self.reset();
            } else if sample_ms - last < MOTION_SAMPLE_MS {
                return Ok(self.result());
            }
        }
        let current: Vec<_> = MOTION_AREAS.into_iter().map(|r| region(bgr, r)).collect();
        let (mut valid, mut moving) = (0, 0);
        for (a, b) in self.previous.iter().zip(&current) {
            if !textured(a) || !textured(b) {
                continue;
            }
            valid += 1;
            let mut differences: Vec<i16> = a
                .iter()
                .zip(b)
                .map(|(a, b)| i16::from(*b) - i16::from(*a))
                .collect();
            differences.sort_unstable();
            let median_twice =
                differences[differences.len() / 2 - 1] + differences[differences.len() / 2];
            let changed = differences
                .iter()
                .filter(|d| (2 * **d - median_twice).abs() > 20)
                .count();
            if changed * 100 >= differences.len() * 8 {
                moving += 1;
            }
        }
        if valid >= 2 {
            if moving * 2 >= valid {
                self.moving = self.moving.saturating_add(1);
            } else {
                self.still = self.still.saturating_add(1);
            }
        }
        self.last_ms = Some(sample_ms);
        self.previous = current;
        Ok(self.result())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionDirection {
    Forward,
    Left,
    Right,
    BackLeft,
    BackRight,
    Back,
}
impl MotionDirection {
    pub fn keys(self) -> &'static [i32] {
        match self {
            Self::Forward => &[87],
            Self::Left => &[65],
            Self::Right => &[68],
            Self::BackLeft => &[83, 65],
            Self::BackRight => &[83, 68],
            Self::Back => &[83],
        }
    }
    fn index(self) -> Option<usize> {
        match self {
            Self::Forward => None,
            Self::Left => Some(0),
            Self::Right => Some(1),
            Self::BackLeft => Some(2),
            Self::BackRight => Some(3),
            Self::Back => Some(4),
        }
    }
}
const DIRECTIONS: [MotionDirection; 5] = [
    MotionDirection::Left,
    MotionDirection::Right,
    MotionDirection::BackLeft,
    MotionDirection::BackRight,
    MotionDirection::Back,
];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MotionAction {
    Probe { direction: MotionDirection },
    ClearObstacle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Phase {
    #[default]
    Normal,
    Start,
    AfterAttack,
    Side {
        direction: MotionDirection,
        steps: u8,
    },
    Forward {
        direction: MotionDirection,
        steps: u8,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MotionRecovery {
    phase: Phase,
    attempts: u8,
    failures: [u8; 5],
    last: Option<MotionDirection>,
    attack_attempted: bool,
    attack_input: Option<InputProbeAction>,
    forward_ms: u32,
    searches: u8,
    camera_distance: u16,
}
impl MotionRecovery {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.attempts > 13
            || self.failures.iter().any(|n| *n > 12)
            || self.forward_ms > 3000
            || self.searches > 10
            || self.camera_distance >= 200
            || self.last == Some(MotionDirection::Forward)
            || matches!(self.phase, Phase::Side{direction,steps}|Phase::Forward{direction,steps} if direction==MotionDirection::Forward || !(1..=3).contains(&steps))
            || self.phase != Phase::Normal && self.attempts == 0
            || self.phase == Phase::AfterAttack && !self.attack_attempted
        {
            return Err("背景运动脱困状态无效");
        }
        if let Some(action) = self.attack_input {
            if !matches!(
                action,
                InputProbeAction::Key { .. } | InputProbeAction::MouseButton { .. }
            ) {
                return Err("清障攻击必须是冻结的键鼠绑定");
            }
            if matches!(action, InputProbeAction::Key { key } if [87,65,83,68,70,18,27].contains(&key))
            {
                return Err("清障攻击不能占用导航或页面控制按键");
            }
            InputProbeConfig {
                mode: InputMode::Foreground,
                action,
                hold_ms: 200,
                start_delay_ms: 0,
            }
            .validate()?;
        }
        Ok(())
    }
    pub fn with_attack(mut self, input: InputProbeAction) -> Result<Self, &'static str> {
        self.attack_input = Some(input);
        self.validate()?;
        Ok(self)
    }
    pub fn attack_input(&self) -> Option<InputProbeAction> {
        self.attack_input
    }
    pub fn attempts(&self) -> u8 {
        self.attempts
    }
    pub fn recovering(&self) -> bool {
        self.phase != Phase::Normal
    }
    pub fn attack_attempted(&self) -> bool {
        self.attack_attempted
    }
    pub fn failures(&self) -> [u8; 5] {
        self.failures
    }
    /// 暂停期间镜头可能改变；保留受阻次数和攻击意图，撤销旧相对方向。
    pub fn resume(&mut self) {
        self.failures = [0; 5];
        self.last = None;
        self.camera_distance = 0;
        if self.recovering() {
            self.phase = Phase::Start;
        }
    }
    fn choose(&self) -> MotionDirection {
        *DIRECTIONS
            .iter()
            .min_by_key(|d| {
                (
                    self.failures[d.index().unwrap()],
                    Some(**d) == self.last,
                    d.index().unwrap(),
                )
            })
            .unwrap()
    }
    pub fn on_forward(&mut self, ms: u32) {
        self.forward_ms = self.forward_ms.saturating_add(ms).min(3000);
        self.searches = 0;
    }
    pub fn on_turn(&mut self, dx: i32, searching: bool) {
        let distance = u32::from(self.camera_distance).saturating_add(dx.unsigned_abs());
        if distance >= 200 {
            self.failures = [0; 5];
            self.last = None;
            self.camera_distance = 0;
        } else {
            self.camera_distance = distance as u16;
        }
        if searching {
            self.searches = (self.searches + 1).min(10);
            if self.searches == 10 && self.phase == Phase::Normal {
                self.begin_escape();
            }
        }
    }
    fn begin_escape(&mut self) {
        self.attempts = self.attempts.saturating_add(1).min(13);
        self.phase = Phase::Start;
        self.forward_ms = 0;
        self.searches = 0;
    }
    pub fn action(&self, attack_available: bool) -> Result<Option<MotionAction>, &'static str> {
        self.validate()?;
        if self.attempts > 12 {
            return Err("迷失之地已达脱困次数上限");
        }
        Ok(match self.phase {
            Phase::Normal if self.forward_ms >= 3000 => Some(MotionAction::Probe {
                direction: MotionDirection::Forward,
            }),
            Phase::Normal => None,
            Phase::Start
                if !self.attack_attempted && self.attack_input.is_some() && attack_available =>
            {
                Some(MotionAction::ClearObstacle)
            }
            Phase::Start => Some(MotionAction::Probe {
                direction: self.choose(),
            }),
            Phase::AfterAttack | Phase::Forward { .. } => Some(MotionAction::Probe {
                direction: MotionDirection::Forward,
            }),
            Phase::Side { direction, .. } => Some(MotionAction::Probe { direction }),
        })
    }
    /// 已提交或未知投递都只尝试一次。明确未投递不调用。
    pub fn clear_submitted(&mut self) -> Result<(), &'static str> {
        if self.action(true)? != Some(MotionAction::ClearObstacle) {
            return Err("没有本次清障攻击意图");
        }
        self.attack_attempted = true;
        self.phase = Phase::AfterAttack;
        Ok(())
    }
    fn fail_direction(&mut self, direction: MotionDirection) {
        let i = direction.index().unwrap();
        self.failures[i] = self.failures[i].saturating_add(1).min(12);
        self.last = Some(direction);
        self.attempts = self.attempts.saturating_add(1).min(13);
        self.phase = Phase::Side {
            direction: self.choose(),
            steps: 1,
        };
    }
    fn finish_escape(&mut self) {
        self.phase = Phase::Normal;
        self.forward_ms = 0;
        self.searches = 0;
    }
    pub fn observed(
        &mut self,
        direction: MotionDirection,
        result: MotionResult,
    ) -> Result<(), &'static str> {
        self.validate()?;
        if self.action(false)? != Some(MotionAction::Probe { direction }) {
            return Err("背景采样方向与脱困阶段不一致");
        }
        match self.phase {
            Phase::Normal => {
                self.forward_ms = 0;
                match result {
                    MotionResult::Still => self.begin_escape(),
                    MotionResult::Moving => {
                        let input = self.attack_input;
                        *self = Self::default();
                        self.attack_input = input;
                    }
                    MotionResult::Unknown => (),
                }
            }
            Phase::AfterAttack if result == MotionResult::Moving => self.finish_escape(),
            Phase::AfterAttack => {
                self.phase = Phase::Side {
                    direction: self.choose(),
                    steps: 1,
                }
            }
            Phase::Start | Phase::Side { .. } => {
                let steps = match self.phase {
                    Phase::Side { steps, .. } => steps,
                    _ => 1,
                };
                self.last = Some(direction);
                if result == MotionResult::Moving {
                    self.phase = Phase::Forward { direction, steps };
                } else {
                    self.fail_direction(direction);
                }
            }
            Phase::Forward { .. } if result == MotionResult::Moving => self.finish_escape(),
            Phase::Forward { direction, steps } if result == MotionResult::Still && steps < 3 => {
                self.phase = Phase::Side {
                    direction,
                    steps: steps + 1,
                }
            }
            Phase::Forward { direction, .. } => self.fail_direction(direction),
        }
        self.validate()
    }
}
