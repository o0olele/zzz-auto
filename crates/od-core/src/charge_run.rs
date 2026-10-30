//! 体力计划的冻结调度与逐次回执。只登记父流程已核实的结果，不执行游戏输入。
use crate::{
    agent_challenge::AgentChallengeResult,
    agent_goal::AgentGoalStatus,
    agent_transport::{AgentTransportRequest, AgentTransportTarget},
    charge_plan::{self, ChargeDecision, ChargePlanConfig, ChargePlanItem, ChargeResources},
    daily::GameDay,
    ether_synthesis::SynthesisStock,
    predefined_teams::TeamConfig,
    simulation_cards::CardsCharge,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;
use uuid::Uuid;

pub const MAX_ATTEMPTS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChargeRunContext {
    pub run_id: Uuid,
    pub instance_id: u32,
    pub group_id: String,
    pub config_revision: u32,
    pub game_day: GameDay,
}
impl ChargeRunContext {
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
            return Err("体力运行身份无效");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeRunRequest {
    pub context: ChargeRunContext,
    pub config: ChargePlanConfig,
    pub teams: TeamConfig,
    /// 开始时实际检查的双倍余量；开启双倍时未知值拒绝运行。
    pub double_left: Option<u8>,
}
/// 输入前冻结账号配置，双倍余量稍后仅由实际读帧绑定，不能用临时零值代替未知。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeRunSeed {
    pub context: ChargeRunContext,
    pub config: ChargePlanConfig,
    pub teams: TeamConfig,
}
impl ChargeRunSeed {
    pub fn from_config(
        config: &crate::config::AccountConfig,
        instance_id: u32,
        group_id: &str,
        run_id: Uuid,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Self, &'static str> {
        config.validate().map_err(|_| "体力运行账号配置无效")?;
        if config.active_instance != instance_id {
            return Err("体力运行账号不是当前账号");
        }
        let account = config
            .instances
            .iter()
            .find(|a| a.id == instance_id)
            .ok_or("体力账号不存在")?;
        let group = account
            .groups
            .iter()
            .find(|g| g.id == group_id)
            .ok_or("体力应用组不存在")?;
        let request = Self {
            context: ChargeRunContext {
                run_id,
                instance_id,
                group_id: group_id.into(),
                config_revision: config.revision,
                game_day: GameDay::at(account.game_region, now)?,
            },
            config: (*group.charge_plan).clone(),
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
    pub fn bind(self, double_left: Option<u8>) -> Result<ChargeRunRequest, &'static str> {
        self.validate()?;
        let request = ChargeRunRequest {
            context: self.context,
            config: self.config,
            teams: self.teams,
            double_left,
        };
        request.validate()?;
        Ok(request)
    }
}
impl ChargeRunRequest {
    pub fn from_config(
        config: &crate::config::AccountConfig,
        instance_id: u32,
        group_id: &str,
        run_id: Uuid,
        now: chrono::DateTime<chrono::Utc>,
        double_left: Option<u8>,
    ) -> Result<Self, &'static str> {
        ChargeRunSeed::from_config(config, instance_id, group_id, run_id, now)?.bind(double_left)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.context.validate()?;
        self.config.validate()?;
        self.teams.validate()?;
        if self.double_left.is_some_and(|n| n > 5)
            || (self.config.double_reward && self.double_left.is_none())
        {
            return Err("体力运行缺少可靠的双倍剩余次数");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeAttempt {
    pub run_id: Uuid,
    pub sequence: u32,
    pub plan: ChargePlanItem,
    pub script: String,
    pub temporary: bool,
    pub resources: ChargeResources,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeSelection {
    AgentTargetsUnavailable,
    NeedsVerification,
    WeeklyRewardsAvailable,
    Ready(Box<ChargeAttempt>),
    Completed,
    InsufficientCharge,
    UnavailableMaterials,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ChargeOutcome {
    Agent {
        execution_id: Uuid,
        target: Option<AgentTransportTarget>,
        result: AgentChallengeResult,
    },
    HuntSucceeded {
        execution_id: Uuid,
        charge_after: CardsCharge,
    },
    HuntInsufficientCharge {
        execution_id: Uuid,
        charge: CardsCharge,
    },
    HuntWeeklyRewardsAvailable {
        execution_id: Uuid,
        remaining: u8,
    },
    ExpertSucceeded {
        execution_id: Uuid,
        charge_after: CardsCharge,
    },
    ExpertInsufficientCharge {
        execution_id: Uuid,
        charge: CardsCharge,
    },
    PatrolSucceeded {
        execution_id: Uuid,
        charge_after: CardsCharge,
    },
    PatrolInsufficientCharge {
        execution_id: Uuid,
        charge: CardsCharge,
    },
    /// execution_id 由实际挑战实例生成；不能在下一轮重用同一次成功。
    Succeeded {
        execution_id: Uuid,
        charge_after: CardsCharge,
    },
    InsufficientCharge {
        execution_id: Uuid,
        charge: CardsCharge,
    },
    SynthesisSucceeded {
        execution_id: Uuid,
        before: SynthesisStock,
        after: SynthesisStock,
    },
    SynthesisUnavailable {
        execution_id: Uuid,
        stock: SynthesisStock,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeAttemptRecord {
    pub attempt: ChargeAttempt,
    pub outcome: ChargeOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Closure {
    resources: ChargeResources,
    selection: ChargeSelection,
}

/// 保存整份检查点后才能执行 pending 或交接下一项。反序列化后必须 validate；
/// 检查点只用于次数与历史复验，不能据此恢复游戏输入。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeSchedule {
    schema_version: u32,
    request: ChargeRunRequest,
    records: Vec<ChargeAttemptRecord>,
    pending: Option<ChargeAttempt>,
    closure: Option<Closure>,
}

pub struct ChargeProgress {
    /// 本次基线经每日重置、整轮扣减和已验证成功更新后的投影，不直接写配置。
    pub config: ChargePlanConfig,
    pub skipped: BTreeSet<String>,
    pub successful_runs: u32,
}
struct State {
    progress: ChargeProgress,
    double_checked: bool,
    stopped_for_charge: bool,
    shortfall_seen: bool,
    missing_synthesis_materials: bool,
    weekly_rewards_available: bool,
    agent_targets_unavailable: bool,
    needs_verification: bool,
    executions: BTreeSet<Uuid>,
}
impl State {
    fn new(request: &ChargeRunRequest) -> Self {
        let mut config = request.config.clone();
        if config.daily_reset_plan_times
            && config.last_daily_reset_day.as_ref() != Some(&request.context.game_day)
        {
            for plan in &mut config.plan_list {
                plan.run_times = 0;
            }
            config.last_daily_reset_day = Some(request.context.game_day.clone());
        }
        Self {
            progress: ChargeProgress {
                config,
                skipped: BTreeSet::new(),
                successful_runs: 0,
            },
            double_checked: false,
            stopped_for_charge: false,
            shortfall_seen: false,
            missing_synthesis_materials: false,
            weekly_rewards_available: false,
            agent_targets_unavailable: false,
            needs_verification: false,
            executions: BTreeSet::new(),
        }
    }
    fn select(
        &mut self,
        request: &ChargeRunRequest,
        resources: ChargeResources,
        sequence: u32,
    ) -> Result<ChargeSelection, &'static str> {
        resources.validate()?;
        if self.needs_verification {
            return Ok(ChargeSelection::NeedsVerification);
        }
        if self.stopped_for_charge {
            return Ok(ChargeSelection::InsufficientCharge);
        }
        // 已达成或确实没有目标的代理人计划不参与下一整轮的最小次数计算。
        // 跳过状态只存在于本次日志重放中；进入下一普通轮时沿用参考的次数清零。
        let mut forecast_config = self.progress.config.clone();
        forecast_config.plan_list.retain(|p| {
            p.mission_type_name != "代理人方案培养" || !self.progress.skipped.contains(&p.plan_id)
        });
        let forecast = charge_plan::preview(
            &forecast_config,
            &request.teams,
            request.context.game_day.clone(),
            Some(resources),
            request.double_left,
        )?;
        let attempt = |plan: ChargePlanItem, temporary| -> Result<_, &'static str> {
            Ok(ChargeSelection::Ready(Box::new(ChargeAttempt {
                run_id: request.context.run_id,
                sequence,
                script: plan.script(&request.teams)?.to_owned(),
                plan,
                temporary,
                resources,
            })))
        };
        if !self.double_checked {
            self.double_checked = true;
            if let Some(plan) = forecast.temporary {
                return attempt(plan, true);
            }
        }
        if forecast.rounds_rebased > 0 {
            for plan in &mut self.progress.config.plan_list {
                if plan.mission_type_name == "代理人方案培养"
                    && self.progress.skipped.contains(&plan.plan_id)
                {
                    plan.run_times = 0;
                }
            }
        }
        // 与只读估算相同的常数时间整轮扣减；不能将跳过当作成功。
        for item in &forecast.items {
            if let Some(plan) = self
                .progress
                .config
                .plan_list
                .iter_mut()
                .find(|p| p.plan_id == item.plan_id)
            {
                plan.run_times = item.effective_run_times;
            }
        }
        for item in forecast.items {
            let plan = self
                .progress
                .config
                .plan_list
                .iter()
                .find(|p| p.plan_id == item.plan_id)
                .ok_or("体力估算缺少原计划")?;
            if self.progress.skipped.contains(&plan.plan_id) {
                continue;
            }
            match item.decision {
                ChargeDecision::Ready | ChargeDecision::NeedsRestore => {
                    return attempt(plan.clone(), false);
                }
                ChargeDecision::Insufficient if self.progress.config.skip_plan => {
                    self.shortfall_seen = true;
                    self.progress.skipped.insert(plan.plan_id.clone());
                }
                ChargeDecision::Insufficient => return Ok(ChargeSelection::InsufficientCharge),
                ChargeDecision::Disabled | ChargeDecision::Completed => {}
                _ => return Err("体力运行资源未知"),
            }
        }
        Ok(if self.shortfall_seen {
            ChargeSelection::InsufficientCharge
        } else if self.missing_synthesis_materials {
            ChargeSelection::UnavailableMaterials
        } else if self.weekly_rewards_available {
            ChargeSelection::WeeklyRewardsAvailable
        } else if self.agent_targets_unavailable {
            ChargeSelection::AgentTargetsUnavailable
        } else {
            ChargeSelection::Completed
        })
    }
    fn apply(&mut self, record: &ChargeAttemptRecord) -> Result<(), &'static str> {
        if let ChargeOutcome::Agent {
            execution_id,
            target,
            result,
        } = &record.outcome
        {
            return self.apply_agent(record, *execution_id, target.as_ref(), result);
        }
        if record.attempt.plan.mission_type_name == "代理人方案培养" {
            return Err("代理人计划不能采用普通挑战回执");
        }
        let category = match record.outcome {
            ChargeOutcome::Agent { .. } => return Err("代理人回执必须保留原计划"),
            ChargeOutcome::HuntSucceeded { .. }
            | ChargeOutcome::HuntInsufficientCharge { .. }
            | ChargeOutcome::HuntWeeklyRewardsAvailable { .. } => "恶名狩猎",
            ChargeOutcome::ExpertSucceeded { .. }
            | ChargeOutcome::ExpertInsufficientCharge { .. } => "专业挑战室",
            ChargeOutcome::PatrolSucceeded { .. }
            | ChargeOutcome::PatrolInsufficientCharge { .. } => "区域巡防",
            ChargeOutcome::Succeeded { .. } | ChargeOutcome::InsufficientCharge { .. } => {
                "实战模拟室"
            }
            ChargeOutcome::SynthesisSucceeded { .. }
            | ChargeOutcome::SynthesisUnavailable { .. } => "合成电池",
        };
        if record.attempt.plan.category_name != category
            || (record.attempt.temporary && category != "实战模拟室")
        {
            return Err("此体力副本没有对应的次数登记证明");
        }
        let execution_id = match record.outcome {
            ChargeOutcome::Agent { .. } => return Err("代理人执行身份缺失"),
            ChargeOutcome::HuntSucceeded { execution_id, .. }
            | ChargeOutcome::HuntInsufficientCharge { execution_id, .. }
            | ChargeOutcome::HuntWeeklyRewardsAvailable { execution_id, .. } => execution_id,
            ChargeOutcome::Succeeded { execution_id, .. }
            | ChargeOutcome::InsufficientCharge { execution_id, .. }
            | ChargeOutcome::SynthesisSucceeded { execution_id, .. }
            | ChargeOutcome::SynthesisUnavailable { execution_id, .. } => execution_id,
            ChargeOutcome::PatrolSucceeded { execution_id, .. }
            | ChargeOutcome::PatrolInsufficientCharge { execution_id, .. }
            | ChargeOutcome::ExpertSucceeded { execution_id, .. }
            | ChargeOutcome::ExpertInsufficientCharge { execution_id, .. } => execution_id,
        };
        if execution_id.is_nil() || !self.executions.insert(execution_id) {
            return Err("体力挑战回执缺少执行身份或已被其他尝试采用");
        }
        match record.outcome {
            ChargeOutcome::Agent { .. } => return Err("代理人结果缺失"),
            ChargeOutcome::HuntSucceeded { charge_after, .. }
            | ChargeOutcome::Succeeded { charge_after, .. }
            | ChargeOutcome::PatrolSucceeded { charge_after, .. }
            | ChargeOutcome::ExpertSucceeded { charge_after, .. } => {
                charge_after.validate()?;
                if (["区域巡防", "恶名狩猎"].contains(&category)
                    && charge_after.required_charge != 60)
                    || (category == "专业挑战室" && charge_after.required_charge != 40)
                    || (category == "实战模拟室"
                        && record
                            .attempt
                            .plan
                            .card_num
                            .is_some_and(|n| u32::from(n) * 20 != charge_after.required_charge))
                {
                    return Err("挑战成功回执重复或实际卡片数量不符");
                }
                if !record.attempt.temporary {
                    let plan = self
                        .progress
                        .config
                        .plan_list
                        .iter_mut()
                        .find(|p| p.plan_id == record.attempt.plan.plan_id)
                        .ok_or("次数登记缺少原计划")?;
                    plan.run_times = plan
                        .run_times
                        .checked_add(1)
                        .filter(|n| *n <= 1_000_000)
                        .ok_or("体力运行次数超出范围")?;
                }
                self.progress.successful_runs += 1;
            }
            ChargeOutcome::HuntInsufficientCharge { charge, .. }
            | ChargeOutcome::InsufficientCharge { charge, .. }
            | ChargeOutcome::PatrolInsufficientCharge { charge, .. }
            | ChargeOutcome::ExpertInsufficientCharge { charge, .. } => {
                charge.validate()?;
                if charge.current_charge >= charge.required_charge
                    || (["区域巡防", "恶名狩猎"].contains(&category)
                        && charge.required_charge != 60)
                    || (category == "专业挑战室" && charge.required_charge != 40)
                    || (category == "实战模拟室"
                        && record
                            .attempt
                            .plan
                            .card_num
                            .is_some_and(|n| u32::from(n) * 20 != charge.required_charge))
                {
                    return Err("实际电量不足回执无效");
                }
                self.shortfall_seen = true;
                if !self.progress.config.skip_plan {
                    self.stopped_for_charge = true;
                } else if !record.attempt.temporary {
                    self.progress
                        .skipped
                        .insert(record.attempt.plan.plan_id.clone());
                }
            }
            ChargeOutcome::HuntWeeklyRewardsAvailable { remaining, .. } => {
                if !(1..=3).contains(&remaining) {
                    return Err("周常阻挡须有实际剩余奖励次数");
                }
                self.weekly_rewards_available = true;
                self.progress
                    .skipped
                    .insert(record.attempt.plan.plan_id.clone());
            }
            ChargeOutcome::SynthesisSucceeded { before, after, .. } => {
                before.validate_after_one(after)?;
                let plan = self
                    .progress
                    .config
                    .plan_list
                    .iter_mut()
                    .find(|p| p.plan_id == record.attempt.plan.plan_id)
                    .ok_or("合成次数登记缺少原计划")?;
                plan.run_times = plan
                    .run_times
                    .checked_add(1)
                    .filter(|n| *n <= 1_000_000)
                    .ok_or("体力运行次数超出范围")?;
                self.progress.successful_runs += 1;
            }
            ChargeOutcome::SynthesisUnavailable { stock, .. } => {
                stock.validate()?;
                self.missing_synthesis_materials = true;
                // 储值电卡或丁尼不足也属于合成失败；本轮跳过该项，不冒充成功。
                self.progress
                    .skipped
                    .insert(record.attempt.plan.plan_id.clone());
            }
        }
        Ok(())
    }
    fn apply_agent(
        &mut self,
        record: &ChargeAttemptRecord,
        execution_id: Uuid,
        target: Option<&AgentTransportTarget>,
        result: &AgentChallengeResult,
    ) -> Result<(), &'static str> {
        let plan = &record.attempt.plan;
        AgentTransportRequest {
            plan: plan.clone(),
            timeout_ms: 1000,
        }
        .validate()?;
        if record.attempt.temporary || plan.category_name == "实战模拟室" {
            return Err("代理人挑战尚未取得此分类的完成证明");
        }
        if execution_id.is_nil() || !self.executions.insert(execution_id) {
            return Err("代理人回执缺少执行身份或执行身份已采用");
        }
        if let Some(target) = target {
            target.validate(&plan.category_name)?;
        }
        if matches!(result, AgentChallengeResult::Unavailable) != target.is_none() {
            return Err("代理人结果缺少本次具体目标或无目标结果混入目标");
        }
        let expected_cost = if plan.category_name == "专业挑战室" {
            40
        } else {
            60
        };
        match result {
            AgentChallengeResult::Completed { charge_after, goal } => {
                charge_after.validate()?;
                if charge_after.required_charge != expected_cost {
                    return Err("代理人完成回执电量不符");
                }
                let original = self
                    .progress
                    .config
                    .plan_list
                    .iter_mut()
                    .find(|p| p.plan_id == plan.plan_id)
                    .ok_or("代理人计次缺少原计划")?;
                original.run_times = original
                    .run_times
                    .checked_add(1)
                    .filter(|n| *n <= 1_000_000)
                    .ok_or("代理人运行次数超出范围")?;
                self.progress.successful_runs += 1;
                match goal {
                    AgentGoalStatus::Achieved => {
                        self.progress.skipped.insert(plan.plan_id.clone());
                    }
                    AgentGoalStatus::NotAchieved => {}
                    AgentGoalStatus::Unknown => {
                        self.needs_verification = true;
                    }
                }
            }
            AgentChallengeResult::Insufficient { charge } => {
                charge.validate()?;
                if charge.required_charge != expected_cost
                    || charge.current_charge >= charge.required_charge
                {
                    return Err("代理人实际电量不足回执无效");
                }
                // 原实现对代理人实际不足直接跳过；预估不足仍按全局 skip_plan 处理。
                self.shortfall_seen = true;
                self.progress.skipped.insert(plan.plan_id.clone());
            }
            AgentChallengeResult::Unavailable => {
                self.agent_targets_unavailable = true;
                self.progress.skipped.insert(plan.plan_id.clone());
            }
            AgentChallengeResult::WeeklyRewardsAvailable { remaining } => {
                if plan.category_name != "恶名狩猎" || !(1..=3).contains(remaining) {
                    return Err("代理人周常阻挡回执无效");
                }
                self.weekly_rewards_available = true;
                self.progress.skipped.insert(plan.plan_id.clone());
            }
        }
        Ok(())
    }
}

impl ChargeSchedule {
    pub fn new(request: ChargeRunRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            schema_version: 1,
            request,
            records: vec![],
            pending: None,
            closure: None,
        })
    }
    pub fn request(&self) -> &ChargeRunRequest {
        &self.request
    }
    pub fn records(&self) -> &[ChargeAttemptRecord] {
        &self.records
    }
    pub fn pending(&self) -> Option<&ChargeAttempt> {
        self.pending.as_ref()
    }
    pub fn final_selection(&self) -> Option<&ChargeSelection> {
        self.closure.as_ref().map(|closed| &closed.selection)
    }
    pub fn validate_successor(&self, previous: &Self) -> Result<(), &'static str> {
        self.validate()?;
        previous.validate()?;
        if self.request != previous.request
            || !self.records.starts_with(&previous.records)
            || (previous.closure.is_some() && self != previous)
        {
            return Err("体力检查点不能回退或改写冻结结果");
        }
        if let Some(attempt) = &previous.pending {
            if self.records.len() == previous.records.len() {
                if self.pending.as_ref() != Some(attempt) {
                    return Err("体力检查点不能撤销未完成意图");
                }
            } else if self.records[previous.records.len()].attempt != *attempt {
                return Err("体力后继回执不属于已保存意图");
            }
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.replay().map(|_| ())
    }
    pub fn progress(&self) -> Result<ChargeProgress, &'static str> {
        Ok(self.replay()?.progress)
    }
    /// 只采用已确认的记录；未执行意图里的整轮扣减不能提前写入用户次数。
    pub fn confirmed_prefix(&self, records: usize) -> Result<Self, &'static str> {
        self.validate()?;
        if records > self.records.len() {
            return Err("体力历史前缀超出已确认记录");
        }
        let mut prefix = self.clone();
        prefix.records.truncate(records);
        prefix.pending = None;
        prefix.closure = None;
        Ok(prefix)
    }
    pub fn prefix_fingerprint(&self, records: usize) -> Result<String, &'static str> {
        use sha2::{Digest, Sha256};
        let prefix = self.confirmed_prefix(records)?;
        let bytes = serde_json::to_vec(&prefix).map_err(|_| "体力历史无法生成指纹")?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }
    fn replay(&self) -> Result<State, &'static str> {
        self.request.validate()?;
        if self.schema_version != 1
            || self.records.len() > MAX_ATTEMPTS
            || (self.pending.is_some() && self.closure.is_some())
            || (self.pending.is_some() && self.records.len() == MAX_ATTEMPTS)
        {
            return Err("体力运行检查点版本或数量无效");
        }
        let mut state = State::new(&self.request);
        for (index, record) in self.records.iter().enumerate() {
            let expected =
                state.select(&self.request, record.attempt.resources, index as u32 + 1)?;
            if expected != ChargeSelection::Ready(Box::new(record.attempt.clone())) {
                return Err("体力运行回执与冻结顺序或计划不符");
            }
            state.apply(record)?;
        }
        if let Some(pending) = &self.pending {
            let expected = state.select(
                &self.request,
                pending.resources,
                self.records.len() as u32 + 1,
            )?;
            if expected != ChargeSelection::Ready(Box::new(pending.clone())) {
                return Err("体力运行意图与冻结顺序或计划不符");
            }
        }
        if let Some(closure) = &self.closure
            && (matches!(closure.selection, ChargeSelection::Ready(_))
                || state.select(
                    &self.request,
                    closure.resources,
                    self.records.len() as u32 + 1,
                )? != closure.selection)
        {
            return Err("体力运行结束证明无效");
        }
        Ok(state)
    }
    pub fn begin(
        &mut self,
        resources: ChargeResources,
        day: &GameDay,
    ) -> Result<ChargeSelection, &'static str> {
        if day != &self.request.context.game_day || self.pending.is_some() || self.closure.is_some()
        {
            return Err("体力运行跨日、已有未完成尝试或已结束");
        }
        let mut state = self.replay()?;
        if self.records.len() >= MAX_ATTEMPTS {
            return Err("体力运行达到 4096 次上限，请开始新任务");
        }
        let selection = state.select(&self.request, resources, self.records.len() as u32 + 1)?;
        match &selection {
            ChargeSelection::Ready(attempt) => self.pending = Some((**attempt).clone()),
            _ => {
                self.closure = Some(Closure {
                    resources,
                    selection: selection.clone(),
                })
            }
        }
        Ok(selection)
    }
    /// 调用方须先取得实际挑战终态与输入释放证明。失败和停止没有成功登记入口。
    pub fn record(
        &mut self,
        attempt: &ChargeAttempt,
        outcome: ChargeOutcome,
    ) -> Result<bool, &'static str> {
        self.validate()?;
        let record = ChargeAttemptRecord {
            attempt: attempt.clone(),
            outcome,
        };
        if let Some(previous) = self
            .records
            .iter()
            .find(|r| r.attempt.sequence == attempt.sequence)
        {
            return if previous == &record {
                Ok(false)
            } else {
                Err("不能覆盖已经登记的体力结果")
            };
        }
        if self.pending.as_ref() != Some(attempt) {
            return Err("体力登记不是当前已保存的尝试");
        }
        let mut next = self.clone();
        next.pending = None;
        next.records.push(record);
        next.validate()?;
        *self = next;
        Ok(true)
    }
}
