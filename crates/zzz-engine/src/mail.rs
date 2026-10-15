//! 邮件业务检查点；原生输入、停止与释放由适配层统一处理。
use anyhow::{Result, ensure};
use maa_adapter::{MailStep, NativeSession, PipelineEnd, PipelineExecution};
use od_core::mail::{MailConfig, MailProgress};
pub use od_core::mail::{MailPhase, MailState};
use od_core::pipeline::{Interrupt, PipelineControl, PipelineRequest};
use serde::Serialize;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Serialize)]
pub struct MailRun {
    pub id: Uuid,
    pub state: MailState,
    pub phase: MailPhase,
    pub claim_attempted: bool,
    pub reward_observed: bool,
    pub returning: bool,
    pub inputs_released: bool,
    pub active_ms: u64,
    pub message: String,
    pub executions: Vec<PipelineExecution>,
    timeout_ms: u64,
    config: MailConfig,
    #[serde(skip)]
    control: PipelineControl,
}

impl MailRun {
    pub fn new(timeout: Duration) -> Result<Self> {
        ensure!(
            timeout >= Duration::from_millis(1) && timeout <= Duration::from_secs(60),
            "邮件超时须为 1ms..60s"
        );
        Self::with_id(
            Uuid::new_v4(),
            MailConfig {
                timeout_ms: timeout.as_millis() as u32,
                start_delay_ms: 0,
            },
        )
    }

    pub fn with_id(id: Uuid, config: MailConfig) -> Result<Self> {
        config.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            id,
            state: MailState::Running,
            phase: MailPhase::Navigate,
            claim_attempted: false,
            reward_observed: false,
            returning: false,
            inputs_released: true,
            active_ms: 0,
            timeout_ms: config.timeout_ms.into(),
            config,
            message: "邮件流程已创建".into(),
            executions: Vec::new(),
            control: PipelineControl::default(),
        })
    }

    pub fn control(&self) -> PipelineRequest {
        self.control.requester()
    }

    pub fn interrupt_requested(&self) -> Option<Interrupt> {
        self.control.requested()
    }
    // 仅供工作线程在确认控制器已释放后，处理投递前的暂停 / 停止。
    pub fn acknowledge_idle(&mut self) -> bool {
        self.inputs_released = true;
        self.interrupt()
    }

    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.state == MailState::Paused,
            "只有已完成输入收尾的暂停任务可以继续"
        );
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.state = MailState::Running;
        // 保留本次运行 ID、领取检查点和有效耗时，从新截图恢复，不重跑业务入口。
        self.phase = MailPhase::Inspect;
        self.message = "正在重新识别邮件步骤".into();
        Ok(())
    }

    pub fn execute(&mut self, session: &mut NativeSession) -> Result<()> {
        self.execute_observed(session, false, |_| {})
    }

    pub fn execute_fixture(
        &mut self,
        session: &mut NativeSession,
        progress: impl FnMut(MailProgress),
    ) -> Result<()> {
        self.execute_observed(session, false, progress)
    }

    pub fn progress(&self) -> MailProgress {
        MailProgress {
            config: self.config,
            state: self.state,
            phase: self.phase,
            claim_attempted: self.claim_attempted,
            reward_observed: self.reward_observed,
            inputs_released: self.inputs_released,
            active_ms: self.active_ms,
        }
    }

    pub fn execute_game(
        &mut self,
        session: &mut NativeSession,
        progress: impl FnMut(MailProgress),
    ) -> Result<()> {
        self.execute_observed(session, true, progress)
    }

    fn execute_observed(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        mut progress: impl FnMut(MailProgress),
    ) -> Result<()> {
        ensure!(
            self.state == MailState::Running
                || (self.state == MailState::Paused
                    && self.control.requested() == Some(Interrupt::Stop)),
            "任务已暂停或结束，不能直接重跑"
        );
        self.inputs_released = false;
        let preparation = if game {
            session.prepare_game_mail(self.control.cancellation(), self.config.start_delay_ms)
        } else {
            session.prepare_fixture_mail()
        };
        let started = Instant::now();
        let result = preparation.and_then(|()| {
            self.inputs_released = true;
            self.execute_inner(session, started, game, &mut progress)
        });
        self.active_ms = self
            .active_ms
            .saturating_add(started.elapsed().as_millis() as u64);
        if let Err(error) = &result {
            self.control.finish();
            self.state = MailState::Failed;
            self.message = format!("邮件流程或输入收尾失败：{error:#}");
        }
        result
    }

    fn interrupt(&mut self) -> bool {
        match self.control.requested() {
            Some(Interrupt::Stop) => {
                self.control.finish();
                self.state = MailState::Stopped;
                self.message = "邮件流程已停止，输入已释放".into();
                true
            }
            Some(Interrupt::Pause) => {
                if !self.control.acknowledge_pause() {
                    return self.interrupt();
                }
                self.state = MailState::Paused;
                self.message = "邮件流程已暂停，输入已释放并保留领取检查点".into();
                true
            }
            None => false,
        }
    }

    fn terminal(&mut self, state: MailState, message: &str) {
        let interrupt = self.control.finish();
        // 已请求停止时，原生成功不能覆盖主动停止。
        if interrupt == Some(Interrupt::Stop) {
            self.state = MailState::Stopped;
            self.message = "邮件流程已停止，输入已释放".into();
        } else {
            self.state = state;
            self.message = message.into();
        }
    }

    fn execute_inner(
        &mut self,
        session: &mut NativeSession,
        started: Instant,
        game: bool,
        progress: &mut impl FnMut(MailProgress),
    ) -> Result<()> {
        loop {
            if self.interrupt() {
                return Ok(());
            }
            let used = self
                .active_ms
                .saturating_add(started.elapsed().as_millis() as u64);
            if used >= self.timeout_ms {
                self.terminal(MailState::TimedOut, "邮件流程超时，输入已释放");
                return Ok(());
            }
            let step = match self.phase {
                MailPhase::Navigate => MailStep::Navigate,
                MailPhase::Inspect => MailStep::Inspect,
                MailPhase::Claim => MailStep::Claim,
                MailPhase::AfterClaim => MailStep::AfterClaim,
                MailPhase::Confirm => MailStep::Confirm,
                MailPhase::AfterConfirm => MailStep::AfterConfirm,
                MailPhase::ReturnList => MailStep::ReturnList,
                MailPhase::ReturnWorld => MailStep::ReturnWorld,
                MailPhase::Done => {
                    self.terminal(MailState::Succeeded, "邮件流程完成，已返回大世界并释放输入");
                    return Ok(());
                }
            };
            let was_claim = self.phase == MailPhase::Claim;
            if was_claim {
                self.claim_attempted = true;
                self.phase = MailPhase::AfterClaim;
            }
            if self.phase == MailPhase::ReturnList {
                self.returning = true;
            }
            self.inputs_released = false;
            let mut current = self.progress();
            current.active_ms = used;
            progress(current);
            let execution = if game {
                session.game_mail_step(
                    step,
                    self.control.cancellation(),
                    Duration::from_millis(self.timeout_ms - used),
                )?
            } else {
                session.fixture_mail_step(
                    step,
                    self.control.cancellation(),
                    Duration::from_millis(self.timeout_ms - used),
                )?
            };
            self.inputs_released = execution.report.released;
            if was_claim && execution.task_id.is_none() {
                self.claim_attempted = false;
            }
            let end = execution.report.end;
            let last = execution
                .nodes
                .iter()
                .rev()
                .find(|node| node.completed)
                .map(|node| node.name.clone());
            self.executions.push(execution);
            ensure!(self.inputs_released, "邮件 Pipeline 尚未确认释放输入");
            // 原生失败 / 超时优先保留失败原因；停止始终优先，暂停只能用于可恢复任务。
            if self.control.requested() == Some(Interrupt::Stop) {
                self.interrupt();
                return Ok(());
            }
            let last_node = last.as_deref().unwrap_or("尚无完成节点");
            let claim_note = if self.claim_attempted {
                "未重放可能已生效的领取"
            } else {
                "未执行领取"
            };
            match end {
                PipelineEnd::TimedOut => {
                    self.terminal(
                        MailState::TimedOut,
                        &format!(
                            "邮件步骤超时（{}；最后完成节点：{last_node}）；{claim_note}，输入已释放",
                            step.entry()
                        ),
                    );
                    return Ok(());
                }
                PipelineEnd::Failed => {
                    self.terminal(
                        MailState::Failed,
                        &format!(
                            "邮件 Pipeline 执行失败（{}；最后完成节点：{last_node}），输入已释放",
                            step.entry()
                        ),
                    );
                    return Ok(());
                }
                PipelineEnd::Cancelled => {
                    ensure!(self.interrupt(), "收到未登记的取消信号");
                    return Ok(());
                }
                PipelineEnd::Succeeded => {}
            }
            let expected: &[&str] = match step {
                MailStep::Navigate => &["Mail.List"],
                MailStep::Inspect => &[
                    "Mail.Available",
                    "Mail.Empty",
                    "Mail.Dialog",
                    "Mail.Menu",
                    "Mail.World",
                ],
                MailStep::Claim => &["Mail.ClickClaim"],
                MailStep::AfterClaim => &["Mail.Dialog", "Mail.Empty"],
                MailStep::Confirm => &["Mail.ClickConfirm"],
                MailStep::AfterConfirm => &["Mail.Empty"],
                MailStep::ReturnList => &["Mail.Menu"],
                MailStep::ReturnWorld => &["Mail.World"],
            };
            if !last.as_deref().is_some_and(|name| expected.contains(&name)) {
                self.terminal(MailState::Failed, "Maa 任务未到达预期业务节点，输入已释放");
                return Ok(());
            }
            if self.interrupt() {
                return Ok(());
            }
            match step {
                MailStep::Navigate => self.phase = MailPhase::Inspect,
                MailStep::Inspect => match last.as_deref() {
                    Some("Mail.Available") if !self.claim_attempted => {
                        self.phase = MailPhase::Claim
                    }
                    Some("Mail.Available") => self.phase = MailPhase::AfterClaim,
                    Some("Mail.Empty") => self.phase = MailPhase::ReturnList,
                    Some("Mail.Dialog") if self.claim_attempted => {
                        self.reward_observed = true;
                        self.phase = MailPhase::Confirm;
                    }
                    Some("Mail.Menu") => {
                        self.phase = if self.returning {
                            MailPhase::ReturnWorld
                        } else {
                            MailPhase::Navigate
                        }
                    }
                    Some("Mail.World") => {
                        self.phase = if self.returning {
                            MailPhase::Done
                        } else {
                            MailPhase::Navigate
                        }
                    }
                    _ => {
                        self.terminal(MailState::Failed, "邮件检查结果不属于本次领取，已停止流程");
                        return Ok(());
                    }
                },
                MailStep::Claim => self.phase = MailPhase::AfterClaim,
                MailStep::AfterClaim => match last.as_deref() {
                    Some("Mail.Dialog") => {
                        self.reward_observed = true;
                        self.phase = MailPhase::Confirm;
                    }
                    Some("Mail.Empty") => self.phase = MailPhase::ReturnList,
                    _ => {
                        self.terminal(MailState::Failed, "未取得可信的领取后状态");
                        return Ok(());
                    }
                },
                MailStep::Confirm => self.phase = MailPhase::AfterConfirm,
                MailStep::AfterConfirm => self.phase = MailPhase::ReturnList,
                MailStep::ReturnList => self.phase = MailPhase::ReturnWorld,
                MailStep::ReturnWorld => self.phase = MailPhase::Done,
            }
            // 防止资源异常造成无界循环或无界保留任务详情。
            if self.executions.len() >= 64 {
                self.terminal(MailState::Failed, "邮件步骤超过上限，输入已释放");
                return Ok(());
            }
        }
    }
}
