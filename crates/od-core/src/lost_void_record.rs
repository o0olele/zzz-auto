//! 账号共用的结算记账核心；序列化证据只能恢复记账，不能恢复游戏输入。
use crate::{
    daily::GameDay,
    lost_void::{LostVoidHistory, LostVoidMission},
    lost_void_battle::{BattleOutcome, BattleReceipt},
    lost_void_level::{LevelOutcome, LostVoidLevelReceipt},
    lost_void_recovery::RetryReason,
    lost_void_roster::{RosterMember, validate_members},
    lost_void_scene::{BountyProgress, LostVoidScreen as S},
    lost_void_settlement::{
        CompletionKind, RewardCaps, RewardEvidence, SettlementAction, SettlementFlow,
        SettlementOutcome,
    },
};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

const MAX_RECORDS: usize = 8192;

fn date(day: &GameDay) -> Result<NaiveDate, &'static str> {
    day.validate()?;
    NaiveDate::parse_from_str(&day.date, "%Y-%m-%d").map_err(|_| "迷失之地记录日期无效")
}
fn same_week(a: &GameDay, b: &GameDay) -> Result<bool, &'static str> {
    Ok(a.region == b.region && date(a)?.iso_week() == date(b)?.iso_week())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidUpCompletion {
    pub observed_day: GameDay,
    pub up_agents: [String; 3],
}
impl LostVoidUpCompletion {
    pub fn matches_party_in_week(&self, day: &GameDay, members: &[RosterMember]) -> bool {
        self.validate(day, members).is_ok()
    }
    fn validate(&self, day: &GameDay, members: &[RosterMember]) -> Result<(), &'static str> {
        if !same_week(&self.observed_day, day)? || date(&self.observed_day)? > date(day)? {
            return Err("迷失之地 UP 读数不属于完成时的游戏周");
        }
        validate_members(members, members.len() as u8)?;
        let known = crate::random_play::agents();
        if self
            .up_agents
            .iter()
            .enumerate()
            .any(|(i, id)| !known.iter().any(|a| a.id == *id) || self.up_agents[..i].contains(id))
            || !members.iter().any(|m| self.up_agents.contains(&m.id))
        {
            return Err("迷失之地 UP 名单不完整或实际最终战队伍未包含 UP 代理人");
        }
        Ok(())
    }
}

/// 不保留藏品规则和 OCR 中间帧；保留足以复验计数及结束性质的来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidSettlementRecord {
    instance_id: u32,
    day: GameDay,
    run_id: Uuid,
    level_id: Uuid,
    settlement_id: Uuid,
    region: String,
    source: LevelOutcome,
    mission: LostVoidMission,
    members: Option<Vec<RosterMember>>,
    expected_count: u8,
    completion: CompletionKind,
    entry: S,
    actions: Vec<SettlementAction>,
    rewards: RewardEvidence,
    caps: RewardCaps,
    bounty: Option<BountyProgress>,
    up: Option<LostVoidUpCompletion>,
}
impl LostVoidSettlementRecord {
    pub fn from_settlement(
        instance_id: u32,
        day: GameDay,
        flow: &SettlementFlow,
        up: Option<LostVoidUpCompletion>,
    ) -> Result<Self, &'static str> {
        day.validate()?;
        flow.validate()?;
        let receipt = flow.receipt().ok_or("迷失之地结算尚未完成")?;
        let SettlementOutcome::Finished {
            completion,
            caps,
            bounty,
            entry,
            rewards,
        } = &receipt.outcome
        else {
            return Err("中途挚交会谈不能提交整场记录");
        };
        let request = flow.request();
        let result = Self {
            instance_id,
            day,
            run_id: receipt.run_id,
            level_id: receipt.level_id,
            settlement_id: receipt.settlement_id,
            region: request.source.region.clone(),
            source: request.source.outcome.clone(),
            mission: request.mission,
            members: request.members.clone(),
            expected_count: request.expected_count,
            completion: *completion,
            entry: *entry,
            actions: receipt.actions.clone(),
            rewards: rewards.clone(),
            caps: *caps,
            bounty: *bounty,
            up,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn day(&self) -> &GameDay {
        &self.day
    }
    pub fn settlement_id(&self) -> Uuid {
        self.settlement_id
    }
    pub fn instance_id(&self) -> u32 {
        self.instance_id
    }
    pub fn run_id(&self) -> Uuid {
        self.run_id
    }
    pub fn completion(&self) -> CompletionKind {
        self.completion
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.day.validate()?;
        self.rewards.validate()?;
        if !(1..=9999).contains(&self.instance_id)
            || self.settlement_id.is_nil()
            || self.run_id == self.level_id
            || [self.run_id, self.level_id].contains(&self.settlement_id)
        {
            return Err("迷失之地账号或结算身份无效");
        }
        if !matches!(self.entry, S::EntryStandard | S::EntryPeriodic)
            || self.actions.len() > 3
            || self.actions.last() != Some(&SettlementAction::Finish)
            || self
                .actions
                .iter()
                .enumerate()
                .any(|(i, a)| self.actions[..i].contains(a))
            || self.completion == CompletionKind::Completed && self.caps != self.rewards.caps()
        {
            return Err("迷失之地记录缺少最终返回入口或奖励采样证明");
        }
        LostVoidLevelReceipt {
            run_id: self.run_id,
            level_id: self.level_id,
            region: self.region.clone(),
            options: Default::default(),
            outcome: self.source.clone(),
        }
        .validate()?;
        let source_id = match &self.source {
            LevelOutcome::ObservedResult {
                battle: Some(b), ..
            }
            | LevelOutcome::AgentDead { battle: b } => Some(b.encounter_id),
            LevelOutcome::Aborted { failure } => Some(failure.id),
            _ => None,
        };
        if source_id
            .is_some_and(|id| [self.run_id, self.level_id, self.settlement_id].contains(&id))
        {
            return Err("迷失之地结算与战斗或失败身份混用");
        }
        if !(1..=3).contains(&self.expected_count)
            || self.completion == CompletionKind::Completed && self.members.is_none()
        {
            return Err("迷失之地通关记录缺少实际完整队伍人数");
        }
        if let Some(m) = &self.members {
            validate_members(m, self.expected_count)?;
        }
        if self
            .bounty
            .is_some_and(|b| b.total != 8000 || b.current > b.total)
        {
            return Err("迷失之地悬赏记录不是完整有效读数");
        }
        let victory = self.region == "战斗-终结之役"
            && matches!(
                &self.source,
                LevelOutcome::ObservedResult {
                    screen: S::BattleResult,
                    battle: Some(BattleReceipt {
                        outcome: BattleOutcome::ObservedPage {
                            screen: S::BattleResult
                        },
                        combat_hud_observed: true,
                        ..
                    })
                }
            );
        let abort = matches!(&self.source, LevelOutcome::Aborted { failure }
            if failure.reason != RetryReason::AgentDead);
        let failure = matches!(
            &self.source,
            LevelOutcome::AgentDead { .. }
                | LevelOutcome::Aborted { .. }
                | LevelOutcome::ObservedResult {
                    screen: S::BattleFail,
                    ..
                }
        );
        if matches!(self.source, LevelOutcome::NextLevel { .. })
            || match self.completion {
                CompletionKind::Completed => !victory || abort || failure,
                CompletionKind::Aborted => !abort,
                CompletionKind::Unattributed => victory || abort || failure,
                CompletionKind::Defeated => abort,
            }
            || self.completion != CompletionKind::Completed
                && (self.caps.evaluation.is_some()
                    || self.caps.period.is_some()
                    || self.up.is_some())
        {
            return Err("迷失之地结束性质与本层战斗来源不一致");
        }
        if let Some(up) = &self.up {
            if self.mission != LostVoidMission::TaskForce {
                return Err("只有特遣调查可提交三名 UP 完成");
            }
            up.validate(
                &self.day,
                self.members.as_deref().ok_or("UP 完成缺少实际最终战队伍")?,
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct LostVoidProgress {
    pub day: GameDay,
    pub daily_run_times: u16,
    pub weekly_run_times: u16,
    pub bounty_commission_complete: bool,
    pub eval_point_complete: Option<bool>,
    pub period_reward_complete: Option<bool>,
    pub complete_task_force_with_up: bool,
}
impl LostVoidProgress {
    pub fn history(&self) -> LostVoidHistory {
        LostVoidHistory {
            day: Some(self.day.clone()),
            daily_run_times: self.daily_run_times,
            weekly_run_times: self.weekly_run_times,
            bounty_commission_complete: self.bounty_commission_complete,
            eval_point_complete: self.eval_point_complete.unwrap_or(false),
            period_reward_complete: self.period_reward_complete.unwrap_or(false),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidAccountLedger {
    instance_id: u32,
    records: BTreeMap<Uuid, LostVoidSettlementRecord>,
}
impl LostVoidAccountLedger {
    pub(crate) fn instance_id(&self) -> u32 {
        self.instance_id
    }
    pub(crate) fn records(&self) -> impl Iterator<Item = &LostVoidSettlementRecord> {
        self.records.values()
    }
    pub fn new(instance_id: u32) -> Result<Self, &'static str> {
        let ledger = Self {
            instance_id,
            records: BTreeMap::new(),
        };
        ledger.validate()?;
        Ok(ledger)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=9999).contains(&self.instance_id) || self.records.len() > MAX_RECORDS {
            return Err("迷失之地账号或结算记录容量无效");
        }
        let mut attempts = BTreeSet::new();
        for (id, r) in &self.records {
            r.validate()?;
            if *id != r.settlement_id
                || r.instance_id != self.instance_id
                || !attempts.insert((r.run_id, r.level_id))
            {
                return Err("迷失之地结算记录重复或账号身份不一致");
            }
        }
        Ok(())
    }
    pub fn commit(&mut self, record: LostVoidSettlementRecord) -> Result<bool, &'static str> {
        self.validate()?;
        record.validate()?;
        if let Some(old) = self.records.get(&record.settlement_id) {
            return if old == &record {
                Ok(false)
            } else {
                Err("迷失之地同一结算的内容发生变化")
            };
        }
        if record.instance_id != self.instance_id
            || self.records.len() == MAX_RECORDS
            || self
                .records
                .values()
                .any(|r| r.run_id == record.run_id && r.level_id == record.level_id)
        {
            return Err("迷失之地记录属于其他账号、同一尝试已记账或容量已满");
        }
        self.records.insert(record.settlement_id, record);
        Ok(true)
    }
    pub fn progress(&self, day: &GameDay) -> Result<LostVoidProgress, &'static str> {
        self.validate()?;
        let current = date(day)?;
        let mut progress = LostVoidProgress {
            day: day.clone(),
            daily_run_times: 0,
            weekly_run_times: 0,
            bounty_commission_complete: false,
            eval_point_complete: None,
            period_reward_complete: None,
            complete_task_force_with_up: false,
        };
        let merge = |old: Option<bool>, new: Option<bool>| match (old, new) {
            (Some(a), Some(b)) => Some(a || b),
            (None, b) => b,
            (a, None) => a,
        };
        for r in self.records.values() {
            if r.day.region == day.region && date(&r.day)? > current {
                return Err("迷失之地结算日期晚于当前游戏日");
            }
            if !same_week(&r.day, day)? {
                continue;
            }
            if r.completion == CompletionKind::Completed {
                progress.weekly_run_times += 1;
                if r.day == *day {
                    progress.daily_run_times += 1;
                }
                progress.eval_point_complete =
                    merge(progress.eval_point_complete, r.caps.evaluation);
                progress.period_reward_complete =
                    merge(progress.period_reward_complete, r.caps.period);
                progress.complete_task_force_with_up |= r.up.is_some();
            }
            progress.bounty_commission_complete |= r.bounty.is_some_and(BountyProgress::completed);
        }
        Ok(progress)
    }
}
