//! 情报板的冻结配置、游戏周与单次委托结果。界面返回本身不能证明战斗成功。
use crate::{
    battle_result::NormalBattleResult,
    daily::{GameDay, GameRegion},
    predefined_teams::TeamConfig,
};
use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const EXP_TARGET: u32 = 5000;
pub const CONTRIBUTION_TARGET: u16 = 1000;
pub const MAX_RUN_COMMISSIONS: usize = 64;
pub const DETECTOR_ID: &str = "yolov26n-736-lost-void-det-20260630";
pub const DETECTOR_FILES: [&str; 2] = [
    "model/detect/yolov26n-736-lost-void-det-20260630.onnx",
    "model/detect/yolov26n-736-lost-void-det-20260630.labels.csv",
];

#[path = "intel_board_pipeline.rs"]
mod definition;
pub use definition::pipeline;
#[path = "intel_board_assets.rs"]
mod assets;
pub use assets::{source_images, templates};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommissionCandidate {
    pub kind: CommissionKind,
    /// Maa 的全帧 [x, y, width, height]，只作为当前帧候选，不能跨步骤缓存点击。
    pub rect: [i32; 4],
}

pub fn valid_rect([x, y, width, height]: [i32; 4]) -> bool {
    x >= 0
        && y >= 0
        && width > 0
        && height > 0
        && x.checked_add(width).is_some_and(|right| right <= 1920)
        && y.checked_add(height).is_some_and(|bottom| bottom <= 1080)
}

/// 与旧版相同，先排除与本人帖子星标横向重叠（向右扩展 30px）的候选，
/// 再按 y、x 选择第一项。重叠的不同类型不能决定应执行哪种战前移动。
pub fn choose_commission(
    candidates: &[CommissionCandidate],
    stars: &[[i32; 4]],
) -> Result<Option<CommissionCandidate>, &'static str> {
    if candidates.len() > 128
        || stars.len() > 128
        || candidates
            .iter()
            .any(|candidate| !valid_rect(candidate.rect))
        || stars.iter().any(|&rect| !valid_rect(rect))
    {
        return Err("情报板候选或本人帖子标记无效");
    }
    let mut candidates: Vec<_> = candidates
        .iter()
        .filter(|candidate| {
            let [x, _, width, _] = candidate.rect;
            !stars.iter().any(|&[star_x, _, star_width, _]| {
                x < star_x + star_width + 30 && star_x < x + width
            })
        })
        .cloned()
        .collect();
    candidates.sort_by_key(|candidate| (candidate.rect[1], candidate.rect[0]));
    candidates.dedup();
    let Some(first) = candidates.first() else {
        return Ok(None);
    };
    let [x, y, width, height] = first.rect;
    if candidates.iter().skip(1).any(|other| {
        let [other_x, other_y, other_width, other_height] = other.rect;
        first.kind != other.kind
            && x < other_x + other_width
            && other_x < x + width
            && y < other_y + other_height
            && other_y < y + height
    }) {
        return Err("情报板首个委托类型有歧义，未选择");
    }
    Ok(Some(first.clone()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct IntelBoardConfig {
    pub predefined_team_idx: i8,
    pub auto_battle_config: String,
    pub exp_grind_mode: bool,
}
impl Default for IntelBoardConfig {
    fn default() -> Self {
        Self {
            predefined_team_idx: -1,
            auto_battle_config: "全配队通用".into(),
            exp_grind_mode: false,
        }
    }
}
impl IntelBoardConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(-1..20).contains(&self.predefined_team_idx) {
            return Err("情报板预备编队须为不选择或 1..20 队");
        }
        let name = &self.auto_battle_config;
        if name.is_empty()
            || name.trim() != name
            || name.len() > 240
            || name.chars().any(char::is_control)
            || name.contains(['/', '\\', ':'])
            || matches!(name.as_str(), "." | "..")
        {
            return Err("情报板战斗脚本名称无效");
        }
        Ok(())
    }
    pub fn script<'a>(&'a self, teams: &'a TeamConfig) -> Result<&'a str, &'static str> {
        self.validate()?;
        teams.validate()?;
        if self.predefined_team_idx == -1 {
            Ok(&self.auto_battle_config)
        } else {
            Ok(&teams.get(self.predefined_team_idx as u8)?.auto_battle)
        }
    }
}

pub fn import(value: &serde_json::Value) -> Result<IntelBoardConfig, &'static str> {
    let object = value.as_object().ok_or("旧情报板配置必须是对象")?;
    let mut config = IntelBoardConfig::default();
    if let Some(index) = object.get("predefined_team_idx") {
        config.predefined_team_idx = index
            .as_i64()
            .and_then(|index| i8::try_from(index).ok())
            .ok_or("旧情报板编队编号不是有效整数")?;
    }
    if let Some(name) = object.get("auto_battle_config") {
        config.auto_battle_config = name.as_str().ok_or("旧情报板脚本名称不是文本")?.into();
    }
    if let Some(mode) = object.get("exp_grind_mode") {
        config.exp_grind_mode = mode.as_bool().ok_or("旧情报板经验模式不是布尔值")?;
    }
    config.validate()?;
    Ok(config)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct IntelWeek {
    pub region: GameRegion,
    pub monday: String,
}
impl IntelWeek {
    pub fn from_day(day: &GameDay) -> Result<Self, &'static str> {
        day.validate()?;
        let date =
            NaiveDate::parse_from_str(&day.date, "%Y-%m-%d").map_err(|_| "情报板游戏日无效")?;
        let monday = date
            .checked_sub_signed(Duration::days(i64::from(
                date.weekday().num_days_from_monday(),
            )))
            .ok_or("情报板游戏周超出日期范围")?;
        Ok(Self {
            region: day.region,
            monday: monday.format("%Y-%m-%d").to_string(),
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let day = GameDay {
            region: self.region,
            date: self.monday.clone(),
        };
        if *self != Self::from_day(&day)? {
            return Err("情报板周期开始日期必须是游戏周一");
        }
        Ok(())
    }
}

/// 只接受完整进度；不能把未知文字、错误分母或空串当作 0/1000。
pub fn parse_progress(text: &str) -> Option<u16> {
    let normalized: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == '／' { '/' } else { c })
        .collect();
    let (current, total) = normalized.split_once('/')?;
    if total != "1000" || current.is_empty() || !current.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    current
        .parse::<u16>()
        .ok()
        .filter(|value| *value <= CONTRIBUTION_TARGET)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CommissionKind {
    NotoriousHunt,
    ExpertChallenge,
}
impl CommissionKind {
    pub fn experience(self) -> u32 {
        match self {
            Self::NotoriousHunt => 500,
            Self::ExpertChallenge => 250,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::NotoriousHunt => "恶名狩猎",
            Self::ExpertChallenge => "专业挑战室",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WeeklyTotals {
    pub contribution: u16,
    /// 与旧版相同的历史贡献估计，不是从游戏直接读出的经验值。
    pub estimated_base_exp: u16,
    pub notorious_hunt_count: u16,
    pub expert_challenge_count: u16,
}

/// 从同一账号、区服与游戏周的运行检查点恢复。source_run_id 只标识最近采用的
/// 快照；历史基线已包含之前的战斗，合并时不能再次累加。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct IntelHistory {
    pub week: Option<IntelWeek>,
    pub totals: WeeklyTotals,
    pub source_run_id: Option<uuid::Uuid>,
    pub unreadable: u32,
}

pub fn apply_history(
    plan: &mut crate::run_plan::GroupPlan,
    history: IntelHistory,
) -> Result<(), &'static str> {
    history.totals.validate()?;
    if history.source_run_id.is_none() && history.totals != WeeklyTotals::default() {
        return Err("情报板累计记录缺少来源运行");
    }
    if history.week.as_ref() != Some(&IntelWeek::from_day(&plan.game_day)?) {
        return Err("情报板历史不属于本次游戏周");
    }
    if history.unreadable > 0 {
        plan.warnings.push(format!(
            "{} 份情报板周记录无法采用，已保留原文件；未知结果不计作完成。",
            history.unreadable
        ));
    }
    if let Some(item) = plan.items.iter_mut().find(|item| {
        item.enabled
            && item.app_id == "intel_board"
            && matches!(
                item.availability,
                crate::run_plan::AppAvailability::DailyTrial
            )
    }) {
        item.completed_run_id = history.source_run_id;
        item.decision = if history.totals.completed(&plan.intel_board_config) {
            crate::run_plan::PlanDecision::AlreadyDone
        } else {
            crate::run_plan::PlanDecision::Run
        };
        plan.all_done = plan.blockers.is_empty()
            && !plan
                .items
                .iter()
                .any(|item| item.decision == crate::run_plan::PlanDecision::Run);
        plan.executable = plan.blockers.is_empty() && !plan.all_done;
    }
    plan.intel_board_history = history;
    Ok(())
}
impl WeeklyTotals {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.contribution > CONTRIBUTION_TARGET
            || self.estimated_base_exp > 3750
            || !self.estimated_base_exp.is_multiple_of(250)
            || u32::from(self.notorious_hunt_count) + u32::from(self.expert_challenge_count) > 4096
        {
            return Err("情报板周计数或历史经验估计无效");
        }
        Ok(())
    }
    pub fn experience(&self) -> u32 {
        u32::from(self.estimated_base_exp)
            + u32::from(self.notorious_hunt_count) * CommissionKind::NotoriousHunt.experience()
            + u32::from(self.expert_challenge_count) * CommissionKind::ExpertChallenge.experience()
    }
    pub fn completed(&self, config: &IntelBoardConfig) -> bool {
        self.validate().is_ok()
            && config.validate().is_ok()
            && if config.exp_grind_mode {
                self.experience() >= EXP_TARGET
            } else {
                self.contribution == CONTRIBUTION_TARGET
            }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SettledCommission {
    pub kind: CommissionKind,
    pub result: NormalBattleResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PendingCommission {
    pub kind: CommissionKind,
    pub deployed: bool,
    pub result: Option<NormalBattleResult>,
}

/// 每次运行只追加自己的已结算委托；冻结的历史计数不重复写成新的战斗。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct IntelLedger {
    pub config: IntelBoardConfig,
    pub week: IntelWeek,
    pub baseline: WeeklyTotals,
    pub initial_contribution: Option<u16>,
    pub observed_contribution: Option<u16>,
    pub estimated_base_exp: u16,
    pub pending: Option<PendingCommission>,
    pub settled: Vec<SettledCommission>,
}
impl IntelLedger {
    pub fn new(
        config: IntelBoardConfig,
        day: &GameDay,
        baseline: WeeklyTotals,
    ) -> Result<Self, &'static str> {
        let result = Self {
            config,
            week: IntelWeek::from_day(day)?,
            estimated_base_exp: baseline.estimated_base_exp,
            baseline,
            initial_contribution: None,
            observed_contribution: None,
            pending: None,
            settled: vec![],
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate()?;
        self.week.validate()?;
        self.baseline.validate()?;
        if self
            .initial_contribution
            .is_some_and(|value| value > CONTRIBUTION_TARGET)
            || self
                .observed_contribution
                .is_some_and(|value| value > CONTRIBUTION_TARGET)
        {
            return Err("情报板贡献值超出周期上限");
        }
        let allowed_base = if self.baseline.estimated_base_exp == 0
            && self.baseline.notorious_hunt_count == 0
            && self.baseline.expert_challenge_count == 0
            && self.config.exp_grind_mode
        {
            self.initial_contribution
                .map(|value| value.div_ceil(70) * 250)
                .unwrap_or(0)
        } else {
            self.baseline.estimated_base_exp
        };
        if self.settled.len() + usize::from(self.pending.is_some()) > MAX_RUN_COMMISSIONS
            || self.initial_contribution.is_some() != self.observed_contribution.is_some()
            || self.initial_contribution > self.observed_contribution
            || self
                .initial_contribution
                .is_some_and(|value| value < self.baseline.contribution)
            || (self.initial_contribution.is_none()
                && (self.pending.is_some() || !self.settled.is_empty()))
            || self.estimated_base_exp != allowed_base
            || self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.result.is_some() && !pending.deployed)
        {
            return Err("情报板检查点缺少战斗、周期或进度证明");
        }
        self.totals().validate()
    }
    pub fn totals(&self) -> WeeklyTotals {
        let mut totals = self.baseline.clone();
        totals.contribution = totals
            .contribution
            .max(self.observed_contribution.unwrap_or(0));
        totals.estimated_base_exp = self.estimated_base_exp;
        for result in &self.settled {
            if result.result == NormalBattleResult::Completed {
                match result.kind {
                    CommissionKind::NotoriousHunt => {
                        totals.notorious_hunt_count = totals.notorious_hunt_count.saturating_add(1)
                    }
                    CommissionKind::ExpertChallenge => {
                        totals.expert_challenge_count =
                            totals.expert_challenge_count.saturating_add(1)
                    }
                }
            }
        }
        totals
    }
    pub fn read_progress(&mut self, text: &str) -> Result<(), &'static str> {
        self.validate()?;
        let value = parse_progress(text).ok_or("情报板周期进度不可读")?;
        if self
            .observed_contribution
            .is_some_and(|previous| value < previous)
            || value < self.baseline.contribution
        {
            return Err("情报板进度下降，请重新核对账号和游戏周");
        }
        let mut next = self.clone();
        // 只在首次读板且尚无本周计数时估算，后续进度已经包含本次战斗。
        if next.initial_contribution.is_none() {
            next.initial_contribution = Some(value);
            if next.baseline.estimated_base_exp == 0
                && next.baseline.notorious_hunt_count == 0
                && next.baseline.expert_challenge_count == 0
                && next.config.exp_grind_mode
            {
                next.estimated_base_exp = value.div_ceil(70) * 250;
            }
        }
        next.observed_contribution = Some(value);
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn begin(&mut self, kind: CommissionKind) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some()
            || self.settled.len() >= MAX_RUN_COMMISSIONS
            || self.observed_contribution.is_none()
            || self.totals().completed(&self.config)
        {
            return Err("情报板尚有未结算委托、未读取进度、已完成周目标或达到运行上限");
        }
        self.pending = Some(PendingCommission {
            kind,
            deployed: false,
            result: None,
        });
        Ok(())
    }
    pub fn mark_deployed(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        let pending = self.pending.as_mut().ok_or("没有本次委托")?;
        if pending.deployed {
            return Err("本次委托已经投递出战");
        }
        pending.deployed = true;
        Ok(())
    }
    pub fn record_battle(&mut self, result: NormalBattleResult) -> Result<(), &'static str> {
        self.validate()?;
        let pending = self.pending.as_mut().ok_or("没有本次委托")?;
        if !pending.deployed || pending.result.is_some_and(|previous| previous != result) {
            return Err("本次委托未出战或结算结果冲突");
        }
        pending.result = Some(result);
        Ok(())
    }
    pub fn return_to_board(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        let pending = self
            .pending
            .as_ref()
            .ok_or("没有待结算委托，不能重复计数")?;
        let result = pending.result.ok_or("返回情报板前未取得本次战斗结算证明")?;
        if self.settled.len() >= MAX_RUN_COMMISSIONS {
            return Err("情报板本次委托数量达到上限");
        }
        let mut next = self.clone();
        next.settled.push(SettledCommission {
            kind: pending.kind,
            result,
        });
        next.pending = None;
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn cancel_unaccepted(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.as_ref().is_none_or(|pending| pending.deployed) {
            return Err("没有可取消的待接取委托");
        }
        self.pending = None;
        Ok(())
    }

    /// 检查点只能推进一个已冻结委托，不能撤回历史或改写已经确认的结果。
    pub fn check_update(&self, next: &Self) -> Result<(), &'static str> {
        self.validate()?;
        next.validate()?;
        if self.config != next.config
            || self.week != next.week
            || self.baseline != next.baseline
            || self
                .initial_contribution
                .is_some_and(|value| next.initial_contribution != Some(value))
            || self.observed_contribution > next.observed_contribution
            || !next.settled.starts_with(&self.settled)
            || next.settled.len() > self.settled.len() + 1
        {
            return Err("情报板冻结配置或已有结果被修改");
        }
        if next.settled.len() > self.settled.len() {
            let pending = self.pending.as_ref().ok_or("新增结算没有对应委托")?;
            let result = pending.result.ok_or("新增结算没有战斗结果")?;
            if next.pending.is_some()
                || next.settled.last()
                    != Some(&SettledCommission {
                        kind: pending.kind,
                        result,
                    })
            {
                return Err("新增结算与本次委托不符");
            }
        } else if let Some(previous) = &self.pending {
            match &next.pending {
                Some(pending)
                    if previous.kind == pending.kind
                        && (!previous.deployed || pending.deployed)
                        && (previous.deployed || pending.result.is_none())
                        && previous
                            .result
                            .is_none_or(|result| pending.result == Some(result)) => {}
                None if !previous.deployed => {}
                _ => return Err("待结算委托被撤销或改写"),
            }
        } else if next
            .pending
            .as_ref()
            .is_some_and(|pending| pending.deployed || pending.result.is_some())
        {
            return Err("新委托不能跳过出战检查点");
        }
        Ok(())
    }
}
