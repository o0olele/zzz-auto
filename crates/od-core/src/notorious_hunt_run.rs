//! 周常计划的可重放日志：实际免费余量、顺序、整轮扣减和逐次去重。
use crate::{
    charge_plan::ChargePlanItem,
    daily::{GameDay, GameRegion},
    notorious_hunt::NotoriousHuntConfig,
    predefined_teams::TeamConfig,
};
use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;
use uuid::Uuid;

pub const MAX_ATTEMPTS: usize = crate::charge_plan::MAX_PLANS + 3;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WeeklyHuntWeek {
    pub region: GameRegion,
    pub monday: String,
}
impl WeeklyHuntWeek {
    pub fn from_day(day: &GameDay) -> Result<Self, &'static str> {
        day.validate()?;
        let date =
            NaiveDate::parse_from_str(&day.date, "%Y-%m-%d").map_err(|_| "周常游戏日无效")?;
        let monday = date
            .checked_sub_signed(Duration::days(i64::from(
                date.weekday().num_days_from_monday(),
            )))
            .ok_or("周常游戏周超出日期范围")?;
        Ok(Self {
            region: day.region,
            monday: monday.format("%Y-%m-%d").to_string(),
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if *self
            != Self::from_day(&GameDay {
                region: self.region,
                date: self.monday.clone(),
            })?
        {
            return Err("周常周期必须从游戏周一开始");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WeeklyRunContext {
    pub run_id: Uuid,
    pub instance_id: u32,
    pub group_id: String,
    pub config_revision: u32,
    pub game_day: GameDay,
}
impl WeeklyRunContext {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.game_day.validate()?;
        if self.run_id.is_nil()
            || !(1..=9999).contains(&self.instance_id)
            || self.group_id.is_empty()
            || self.group_id.len() > 128
            || !self
                .group_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err("周常运行身份无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyRunRequest {
    pub context: WeeklyRunContext,
    pub config: NotoriousHuntConfig,
    pub teams: TeamConfig,
}
impl WeeklyRunRequest {
    pub fn from_config(
        config: &crate::config::AccountConfig,
        instance_id: u32,
        group_id: &str,
        run_id: Uuid,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Self, &'static str> {
        config.validate().map_err(|_| "周常账号配置无效")?;
        if config.active_instance != instance_id {
            return Err("周常运行账号不是当前账号");
        }
        let account = config
            .instances
            .iter()
            .find(|a| a.id == instance_id)
            .ok_or("周常账号不存在")?;
        let group = account
            .groups
            .iter()
            .find(|g| g.id == group_id)
            .ok_or("周常应用组不存在")?;
        let request = Self {
            context: WeeklyRunContext {
                run_id,
                instance_id,
                group_id: group_id.into(),
                config_revision: config.revision,
                game_day: GameDay::at(account.game_region, now)?,
            },
            config: (*group.notorious_hunt).clone(),
            teams: account.predefined_teams.clone(),
        };
        request.validate()?;
        Ok(request)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.context.validate()?;
        self.config.validate()?;
        self.teams.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyAttempt {
    pub run_id: Uuid,
    pub sequence: u32,
    pub plan: ChargePlanItem,
    pub script: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklySelection {
    Ready(Box<WeeklyAttempt>),
    NoPlans,
    BeforeStartDay,
    Completed,
    Exhausted,
    Skipped,
    AwaitingVerification,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklySkipReason {
    AgentUnavailable,
    AgentCompleted,
    ChallengeFailed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum WeeklyOutcome {
    Succeeded {
        execution_id: Uuid,
        rewards_before: u8,
        rewards_after: u8,
    },
    AgentSucceeded {
        execution_id: Uuid,
        rewards_before: u8,
        rewards_after: u8,
        goal: crate::agent_goal::AgentGoalStatus,
    },
    Exhausted {
        execution_id: Uuid,
    },
    AwaitingVerification {
        execution_id: Uuid,
    },
    /// 只记录已由执行器确认的不可执行或失败收尾，不增加计划次数。
    Skipped {
        execution_id: Uuid,
        reason: WeeklySkipReason,
    },
}
impl WeeklyOutcome {
    fn execution_id(&self) -> Uuid {
        match self {
            Self::Succeeded { execution_id, .. }
            | Self::AgentSucceeded { execution_id, .. }
            | Self::Exhausted { execution_id }
            | Self::AwaitingVerification { execution_id }
            | Self::Skipped { execution_id, .. } => *execution_id,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyAttemptRecord {
    pub attempt: WeeklyAttempt,
    pub outcome: WeeklyOutcome,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklySchedule {
    schema_version: u32,
    request: WeeklyRunRequest,
    records: Vec<WeeklyAttemptRecord>,
    pending: Option<WeeklyAttempt>,
    closure: Option<WeeklySelection>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeeklyProgress {
    pub config: NotoriousHuntConfig,
    pub skipped: BTreeSet<String>,
    pub successful_runs: u8,
    pub rewards_left: Option<u8>,
}
struct State {
    progress: WeeklyProgress,
    executions: BTreeSet<Uuid>,
    needs_verification: bool,
}
impl State {
    fn new(r: &WeeklyRunRequest) -> Self {
        Self {
            progress: WeeklyProgress {
                config: r.config.clone(),
                skipped: BTreeSet::new(),
                successful_runs: 0,
                rewards_left: None,
            },
            executions: BTreeSet::new(),
            needs_verification: false,
        }
    }
    fn select(
        &mut self,
        r: &WeeklyRunRequest,
        sequence: u32,
    ) -> Result<WeeklySelection, &'static str> {
        let c = &self.progress.config;
        if !c.plan_list.iter().any(|p| p.plan_times > 0) {
            return Ok(WeeklySelection::NoPlans);
        }
        let weekday = NaiveDate::parse_from_str(&r.context.game_day.date, "%Y-%m-%d")
            .map_err(|_| "周常游戏日无效")?
            .weekday()
            .number_from_monday() as u8;
        if weekday < c.weekly_challenge_start_weekday {
            return Ok(WeeklySelection::BeforeStartDay);
        }
        if self.needs_verification {
            return Ok(WeeklySelection::AwaitingVerification);
        }
        if self.progress.rewards_left == Some(0) {
            return Ok(WeeklySelection::Exhausted);
        }
        // 跳过的代理人不参与整轮最小值；普通失败仍阻止该轮被当成全部完成。
        if c.r#loop {
            let rounds = c
                .plan_list
                .iter()
                .filter(|p| {
                    p.plan_times > 0
                        && !(p.mission_type_name == "代理人方案培养"
                            && self.progress.skipped.contains(&p.plan_id))
                })
                .map(|p| p.run_times / p.plan_times)
                .min();
            if rounds.is_none_or(|n| n > 0) {
                for p in &mut self.progress.config.plan_list {
                    if p.mission_type_name == "代理人方案培养"
                        && self.progress.skipped.contains(&p.plan_id)
                    {
                        p.run_times = 0;
                    } else {
                        p.run_times -= rounds.unwrap_or(0) * p.plan_times;
                    }
                }
            }
        }
        for p in &self.progress.config.plan_list {
            if p.run_times < p.plan_times && !self.progress.skipped.contains(&p.plan_id) {
                return Ok(WeeklySelection::Ready(Box::new(WeeklyAttempt {
                    run_id: r.context.run_id,
                    sequence,
                    plan: p.clone(),
                    script: p.script(&r.teams)?.into(),
                })));
            }
        }
        Ok(if self.progress.skipped.is_empty() {
            WeeklySelection::Completed
        } else {
            WeeklySelection::Skipped
        })
    }
    fn apply(&mut self, record: &WeeklyAttemptRecord) -> Result<(), &'static str> {
        let execution = record.outcome.execution_id();
        if execution.is_nil() || !self.executions.insert(execution) {
            return Err("周常执行身份无效或已经采用");
        }
        match record.outcome {
            WeeklyOutcome::Succeeded {
                rewards_before,
                rewards_after,
                ..
            }
            | WeeklyOutcome::AgentSucceeded {
                rewards_before,
                rewards_after,
                ..
            } => {
                if let WeeklyOutcome::AgentSucceeded { goal, .. } = record.outcome {
                    if record.attempt.plan.mission_type_name != "代理人方案培养" {
                        return Err("普通周常计划不能采用培养达成结果");
                    }
                    match goal {
                        crate::agent_goal::AgentGoalStatus::Achieved => {
                            self.progress
                                .skipped
                                .insert(record.attempt.plan.plan_id.clone());
                        }
                        crate::agent_goal::AgentGoalStatus::Unknown => {
                            self.needs_verification = true
                        }
                        crate::agent_goal::AgentGoalStatus::NotAchieved => (),
                    }
                }
                if !(1..=3).contains(&rewards_before)
                    || rewards_after.checked_add(1) != Some(rewards_before)
                    || self
                        .progress
                        .rewards_left
                        .is_some_and(|n| n != rewards_before)
                    || self.progress.successful_runs >= 3
                {
                    return Err("周常结果缺少连续的实际免费余量减少证明");
                }
                let p = self
                    .progress
                    .config
                    .plan_list
                    .iter_mut()
                    .find(|p| p.plan_id == record.attempt.plan.plan_id)
                    .ok_or("周常结果缺少原计划")?;
                p.run_times = p
                    .run_times
                    .checked_add(1)
                    .filter(|n| *n <= 1_000_000)
                    .ok_or("周常计划次数超出范围")?;
                self.progress.successful_runs += 1;
                self.progress.rewards_left = Some(rewards_after);
            }
            WeeklyOutcome::Exhausted { .. } => self.progress.rewards_left = Some(0),
            WeeklyOutcome::AwaitingVerification { .. } => self.needs_verification = true,
            WeeklyOutcome::Skipped { reason, .. } => {
                if reason != WeeklySkipReason::ChallengeFailed
                    && record.attempt.plan.mission_type_name != "代理人方案培养"
                {
                    return Err("普通周常计划不能采用代理人跳过结果");
                }
                self.progress
                    .skipped
                    .insert(record.attempt.plan.plan_id.clone());
            }
        }
        Ok(())
    }
}
impl WeeklySchedule {
    pub fn new(request: WeeklyRunRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            schema_version: 1,
            request,
            records: vec![],
            pending: None,
            closure: None,
        })
    }
    pub fn request(&self) -> &WeeklyRunRequest {
        &self.request
    }
    pub fn records(&self) -> &[WeeklyAttemptRecord] {
        &self.records
    }
    pub fn pending(&self) -> Option<&WeeklyAttempt> {
        self.pending.as_ref()
    }
    pub fn final_selection(&self) -> Option<&WeeklySelection> {
        self.closure.as_ref()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.replay().map(|_| ())
    }
    pub fn progress(&self) -> Result<WeeklyProgress, &'static str> {
        Ok(self.replay()?.progress)
    }
    pub fn confirmed_prefix(&self, records: usize) -> Result<Self, &'static str> {
        self.validate()?;
        if records > self.records.len() {
            return Err("周常历史前缀超出已确认记录");
        }
        let mut p = self.clone();
        p.records.truncate(records);
        p.pending = None;
        p.closure = None;
        Ok(p)
    }
    pub fn prefix_fingerprint(&self, records: usize) -> Result<String, &'static str> {
        use sha2::{Digest, Sha256};
        let p = self.confirmed_prefix(records)?;
        Ok(hex::encode(Sha256::digest(
            serde_json::to_vec(&p).map_err(|_| "周常历史无法生成指纹")?,
        )))
    }
    pub fn validate_successor(&self, previous: &Self) -> Result<(), &'static str> {
        self.validate()?;
        previous.validate()?;
        if self.request != previous.request
            || !self.records.starts_with(&previous.records)
            || (previous.closure.is_some() && self != previous)
        {
            return Err("周常检查点不能回退或改写冻结结果");
        }
        if let Some(a) = &previous.pending {
            if self.records.len() == previous.records.len() {
                if self.pending.as_ref() != Some(a) {
                    return Err("周常检查点不能撤销已保存意图");
                }
            } else if self.records[previous.records.len()].attempt != *a {
                return Err("周常后继结果不属于已保存意图");
            }
        }
        Ok(())
    }
    fn replay(&self) -> Result<State, &'static str> {
        self.request.validate()?;
        if self.schema_version != 1
            || self.records.len() > MAX_ATTEMPTS
            || (self.pending.is_some() && self.closure.is_some())
            || (self.pending.is_some() && self.records.len() == MAX_ATTEMPTS)
        {
            return Err("周常检查点版本或数量无效");
        }
        let mut s = State::new(&self.request);
        for (i, record) in self.records.iter().enumerate() {
            if s.select(&self.request, i as u32 + 1)?
                != WeeklySelection::Ready(Box::new(record.attempt.clone()))
            {
                return Err("周常结果与冻结顺序或计划不符");
            }
            s.apply(record)?;
        }
        if let Some(a) = &self.pending
            && s.select(&self.request, self.records.len() as u32 + 1)?
                != WeeklySelection::Ready(Box::new(a.clone()))
        {
            return Err("周常意图与冻结顺序不符");
        }
        if let Some(c) = &self.closure
            && (matches!(c, WeeklySelection::Ready(_))
                || s.select(&self.request, self.records.len() as u32 + 1)? != *c)
        {
            return Err("周常结束证明无效");
        }
        Ok(s)
    }
    pub fn begin(&mut self, day: &GameDay) -> Result<WeeklySelection, &'static str> {
        if day != &self.request.context.game_day || self.pending.is_some() || self.closure.is_some()
        {
            return Err("周常运行跨日、已有未完成尝试或已结束");
        }
        let mut s = self.replay()?;
        let selection = s.select(&self.request, self.records.len() as u32 + 1)?;
        match &selection {
            WeeklySelection::Ready(a) => {
                if self.records.len() >= MAX_ATTEMPTS {
                    return Err("周常尝试达到上限");
                }
                self.pending = Some((**a).clone());
            }
            _ => self.closure = Some(selection.clone()),
        }
        Ok(selection)
    }
    pub fn record(
        &mut self,
        attempt: &WeeklyAttempt,
        outcome: WeeklyOutcome,
    ) -> Result<bool, &'static str> {
        self.validate()?;
        let record = WeeklyAttemptRecord {
            attempt: attempt.clone(),
            outcome,
        };
        if let Some(old) = self
            .records
            .iter()
            .find(|r| r.attempt.sequence == attempt.sequence)
        {
            return if *old == record {
                Ok(false)
            } else {
                Err("不能覆盖已经登记的周常结果")
            };
        }
        if self.pending.as_ref() != Some(attempt) {
            return Err("周常登记不是当前已保存尝试");
        }
        let mut n = self.clone();
        n.pending = None;
        n.records.push(record);
        n.validate()?;
        *self = n;
        Ok(true)
    }
}
