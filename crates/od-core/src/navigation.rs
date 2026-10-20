//! 小地图坐标为正右 0°、逆时针增加。绝对转向只使用本次截图的反馈。
use serde::{Deserialize, Serialize};

pub const HEADING_TOLERANCE: f64 = 2.0;
pub const MAX_TURN_COMMANDS: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnCalibration {
    /// 负值表示鼠标向右移动使小地图角度减少；旧版默认灵敏度的经验值。
    pub pixels_per_degree: f64,
}

impl Default for TurnCalibration {
    fn default() -> Self {
        Self {
            pixels_per_degree: -5.5,
        }
    }
}

impl TurnCalibration {
    pub fn validate(self) -> Result<(), &'static str> {
        if !self.pixels_per_degree.is_finite() || !(-50.0..=-0.1).contains(&self.pixels_per_degree)
        {
            return Err("转向标定须为 -50..=-0.1 像素/度的有限值");
        }
        Ok(())
    }
}

pub fn angle_delta(from: f64, to: f64) -> f64 {
    let delta = (to - from).rem_euclid(360.0);
    if delta > 180.0 { delta - 360.0 } else { delta }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TurnDecision {
    /// 第一次进入容差；需下一张新截图再次确认。
    Observe,
    Aligned,
    Move {
        dx: i32,
        degrees: f64,
    },
}

/// 生命周期限定为一次连续转向；暂停后重建，不能重放上次鼠标位移。
pub struct AbsoluteTurn {
    target: f64,
    calibration: TurnCalibration,
    scale: f64,
    pending: Option<(f64, f64)>,
    stable: bool,
    commands: u32,
}

impl AbsoluteTurn {
    pub fn new(target: f64, calibration: TurnCalibration) -> Result<Self, &'static str> {
        calibration.validate()?;
        if !target.is_finite() || !(0.0..360.0).contains(&target) {
            return Err("目标朝向须为 0..360° 的有限值（不含 360）");
        }
        Ok(Self {
            target,
            calibration,
            scale: 1.0,
            pending: None,
            stable: false,
            commands: 0,
        })
    }

    pub fn commands(&self) -> u32 {
        self.commands
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// 丢帧、画面不明或过期帧打断连续确认，也不用于学习鼠标补偿。
    pub fn invalidate_observation(&mut self) {
        self.pending = None;
        self.stable = false;
    }

    pub fn observe(&mut self, angle: f64) -> Result<TurnDecision, &'static str> {
        if !angle.is_finite() || !(0.0..360.0).contains(&angle) {
            self.invalidate_observation();
            return Err("当前朝向须为 0..360° 的有限值（不含 360）");
        }
        if let Some((source, command)) = self.pending.take() {
            let mut actual = angle_delta(source, angle);
            // 接近半圈时，一次过冲可能跨过最短角差的符号边界。
            if actual * command < 0.0 && actual.abs() >= 150.0 && command.abs() >= 150.0 {
                actual += 360.0 * command.signum();
            }
            if actual.abs() > 0.01 && actual * command > 0.0 {
                self.scale =
                    (self.scale + (command / actual - self.scale).clamp(-0.1, 0.1)).clamp(0.5, 2.0);
            }
        }
        let delta = angle_delta(angle, self.target);
        if delta.abs() <= HEADING_TOLERANCE {
            let decision = if self.stable {
                TurnDecision::Aligned
            } else {
                TurnDecision::Observe
            };
            self.stable = true;
            return Ok(decision);
        }
        self.stable = false;
        if self.commands >= MAX_TURN_COMMANDS {
            return Err("绝对转向未收敛，已停止追加鼠标输入");
        }
        // 单次不超过半圈，下一次截图反馈继续校正。
        let effective = (delta * self.scale).clamp(-180.0, 180.0);
        let dx = (effective * self.calibration.pixels_per_degree).round() as i32;
        if dx == 0 {
            return Err("转向标定导致零位移，不能校正朝向");
        }
        // 学习实际投递的整数像素对应角度，避免舍入误差累积。
        self.pending = Some((angle, dx as f64 / self.calibration.pixels_per_degree));
        self.commands += 1;
        Ok(TurnDecision::Move {
            dx,
            degrees: effective,
        })
    }
}
