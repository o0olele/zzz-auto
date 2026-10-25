//! 区域探索检查点与拖图扫描顺序。屏幕坐标不跨暂停复用。
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const APP_ID: &str = "map_exploration";
pub const TIMEOUT_MS: u32 = 1_800_000;

/// 最小缩放下相邻窗口保留重叠；完整扫描一轮最多 49 个位置。
pub const SWEEP_STEP: [i32; 2] = [520, 360];
#[derive(Debug, Clone)]
pub struct MapSweep {
    pub index: usize,
    pub offset: [i32; 2],
    views: Vec<[i32; 2]>,
}
impl Default for MapSweep {
    fn default() -> Self {
        let mut views = vec![];
        let mut point: [i32; 2] = [0, 0];
        let mut length = 1;
        let directions = [[1, 0], [0, 1], [-1, 0], [0, -1]];
        let mut direction = 0;
        while views.len() < 48 {
            for _ in 0..2 {
                let d = directions[direction % 4];
                for _ in 0..length {
                    point = [point[0] + d[0], point[1] + d[1]];
                    if point[0].abs() <= 3 && point[1].abs() <= 3 {
                        views.push(point);
                    }
                    if views.len() == 48 {
                        break;
                    }
                }
                direction += 1;
                if views.len() == 48 {
                    break;
                }
            }
            length += 1;
        }
        // 第一次识别前先平移，原始窗口在最后补扫。
        views.push([0, 0]);
        Self {
            index: 0,
            offset: [0, 0],
            views,
        }
    }
}
impl MapSweep {
    pub fn total(&self) -> usize {
        self.views.len()
    }
    pub fn drag(&self) -> Option<([i32; 2], [i32; 2])> {
        let view = self.views.get(self.index)?;
        let dx = (view[0] * SWEEP_STEP[0] - self.offset[0]).clamp(-SWEEP_STEP[0], SWEEP_STEP[0]);
        let dy = (view[1] * SWEEP_STEP[1] - self.offset[1]).clamp(-SWEEP_STEP[1], SWEEP_STEP[1]);
        ((dx, dy) != (0, 0)).then_some(([720, 540], [720 - dx, 540 - dy]))
    }
    pub fn applied(&mut self, start: [i32; 2], end: [i32; 2]) {
        self.offset[0] += start[0] - end[0];
        self.offset[1] += start[1] - end[1];
    }
    pub fn advance(&mut self) -> bool {
        self.index += 1;
        self.index < self.views.len()
    }
    pub fn restart(&mut self) {
        self.index = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ExplorationTarget {
    Observation,
    Cargo,
}
impl ExplorationTarget {
    pub fn name(self) -> &'static str {
        match self {
            Self::Observation => "观测奇点",
            Self::Cargo => "小卡格车",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ExplorationProgress {
    pub target: ExplorationTarget,
    pub observations: u16,
    pub cargo: u16,
    pub scan_finished: bool,
    pub interaction_attempted: bool,
}
impl Default for ExplorationProgress {
    fn default() -> Self {
        Self {
            target: ExplorationTarget::Observation,
            observations: 0,
            cargo: 0,
            scan_finished: false,
            interaction_attempted: false,
        }
    }
}
impl ExplorationProgress {
    pub fn exhausted(&mut self) {
        match self.target {
            ExplorationTarget::Observation => self.target = ExplorationTarget::Cargo,
            ExplorationTarget::Cargo => self.scan_finished = true,
        }
        self.interaction_attempted = false;
    }
    pub fn confirm(&mut self) -> Result<(), &'static str> {
        if !self.interaction_attempted || self.scan_finished {
            return Err("空洞探索尚无交互，不能确认解锁");
        }
        let count = match self.target {
            ExplorationTarget::Observation => &mut self.observations,
            ExplorationTarget::Cargo => &mut self.cargo,
        };
        *count = count.checked_add(1).ok_or("空洞探索目标数量超限")?;
        self.interaction_attempted = false;
        // 开箱可能揭示新的奇点，每次交互后重新检查奇点优先级。
        self.target = ExplorationTarget::Observation;
        Ok(())
    }
    pub fn check_update(&self, next: &Self) -> Result<(), &'static str> {
        if next.observations < self.observations
            || next.cargo < self.cargo
            || u32::from(next.observations - self.observations) + u32::from(next.cargo - self.cargo)
                > 1
            || (self.scan_finished && next != self)
        {
            return Err("空洞探索检查点回退或目标数量无效");
        }
        if (next.observations != self.observations || next.cargo != self.cargo)
            && (!self.interaction_attempted
                || (self.target == ExplorationTarget::Observation && next.cargo != self.cargo)
                || (self.target == ExplorationTarget::Cargo
                    && next.observations != self.observations))
        {
            return Err("空洞探索缺少交互检查点");
        }
        Ok(())
    }
}
