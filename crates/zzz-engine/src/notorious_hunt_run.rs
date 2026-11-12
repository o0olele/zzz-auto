//! 周常计划交接与结果提交。只采用活执行器证明，补交历史不重放输入。
use crate::notorious_hunt_challenge::WeeklyHuntChallengeRun;
use anyhow::{Result, ensure};
use od_core::{
    config::{AccountConfig, ConfigStore},
    notorious_hunt_run::{
        WeeklyAttempt, WeeklyAttemptRecord, WeeklyOutcome, WeeklyRunContext, WeeklySchedule,
        WeeklySelection,
    },
    pipeline::PipelineControl,
};

pub struct WeeklyHuntRegistrar {
    schedule: WeeklySchedule,
    control: PipelineControl,
    live_revision: u32,
    pending_commit: Option<WeeklyAttemptRecord>,
}
impl WeeklyHuntRegistrar {
    pub fn new(schedule: WeeklySchedule, control: PipelineControl) -> Result<Self> {
        schedule.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            schedule.records().is_empty()
                && schedule.pending().is_none()
                && schedule.final_selection().is_none(),
            "周常执行只能从新计划启动；历史检查点仅供补交，不恢复输入"
        );
        let live_revision = schedule.request().context.config_revision;
        Ok(Self {
            schedule,
            control,
            live_revision,
            pending_commit: None,
        })
    }
    pub fn snapshot(&self) -> &WeeklySchedule {
        &self.schedule
    }
    pub fn current_context(&self) -> WeeklyRunContext {
        let mut context = self.schedule.request().context.clone();
        context.config_revision = self.live_revision;
        context
    }
    fn check(&self, live: &WeeklyRunContext) -> Result<()> {
        live.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            live == &self.current_context(),
            "周常账号、应用组、修订、运行或游戏日已变化"
        );
        ensure!(
            self.control.requested().is_none(),
            "周常运行正在暂停或停止，不能交接输入"
        );
        Ok(())
    }
    fn check_store(&self, store: &ConfigStore) -> Result<()> {
        let config = store.read()?;
        ensure!(
            config.revision == self.live_revision,
            "周常配置已变化，不能采用旧意图继续输入"
        );
        let source = &self.schedule.request().context;
        let account = config
            .instances
            .iter()
            .find(|a| a.id == source.instance_id)
            .ok_or_else(|| anyhow::anyhow!("周常账号已变化"))?;
        let group = account
            .groups
            .iter()
            .find(|g| g.id == source.group_id)
            .ok_or_else(|| anyhow::anyhow!("周常应用组已变化"))?;
        let expected = if config
            .weekly_hunt_commits
            .iter()
            .any(|r| r.context.run_id == source.run_id)
        {
            self.schedule
                .confirmed_prefix(self.schedule.records().len())
                .and_then(|s| s.progress())
                .map_err(anyhow::Error::msg)?
                .config
        } else {
            self.schedule.request().config.clone()
        };
        ensure!(
            config.active_instance == source.instance_id
                && account.game_region == source.game_day.region
                && account.predefined_teams == self.schedule.request().teams
                && *group.notorious_hunt == expected,
            "周常冻结配置内容已变化，不能仅凭相同修订继续输入"
        );
        Ok(())
    }
    /// 保存意图成功后才交接；暂停期间保存成功的意图在同一运行继续时复用。
    pub fn begin(
        &mut self,
        live: &WeeklyRunContext,
        mut publish: impl FnMut(&WeeklySchedule) -> Result<()>,
    ) -> Result<WeeklySelection> {
        ensure!(
            self.pending_commit.is_none(),
            "周常已完成结果等待保存，不能重新挑战"
        );
        self.check(live)?;
        if let Some(a) = self.schedule.pending() {
            return Ok(WeeklySelection::Ready(Box::new(a.clone())));
        }
        let mut next = self.schedule.clone();
        let selection = next.begin(&live.game_day).map_err(anyhow::Error::msg)?;
        publish(&next)?;
        self.schedule = next;
        self.check(live)?;
        Ok(selection)
    }
    pub fn begin_persisted(
        &mut self,
        live: &WeeklyRunContext,
        store: &ConfigStore,
    ) -> Result<WeeklySelection> {
        self.check_store(store)?;
        if self.schedule.pending().is_none()
            && self.schedule.records().is_empty()
            && self.schedule.final_selection().is_none()
        {
            ensure!(
                store
                    .weekly_hunt_checkpoint(self.schedule.request().context.run_id)?
                    .is_none(),
                "这个周常运行已有磁盘历史；新执行器不能重放旧输入意图"
            );
        }
        let mut revision = None;
        let result = self.begin(live, |s| {
            let committed = store.commit_weekly_hunt_run(s)?;
            revision = Some(verify_commit_revision(s, &committed.config)?);
            Ok(())
        });
        if let Some(r) = revision {
            self.live_revision = r;
        }
        result
    }
    /// 已完成并释放输入的事实必须先登记；迟到的暂停或停止只阻止下一次输入。
    pub fn record_challenge(
        &mut self,
        attempt: &WeeklyAttempt,
        challenge: &WeeklyHuntChallengeRun,
        publish: impl FnMut(&WeeklySchedule) -> Result<()>,
    ) -> Result<bool> {
        let s = challenge.snapshot();
        ensure!(
            challenge.uses_control(&self.control)
                && s.id == self.schedule.request().context.run_id
                && attempt.run_id == s.id
                && s.request.plan == attempt.plan
                && s.request.teams == self.schedule.request().teams
                && s.request
                    .plan
                    .script(&s.request.teams)
                    .map_err(anyhow::Error::msg)?
                    == attempt.script,
            "周常挑战与冻结计划、编队或控制句柄不符"
        );
        let outcome = challenge
            .outcome_for_parent()
            .ok_or_else(|| anyhow::anyhow!("周常挑战尚无正常完成并释放输入的本次结果"))?;
        self.record_outcome(attempt, outcome, publish)
    }
    pub fn record_persisted(
        &mut self,
        attempt: &WeeklyAttempt,
        challenge: &WeeklyHuntChallengeRun,
        store: &ConfigStore,
    ) -> Result<bool> {
        let mut revision = None;
        let result = self.record_challenge(attempt, challenge, |s| {
            let committed = store.commit_weekly_hunt_run(s)?;
            revision = Some(verify_commit_revision(s, &committed.config)?);
            Ok(())
        });
        if let Some(r) = revision {
            self.live_revision = r;
        }
        result
    }
    pub fn record_agent_persisted(
        &mut self,
        attempt: &WeeklyAttempt,
        agent: &crate::notorious_hunt_agent::WeeklyAgentRun,
        store: &ConfigStore,
    ) -> Result<bool> {
        let s = agent.snapshot();
        ensure!(
            agent.uses_control(&self.control)
                && s.id == self.schedule.request().context.run_id
                && attempt.run_id == s.id
                && s.request.plan == attempt.plan
                && s.request.teams == self.schedule.request().teams
                && s.request
                    .plan
                    .script(&s.request.teams)
                    .map_err(anyhow::Error::msg)?
                    == attempt.script,
            "周常培养与冻结原计划、编队或控制句柄不符"
        );
        let outcome = agent
            .outcome_for_parent()
            .ok_or_else(|| anyhow::anyhow!("周常培养尚无本次已保存并释放输入的结果"))?;
        let mut revision = None;
        let result = self.record_outcome(attempt, outcome, |s| {
            let committed = store.commit_weekly_hunt_run(s)?;
            revision = Some(verify_commit_revision(s, &committed.config)?);
            Ok(())
        });
        if let Some(r) = revision {
            self.live_revision = r;
        }
        result
    }
    fn record_outcome(
        &mut self,
        attempt: &WeeklyAttempt,
        outcome: WeeklyOutcome,
        mut publish: impl FnMut(&WeeklySchedule) -> Result<()>,
    ) -> Result<bool> {
        let record = WeeklyAttemptRecord {
            attempt: attempt.clone(),
            outcome: outcome.clone(),
        };
        ensure!(
            self.pending_commit.as_ref().is_none_or(|p| p == &record),
            "待保存的周常结果不能换成另一次结果"
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
        Ok(changed)
    }
    pub fn has_pending_commit(&self) -> bool {
        self.pending_commit.is_some()
    }
    /// 不受暂停/停止影响，也不发送输入；配置冲突时保留已确认日志，禁止后继挑战。
    pub fn retry_pending_persisted(&mut self, store: &ConfigStore) -> Result<bool> {
        let record = self
            .pending_commit
            .clone()
            .ok_or_else(|| anyhow::anyhow!("没有待补交的周常结果"))?;
        let mut next = self.schedule.clone();
        let changed = next
            .record(&record.attempt, record.outcome)
            .map_err(anyhow::Error::msg)?;
        let committed = store.commit_weekly_hunt_run(&next)?;
        self.live_revision = verify_commit_revision(&next, &committed.config)?;
        self.schedule = next;
        self.pending_commit = None;
        Ok(changed)
    }
}
fn verify_commit_revision(schedule: &WeeklySchedule, config: &AccountConfig) -> Result<u32> {
    let source = &schedule.request().context;
    let expected = config
        .weekly_hunt_commits
        .iter()
        .find(|c| c.context.run_id == source.run_id)
        .map_or(source.config_revision, |c| c.saved_revision);
    ensure!(
        config.revision == expected,
        "周常结果已经提交，但后续配置已变化，不能继续本次输入"
    );
    Ok(expected)
}
