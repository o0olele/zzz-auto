//! 连接实际模拟室终态与体力次数检查点；公开运行入口仍等待关卡证明生产者。
use crate::{ether_synthesis::SynthesisRun, simulation_challenge::SimulationChallengeRun};
use anyhow::{Result, ensure};
use od_core::{
    charge_plan::ChargeResources,
    charge_run::{
        ChargeAttempt, ChargeAttemptRecord, ChargeOutcome, ChargeRunContext, ChargeSchedule,
        ChargeSelection,
    },
    pipeline::PipelineControl,
};

pub struct ChargeRegistrar {
    schedule: ChargeSchedule,
    control: PipelineControl,
    live_revision: u32,
    pending_commit: Option<ChargeAttemptRecord>,
}
impl ChargeRegistrar {
    /// 已取得正常返回或确定阻挡后，先保存原计划事实，再处理迟到的控制请求。
    pub(crate) fn record_completed_agent_persisted(
        &mut self,
        agent: &crate::agent_challenge::AgentChallengeRun,
        store: &od_core::config::ConfigStore,
    ) -> Result<bool> {
        let attempt = self
            .schedule
            .pending()
            .ok_or_else(|| anyhow::anyhow!("代理人体力尝试缺失"))?
            .clone();
        let snapshot = agent.snapshot();
        let proof = agent
            .result_for_parent()
            .ok_or_else(|| anyhow::anyhow!("代理人尚无已释放输入的本次结果"))?;
        ensure!(
            agent.uses_control(&self.control)
                && proof.run_id == self.schedule.request().context.run_id
                && snapshot.id == proof.run_id
                && proof.plan == attempt.plan
                && snapshot.request.plan == attempt.plan
                && snapshot.request.teams == self.schedule.request().teams
                && snapshot.request.restore_charge == self.schedule.request().config.restore_charge,
            "代理人结果与本次原始体力尝试或控制句柄不符"
        );
        let outcome = ChargeOutcome::Agent {
            execution_id: proof.execution_id,
            target: proof.target,
            result: proof.result,
        };
        let mut revision = None;
        let result = self.record_outcome(&attempt, outcome, |schedule| {
            let committed = store.commit_charge_run(schedule)?;
            revision = Some(verify_commit_revision(schedule, &committed.config)?);
            Ok(())
        });
        if let Some(r) = revision {
            self.live_revision = r;
        }
        result
    }
    pub fn new(schedule: ChargeSchedule, control: PipelineControl) -> Result<Self> {
        schedule.validate().map_err(anyhow::Error::msg)?;
        let live_revision = schedule.request().context.config_revision;
        Ok(Self {
            schedule,
            control,
            live_revision,
            pending_commit: None,
        })
    }
    pub fn snapshot(&self) -> &ChargeSchedule {
        &self.schedule
    }
    pub fn current_context(&self) -> ChargeRunContext {
        let mut context = self.schedule.request().context.clone();
        context.config_revision = self.live_revision;
        context
    }
    fn check(&self, live: &ChargeRunContext) -> Result<()> {
        live.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            live == &self.current_context(),
            "体力账号、应用组、修订、运行或游戏日已变化"
        );
        ensure!(
            self.control.requested().is_none(),
            "体力运行正在暂停或停止，不能交接或登记"
        );
        Ok(())
    }
    /// 持久化路径维护自己提交造成的新修订，冻结请求仍保留启动修订用于历史复验。
    pub fn begin_persisted(
        &mut self,
        live: &ChargeRunContext,
        resources: ChargeResources,
        store: &od_core::config::ConfigStore,
    ) -> Result<ChargeSelection> {
        self.check_store(store)?;
        let mut revision = None;
        let result = self.begin(live, resources, |schedule| {
            let committed = store.commit_charge_run(schedule)?;
            revision = Some(verify_commit_revision(schedule, &committed.config)?);
            Ok(())
        });
        if let Some(revision) = revision {
            self.live_revision = revision;
        }
        result
    }
    pub fn record_persisted(
        &mut self,
        live: &ChargeRunContext,
        attempt: &ChargeAttempt,
        challenge: &SimulationChallengeRun,
        store: &od_core::config::ConfigStore,
    ) -> Result<bool> {
        let mut revision = None;
        let result = self.record_challenge(live, attempt, challenge, |schedule| {
            let committed = store.commit_charge_run(schedule)?;
            revision = Some(verify_commit_revision(schedule, &committed.config)?);
            Ok(())
        });
        if let Some(revision) = revision {
            self.live_revision = revision;
        }
        result
    }
    pub fn record_patrol_persisted(
        &mut self,
        live: &ChargeRunContext,
        attempt: &ChargeAttempt,
        patrol: &crate::area_patrol::AreaPatrolRun,
        store: &od_core::config::ConfigStore,
    ) -> Result<bool> {
        self.check(live)?;
        let snapshot = patrol.snapshot();
        ensure!(
            patrol.uses_control(&self.control)
                && snapshot.id == live.run_id
                && snapshot.request.plan == attempt.plan
                && snapshot.request.teams == self.schedule.request().teams
                && snapshot.request.restore_charge == self.schedule.request().config.restore_charge,
            "区域巡防与冻结体力尝试或控制句柄不一致"
        );
        let outcome = if let Some(proof) = patrol.proof() {
            ensure!(
                proof.run_id == live.run_id && proof.plan == attempt.plan,
                "区域巡防成功证明不属于本次尝试"
            );
            ChargeOutcome::PatrolSucceeded {
                execution_id: proof.execution_id,
                charge_after: proof.charge_after,
            }
        } else if let Some(charge) = patrol.shortage() {
            ChargeOutcome::PatrolInsufficientCharge {
                execution_id: patrol.execution_id(),
                charge,
            }
        } else {
            anyhow::bail!("区域巡防尚无正常完成或电量不足终态");
        };
        let mut revision = None;
        let result = self.record_outcome(attempt, outcome, |s| {
            let committed = store.commit_charge_run(s)?;
            revision = Some(verify_commit_revision(s, &committed.config)?);
            Ok(())
        });
        if let Some(r) = revision {
            self.live_revision = r;
        }
        result
    }
    pub fn record_expert_persisted(
        &mut self,
        live: &ChargeRunContext,
        attempt: &ChargeAttempt,
        expert: &crate::expert_challenge::ExpertChallengeRun,
        store: &od_core::config::ConfigStore,
    ) -> Result<bool> {
        self.check(live)?;
        let snapshot = expert.snapshot();
        ensure!(
            expert.uses_control(&self.control)
                && snapshot.id == live.run_id
                && snapshot.request.plan == attempt.plan
                && snapshot.request.teams == self.schedule.request().teams
                && snapshot.request.restore_charge == self.schedule.request().config.restore_charge,
            "专业挑战室与冻结体力尝试或控制句柄不一致"
        );
        let outcome = if let Some(proof) = expert.proof() {
            ensure!(
                proof.run_id == live.run_id && proof.plan == attempt.plan,
                "专业挑战室成功证明不属于本次尝试"
            );
            ChargeOutcome::ExpertSucceeded {
                execution_id: proof.execution_id,
                charge_after: proof.charge_after,
            }
        } else if let Some(charge) = expert.shortage() {
            ChargeOutcome::ExpertInsufficientCharge {
                execution_id: expert.execution_id(),
                charge,
            }
        } else {
            anyhow::bail!("专业挑战室尚无正常完成或电量不足终态");
        };
        let mut revision = None;
        let result = self.record_outcome(attempt, outcome, |s| {
            let committed = store.commit_charge_run(s)?;
            revision = Some(verify_commit_revision(s, &committed.config)?);
            Ok(())
        });
        if let Some(r) = revision {
            self.live_revision = r;
        }
        result
    }
    pub fn record_hunt_persisted(
        &mut self,
        live: &ChargeRunContext,
        attempt: &ChargeAttempt,
        hunt: &crate::deep_hunt::DeepHuntRun,
        store: &od_core::config::ConfigStore,
    ) -> Result<bool> {
        self.check(live)?;
        let snapshot = hunt.snapshot();
        ensure!(
            hunt.uses_control(&self.control)
                && snapshot.id == live.run_id
                && snapshot.request.plan == attempt.plan
                && snapshot.request.teams == self.schedule.request().teams
                && snapshot.request.restore_charge == self.schedule.request().config.restore_charge,
            "深度追猎与冻结体力尝试或控制句柄不一致"
        );
        let outcome = if let Some(proof) = hunt.proof() {
            ensure!(
                proof.run_id == live.run_id && proof.plan == attempt.plan,
                "深度追猎成功证明不属于本次尝试"
            );
            ChargeOutcome::HuntSucceeded {
                execution_id: proof.execution_id,
                charge_after: proof.charge_after,
            }
        } else if let Some(charge) = hunt.shortage() {
            ChargeOutcome::HuntInsufficientCharge {
                execution_id: hunt.execution_id(),
                charge,
            }
        } else if let Some(remaining) = hunt.weekly_rewards() {
            ChargeOutcome::HuntWeeklyRewardsAvailable {
                execution_id: hunt.execution_id(),
                remaining,
            }
        } else {
            anyhow::bail!("深度追猎尚无正常完成或电量不足终态");
        };
        let mut revision = None;
        let result = self.record_outcome(attempt, outcome, |s| {
            let committed = store.commit_charge_run(s)?;
            revision = Some(verify_commit_revision(s, &committed.config)?);
            Ok(())
        });
        if let Some(r) = revision {
            self.live_revision = r;
        }
        result
    }
    pub fn record_synthesis_persisted(
        &mut self,
        live: &ChargeRunContext,
        attempt: &ChargeAttempt,
        synthesis: &SynthesisRun,
        store: &od_core::config::ConfigStore,
    ) -> Result<bool> {
        let mut revision = None;
        let result = self.record_synthesis(live, attempt, synthesis, |schedule| {
            let committed = store.commit_charge_run(schedule)?;
            revision = Some(verify_commit_revision(schedule, &committed.config)?);
            Ok(())
        });
        if let Some(revision) = revision {
            self.live_revision = revision;
        }
        result
    }
    /// 只登记父任务已经取得的正常区域终态；迟到控制请求不能撤销已完成的消费。
    pub(crate) fn record_completed_patrol_persisted(
        &mut self,
        patrol: &crate::area_patrol::AreaPatrolRun,
        store: &od_core::config::ConfigStore,
    ) -> Result<bool> {
        let attempt = self
            .schedule
            .pending()
            .ok_or_else(|| anyhow::anyhow!("体力尝试缺失"))?
            .clone();
        let s = patrol.snapshot();
        ensure!(
            patrol.uses_control(&self.control)
                && s.id == self.schedule.request().context.run_id
                && s.request.plan == attempt.plan
                && s.request.teams == self.schedule.request().teams
                && s.request.restore_charge == self.schedule.request().config.restore_charge,
            "区域结果与本次冻结尝试或控制句柄不符"
        );
        let outcome = if let Some(p) = patrol.proof_for_parent() {
            ChargeOutcome::PatrolSucceeded {
                execution_id: p.execution_id,
                charge_after: p.charge_after,
            }
        } else if let Some(charge) = patrol.shortage_for_parent() {
            ChargeOutcome::PatrolInsufficientCharge {
                execution_id: patrol.execution_id(),
                charge,
            }
        } else {
            anyhow::bail!("区域尚无正常保存和释放的本次结果");
        };
        let mut revision = None;
        let result = self.record_outcome(&attempt, outcome, |schedule| {
            let committed = store.commit_charge_run(schedule)?;
            revision = Some(verify_commit_revision(schedule, &committed.config)?);
            Ok(())
        });
        if let Some(r) = revision {
            self.live_revision = r;
        }
        result
    }
    pub(crate) fn record_completed_expert_persisted(
        &mut self,
        expert: &crate::expert_challenge::ExpertChallengeRun,
        store: &od_core::config::ConfigStore,
    ) -> Result<bool> {
        let attempt = self
            .schedule
            .pending()
            .ok_or_else(|| anyhow::anyhow!("体力尝试缺失"))?
            .clone();
        let s = expert.snapshot();
        ensure!(
            expert.uses_control(&self.control)
                && s.id == self.schedule.request().context.run_id
                && s.request.plan == attempt.plan
                && s.request.teams == self.schedule.request().teams
                && s.request.restore_charge == self.schedule.request().config.restore_charge,
            "专业结果与本次冻结尝试或控制句柄不符"
        );
        let outcome = if let Some(p) = expert.proof_for_parent() {
            ChargeOutcome::ExpertSucceeded {
                execution_id: p.execution_id,
                charge_after: p.charge_after,
            }
        } else if let Some(charge) = expert.shortage_for_parent() {
            ChargeOutcome::ExpertInsufficientCharge {
                execution_id: expert.execution_id(),
                charge,
            }
        } else {
            anyhow::bail!("专业尚无正常保存和释放的本次结果");
        };
        let mut revision = None;
        let result = self.record_outcome(&attempt, outcome, |schedule| {
            let committed = store.commit_charge_run(schedule)?;
            revision = Some(verify_commit_revision(schedule, &committed.config)?);
            Ok(())
        });
        if let Some(r) = revision {
            self.live_revision = r;
        }
        result
    }
    pub(crate) fn record_completed_hunt_persisted(
        &mut self,
        hunt: &crate::deep_hunt::DeepHuntRun,
        store: &od_core::config::ConfigStore,
    ) -> Result<bool> {
        let attempt = self
            .schedule
            .pending()
            .ok_or_else(|| anyhow::anyhow!("体力尝试缺失"))?
            .clone();
        let s = hunt.snapshot();
        ensure!(
            hunt.uses_control(&self.control)
                && s.id == self.schedule.request().context.run_id
                && s.request.plan == attempt.plan
                && s.request.teams == self.schedule.request().teams
                && s.request.restore_charge == self.schedule.request().config.restore_charge,
            "追猎结果与本次冻结尝试或控制句柄不符"
        );
        let outcome = if let Some(p) = hunt.proof_for_parent() {
            ChargeOutcome::HuntSucceeded {
                execution_id: p.execution_id,
                charge_after: p.charge_after,
            }
        } else if let Some(charge) = hunt.shortage_for_parent() {
            ChargeOutcome::HuntInsufficientCharge {
                execution_id: hunt.execution_id(),
                charge,
            }
        } else if let Some(remaining) = hunt.weekly_for_parent() {
            ChargeOutcome::HuntWeeklyRewardsAvailable {
                execution_id: hunt.execution_id(),
                remaining,
            }
        } else {
            anyhow::bail!("追猎尚无正常保存和释放的本次结果");
        };
        let mut revision = None;
        let result = self.record_outcome(&attempt, outcome, |schedule| {
            let committed = store.commit_charge_run(schedule)?;
            revision = Some(verify_commit_revision(schedule, &committed.config)?);
            Ok(())
        });
        if let Some(r) = revision {
            self.live_revision = r;
        }
        result
    }
    pub fn record_synthesis_challenge_persisted(
        &mut self,
        live: &ChargeRunContext,
        attempt: &ChargeAttempt,
        challenge: &crate::ether_synthesis_challenge::SynthesisChallengeRun,
        store: &od_core::config::ConfigStore,
    ) -> Result<bool> {
        self.record_synthesis_persisted(live, attempt, challenge.verified_exchange()?, store)
    }
    /// 消费已经完成，后来的控制请求或配置变化只能阻止后继输入，不能撤销日志证据。
    pub(crate) fn record_completed_synthesis_persisted(
        &mut self,
        challenge: &crate::ether_synthesis_challenge::SynthesisChallengeRun,
        store: &od_core::config::ConfigStore,
    ) -> Result<bool> {
        let synthesis = challenge.exchange_for_registration()?;
        let attempt = self
            .schedule
            .pending()
            .ok_or_else(|| anyhow::anyhow!("体力尝试缺失"))?
            .clone();
        ensure!(
            synthesis.id() == self.schedule.request().context.run_id
                && synthesis.uses_control(&self.control)
                && synthesis.snapshot().flow.request().plan == attempt.plan,
            "合成登记与体力运行或控制句柄不符"
        );
        use od_core::ether_synthesis::SynthesisResult;
        let outcome = match synthesis.result_for_parent()? {
            SynthesisResult::Succeeded { before, after } => ChargeOutcome::SynthesisSucceeded {
                execution_id: synthesis.execution_id(),
                before,
                after,
            },
            SynthesisResult::Unavailable { stock } => ChargeOutcome::SynthesisUnavailable {
                execution_id: synthesis.execution_id(),
                stock,
            },
        };
        let mut revision = None;
        let result = self.record_outcome(&attempt, outcome, |schedule| {
            let committed = store.commit_charge_run(schedule)?;
            revision = Some(verify_commit_revision(schedule, &committed.config)?);
            Ok(())
        });
        if let Some(revision) = revision {
            self.live_revision = revision;
        }
        result
    }
    pub(crate) fn check_store(&self, store: &od_core::config::ConfigStore) -> Result<()> {
        let config = store.read()?;
        ensure!(
            config.revision == self.live_revision,
            "体力配置已变化，不能采用旧意图继续输入"
        );
        let source = &self.schedule.request().context;
        let account = config
            .instances
            .iter()
            .find(|a| a.id == source.instance_id)
            .ok_or_else(|| anyhow::anyhow!("体力账号已变化"))?;
        let group = account
            .groups
            .iter()
            .find(|g| g.id == source.group_id)
            .ok_or_else(|| anyhow::anyhow!("体力应用组已变化"))?;
        let expected = if config
            .charge_commits
            .iter()
            .any(|r| r.context.run_id == source.run_id)
        {
            self.schedule
                .confirmed_prefix(self.schedule.records().len())
                .map_err(anyhow::Error::msg)?
                .progress()
                .map_err(anyhow::Error::msg)?
                .config
        } else {
            self.schedule.request().config.clone()
        };
        ensure!(
            config.active_instance == source.instance_id
                && account.game_region == source.game_day.region
                && account.predefined_teams == self.schedule.request().teams
                && *group.charge_plan == expected,
            "体力冻结配置内容已变化，不能仅凭相同修订继续输入"
        );
        Ok(())
    }
    /// 保存失败后只重试此前已经取得的结果；暂停或停止期间也可保存，绝不发送输入。
    pub fn retry_pending_persisted(
        &mut self,
        store: &od_core::config::ConfigStore,
    ) -> Result<bool> {
        let record = self
            .pending_commit
            .clone()
            .ok_or_else(|| anyhow::anyhow!("没有待重试的已确认体力结果"))?;
        let mut next = self.schedule.clone();
        let changed = next
            .record(&record.attempt, record.outcome)
            .map_err(anyhow::Error::msg)?;
        let committed = store.commit_charge_run(&next)?;
        self.live_revision = verify_commit_revision(&next, &committed.config)?;
        self.schedule = next;
        self.pending_commit = None;
        Ok(changed)
    }
    pub fn has_pending_commit(&self) -> bool {
        self.pending_commit.is_some()
    }
    pub(crate) fn history_schedule(&self) -> Result<ChargeSchedule> {
        let mut schedule = self.schedule.clone();
        if let Some(pending) = &self.pending_commit {
            schedule
                .record(&pending.attempt, pending.outcome.clone())
                .map_err(anyhow::Error::msg)?;
        }
        Ok(schedule)
    }
    /// 保存成功才返回输入许可。保存期间暂停时保留已保存意图，继续后可重新取出；
    /// challenge 自身仍须用新帧核对关卡、电量和编队。
    pub fn begin(
        &mut self,
        live: &ChargeRunContext,
        resources: ChargeResources,
        mut publish: impl FnMut(&ChargeSchedule) -> Result<()>,
    ) -> Result<ChargeSelection> {
        ensure!(
            self.pending_commit.is_none(),
            "已有已完成挑战等待保存，请重试登记，不能重新挑战"
        );
        self.check(live)?;
        if let Some(attempt) = self.schedule.pending() {
            return Ok(ChargeSelection::Ready(Box::new(attempt.clone())));
        }
        let mut next = self.schedule.clone();
        let selection = next
            .begin(resources, &live.game_day)
            .map_err(anyhow::Error::msg)?;
        publish(&next)?;
        self.schedule = next;
        self.check(live)?;
        Ok(selection)
    }
    /// 只从仍持有本次终态的执行器取证明；快照或手工布尔值不能登记成功。
    /// publish 失败保持原内存状态，调用方应重试登记，禁止重跑挑战。
    pub fn record_challenge(
        &mut self,
        live: &ChargeRunContext,
        attempt: &ChargeAttempt,
        challenge: &SimulationChallengeRun,
        publish: impl FnMut(&ChargeSchedule) -> Result<()>,
    ) -> Result<bool> {
        self.check(live)?;
        let request = &challenge.snapshot().request;
        ensure!(
            challenge.uses_control(&self.control)
                && challenge.snapshot().id == live.run_id
                && request.plan == attempt.plan
                && request.teams == self.schedule.request().teams
                && request.restore_charge == self.schedule.request().config.restore_charge,
            "挑战与冻结体力尝试或控制句柄不符"
        );
        let outcome = if let Some(proof) = challenge.proof() {
            ensure!(
                proof.run_id == live.run_id
                    && proof.battle.run_id == live.run_id
                    && proof.plan == attempt.plan
                    && proof.battle.mission == request.mission,
                "体力挑战成功证明不属于本次尝试"
            );
            ChargeOutcome::Succeeded {
                execution_id: proof.execution_id,
                charge_after: proof.battle.charge_after,
            }
        } else if let Some(shortage) = challenge.shortage() {
            ChargeOutcome::InsufficientCharge {
                execution_id: challenge.execution_id(),
                charge: shortage.charge,
            }
        } else {
            anyhow::bail!("挑战尚无正常成功或电量不足终态，不能登记次数");
        };
        self.record_outcome(attempt, outcome, publish)
    }
    pub fn record_synthesis(
        &mut self,
        live: &ChargeRunContext,
        attempt: &ChargeAttempt,
        synthesis: &SynthesisRun,
        publish: impl FnMut(&ChargeSchedule) -> Result<()>,
    ) -> Result<bool> {
        self.check(live)?;
        ensure!(
            synthesis.id() == live.run_id
                && synthesis.uses_control(&self.control)
                && synthesis.snapshot().flow.request().plan == attempt.plan,
            "合成与冻结体力尝试或控制句柄不符"
        );
        use od_core::ether_synthesis::SynthesisResult;
        let outcome = match synthesis.proof()? {
            SynthesisResult::Succeeded { before, after } => ChargeOutcome::SynthesisSucceeded {
                execution_id: synthesis.execution_id(),
                before,
                after,
            },
            SynthesisResult::Unavailable { stock } => ChargeOutcome::SynthesisUnavailable {
                execution_id: synthesis.execution_id(),
                stock,
            },
        };
        self.record_outcome(attempt, outcome, publish)
    }
    fn record_outcome(
        &mut self,
        attempt: &ChargeAttempt,
        outcome: ChargeOutcome,
        mut publish: impl FnMut(&ChargeSchedule) -> Result<()>,
    ) -> Result<bool> {
        let record = ChargeAttemptRecord {
            attempt: attempt.clone(),
            outcome: outcome.clone(),
        };
        ensure!(
            self.pending_commit
                .as_ref()
                .is_none_or(|previous| previous == &record),
            "待保存的挑战结果不能换成另一次结果"
        );
        let mut next = self.schedule.clone();
        let changed = next.record(attempt, outcome).map_err(anyhow::Error::msg)?;
        if changed {
            if let Err(error) = publish(&next) {
                self.pending_commit = Some(record);
                return Err(error);
            }
            self.schedule = next;
            self.pending_commit = None;
        }
        // 此时只提交已取得的结果；即使保存期间收到控制请求，begin 也禁止后继。
        Ok(changed)
    }
}

fn verify_commit_revision(
    schedule: &ChargeSchedule,
    config: &od_core::config::AccountConfig,
) -> Result<u32> {
    let source = &schedule.request().context;
    let expected = config
        .charge_commits
        .iter()
        .find(|c| c.context.run_id == source.run_id)
        .map_or(source.config_revision, |c| c.saved_revision);
    ensure!(
        config.revision == expected,
        "体力结果已经提交，但后续配置已变化，不能继续本次输入"
    );
    Ok(expected)
}
