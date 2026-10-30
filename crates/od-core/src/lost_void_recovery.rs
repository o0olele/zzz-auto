//! 同一层的失败与重开额度。只分配尝试身份，不提供菜单输入或到达证明。
use crate::{lost_void::REGIONS, lost_void_npc::NpcMemory};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

pub const MAX_LAYER_RESTARTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryReason {
    NavigationTimeout,
    BattleTimeout,
    AgentDead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerFailure {
    pub id: Uuid,
    pub run_id: Uuid,
    pub attempt_id: Uuid,
    pub reason: RetryReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestartIntent {
    pub failure: LayerFailure,
    pub next_attempt_id: Uuid,
    pub ordinal: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestartAttempt {
    intent: RestartIntent,
    confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    FindExit,
    Restart(RestartIntent),
    Exit,
    AlreadyHandled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerRetryBudget {
    run_id: Uuid,
    layer_id: Uuid,
    region: String,
    restarts: Vec<RestartAttempt>,
    skipped: Vec<LayerFailure>,
    exit: Option<LayerFailure>,
}

impl LayerRetryBudget {
    pub fn new(run_id: Uuid, layer_id: Uuid, region: String) -> Result<Self, &'static str> {
        let value = Self {
            run_id,
            layer_id,
            region,
            restarts: vec![],
            skipped: vec![],
            exit: None,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn run_id(&self) -> Uuid {
        self.run_id
    }
    pub fn layer_id(&self) -> Uuid {
        self.layer_id
    }
    pub fn region(&self) -> &str {
        &self.region
    }
    pub fn current_attempt_id(&self) -> Uuid {
        self.restarts
            .iter()
            .rev()
            .find(|r| r.confirmed)
            .map_or(self.layer_id, |r| r.intent.next_attempt_id)
    }
    pub fn restart_count(&self) -> u8 {
        self.restarts.len() as u8
    }
    pub fn pending_restart(&self) -> Option<&RestartIntent> {
        self.restarts
            .last()
            .filter(|r| !r.confirmed)
            .map(|r| &r.intent)
    }
    pub fn exit_failure(&self) -> Option<&LayerFailure> {
        self.exit.as_ref()
    }
    pub fn uses_identity(&self, id: Uuid) -> bool {
        id == self.run_id
            || id == self.layer_id
            || self.restarts.iter().any(|r| r.intent.next_attempt_id == id)
            || self.failures().any(|f| f.id == id || f.attempt_id == id)
    }
    pub fn skipped_current_exclamations(&self) -> bool {
        self.skipped
            .iter()
            .any(|f| f.attempt_id == self.current_attempt_id())
    }
    fn failures(&self) -> impl Iterator<Item = &LayerFailure> {
        self.restarts
            .iter()
            .map(|r| &r.intent.failure)
            .chain(&self.skipped)
            .chain(self.exit.as_ref())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.run_id.is_nil()
            || self.layer_id.is_nil()
            || self.run_id == self.layer_id
            || !REGIONS.contains(&self.region.as_str())
            || self.restarts.len() > MAX_LAYER_RESTARTS
            || self.skipped.len() > MAX_LAYER_RESTARTS + 1
        {
            return Err("迷失之地重开额度身份、区域或次数无效");
        }
        let mut attempt = self.layer_id;
        let mut attempts = BTreeSet::from([attempt]);
        for (i, r) in self.restarts.iter().enumerate() {
            if r.intent.failure.attempt_id != attempt
                || r.intent.ordinal as usize != i + 1
                || (!r.confirmed && i + 1 != self.restarts.len())
                || r.intent.next_attempt_id.is_nil()
                || r.intent.next_attempt_id == self.run_id
                || !attempts.insert(r.intent.next_attempt_id)
            {
                return Err("迷失之地重开尝试链无效");
            }
            if r.confirmed {
                attempt = r.intent.next_attempt_id;
            }
        }
        let mut failure_ids = BTreeSet::new();
        for f in self.failures() {
            if f.id.is_nil()
                || f.run_id != self.run_id
                || f.id == self.run_id
                || attempts.contains(&f.id)
                || !failure_ids.insert(f.id)
            {
                return Err("迷失之地失败身份重复或属于其他运行");
            }
        }
        let confirmed: BTreeSet<_> = self
            .restarts
            .iter()
            .filter(|r| r.confirmed)
            .map(|r| r.intent.next_attempt_id)
            .chain([self.layer_id])
            .collect();
        let mut skipped_attempts = BTreeSet::new();
        if self.skipped.iter().any(|f| {
            self.region != "入口"
                || f.reason != RetryReason::NavigationTimeout
                || !confirmed.contains(&f.attempt_id)
                || !skipped_attempts.insert(f.attempt_id)
        }) {
            return Err("迷失之地入口放弃感叹号记录无效");
        }
        if self.exit.as_ref().is_some_and(|f| {
            self.restarts.len() != MAX_LAYER_RESTARTS
                || self.pending_restart().is_some()
                || f.attempt_id != self.current_attempt_id()
        }) {
            return Err("迷失之地退出尚未耗尽本层重开额度");
        }
        Ok(())
    }
    /// 重复失败只回交原意图；已确认的旧失败不会重新分配尝试或产生新动作。
    pub fn begin(
        &mut self,
        failure: LayerFailure,
        memory: &NpcMemory,
        next_attempt_id: Uuid,
    ) -> Result<RetryDecision, &'static str> {
        self.validate()?;
        memory.validate()?;
        if memory.run_id() != self.run_id || memory.region() != self.region {
            return Err("迷失之地重开缺少当前运行的层记忆");
        }
        if let Some(old) = self.failures().find(|f| f.id == failure.id) {
            if old != &failure {
                return Err("迷失之地同次失败内容发生变化");
            }
            if self.pending_restart().is_some_and(|i| i.failure == failure)
                && memory.level_id() != failure.attempt_id
            {
                return Err("迷失之地待确认重开与实际层记忆不符");
            }
            return Ok(self
                .pending_restart()
                .filter(|i| i.failure == failure)
                .map_or(RetryDecision::AlreadyHandled, |i| {
                    RetryDecision::Restart(i.clone())
                }));
        }
        if self.exit.is_some()
            || self.pending_restart().is_some()
            || failure.run_id != self.run_id
            || failure.attempt_id != self.current_attempt_id()
            || memory.level_id() != failure.attempt_id
        {
            return Err("迷失之地失败不是当前可恢复的层尝试");
        }
        let mut next = self.clone();
        let decision = if self.region == "入口"
            && failure.reason == RetryReason::NavigationTimeout
            && !memory.completed_first().is_empty()
            && memory.ready_for_navigation()
            && !memory.skip_exclamations()
            && !self.skipped_current_exclamations()
        {
            next.skipped.push(failure);
            RetryDecision::FindExit
        } else if self.restarts.len() < MAX_LAYER_RESTARTS {
            let intent = RestartIntent {
                failure,
                next_attempt_id,
                ordinal: self.restart_count() + 1,
            };
            next.restarts.push(RestartAttempt {
                intent: intent.clone(),
                confirmed: false,
            });
            RetryDecision::Restart(intent)
        } else {
            next.exit = Some(failure);
            RetryDecision::Exit
        };
        next.validate()?;
        *self = next;
        Ok(decision)
    }
    /// 调用方须先取得实际重开后态。此确认本身不能用于构造生产层执行器。
    pub fn confirm_restart(&mut self, intent: &RestartIntent) -> Result<bool, &'static str> {
        self.validate()?;
        let Some(index) = self.restarts.iter().position(|r| &r.intent == intent) else {
            return Err("迷失之地重开确认不是本次预留意图");
        };
        if self.restarts[index].confirmed {
            return Ok(false);
        }
        let mut next = self.clone();
        next.restarts[index].confirmed = true;
        next.validate()?;
        *self = next;
        Ok(true)
    }
    pub fn validate_successor(&self, old: &Self) -> Result<(), &'static str> {
        self.validate()?;
        old.validate()?;
        if self.run_id != old.run_id
            || self.layer_id != old.layer_id
            || self.region != old.region
            || self.restarts.len() < old.restarts.len()
            || !self.skipped.starts_with(&old.skipped)
            || old
                .exit
                .as_ref()
                .is_some_and(|f| self.exit.as_ref() != Some(f))
            || old
                .restarts
                .iter()
                .zip(&self.restarts)
                .any(|(a, b)| a.intent != b.intent || a.confirmed && !b.confirmed)
        {
            return Err("迷失之地重开历史倒退或身份变化");
        }
        Ok(())
    }
}
