use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, PipelineEnd};
use od_core::{
    basic_daily::{DailyConfig, DailyProgress},
    mail::MailState,
    pipeline::{Interrupt, PipelineControl, PipelineRequest},
};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub struct DailyRun {
    pub id: Uuid,
    pub progress: DailyProgress,
    pub message: String,
    control: PipelineControl,
    steps: u32,
}
impl DailyRun {
    pub fn new(id: Uuid, app: &str, config: DailyConfig) -> Result<Self> {
        Ok(Self {
            id,
            progress: DailyProgress::new(app, config).map_err(anyhow::Error::msg)?,
            message: "日常流程已创建".into(),
            control: PipelineControl::default(),
            steps: 0,
        })
    }
    pub fn control(&self) -> PipelineRequest {
        self.control.requester()
    }
    pub fn interrupt_requested(&self) -> Option<Interrupt> {
        self.control.requested()
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.progress.state == MailState::Paused && self.progress.inputs_released,
            "日常尚未安全暂停"
        );
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.progress.resume();
        Ok(())
    }
    pub fn acknowledge_idle(&mut self) {
        self.progress.inputs_released = true;
        self.interrupt();
    }
    fn interrupt(&mut self) -> bool {
        match self.control.requested() {
            Some(Interrupt::Stop) => {
                self.control.finish();
                self.progress.state = MailState::Stopped;
                self.message = "日常已停止，输入已释放".into();
                true
            }
            Some(Interrupt::Pause) => {
                if !self.control.acknowledge_pause() {
                    return self.interrupt();
                }
                self.progress.state = MailState::Paused;
                self.message = "日常已暂停，输入已释放并保留检查点".into();
                true
            }
            None => false,
        }
    }
    fn terminal(&mut self, state: MailState, message: &str) {
        if self.control.finish() == Some(Interrupt::Stop) {
            self.progress.state = MailState::Stopped;
            self.message = "日常已停止，输入已释放".into();
        } else {
            self.progress.state = state;
            self.message = message.into();
        }
    }
    pub fn execute(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        mut publish: impl FnMut(DailyProgress) -> Result<()>,
    ) -> Result<()> {
        ensure!(
            self.progress.state == MailState::Running
                || (self.progress.state == MailState::Paused
                    && self.control.requested() == Some(Interrupt::Stop)),
            "日常任务不能直接重跑"
        );
        if game {
            session.prepare_game_mail(
                self.control.cancellation(),
                self.progress.config.start_delay_ms,
            )?;
        } else {
            session.prepare_fixture_mail()?;
        }
        let started = Instant::now();
        let result = (|| -> Result<()> {
            loop {
                if self.interrupt() {
                    return Ok(());
                }
                if self.progress.stage == "Done" {
                    if self.progress.completed() {
                        self.terminal(MailState::Succeeded, "日常完成，已返回大世界并释放输入");
                    } else {
                        self.terminal(MailState::Failed, "日常未取得完整完成证明，已返回大世界");
                    }
                    return Ok(());
                }
                let used = self
                    .progress
                    .active_ms
                    .saturating_add(started.elapsed().as_millis() as u64);
                if used >= u64::from(self.progress.config.timeout_ms) {
                    self.terminal(MailState::TimedOut, "日常流程超时，输入已释放");
                    return Ok(());
                }
                ensure!(self.steps < 128, "日常步骤超过上限");
                let before = self.progress.clone();
                self.progress.before_step();
                let mut current = self.progress.clone();
                current.active_ms = used;
                // 保存失败发生在本步骤投递前；保留上一个安全检查点。
                if let Err(error) = publish(current) {
                    self.progress = before;
                    return Err(error);
                }
                let execution = session.basic_daily_step(
                    &self.progress,
                    self.control.cancellation(),
                    Duration::from_millis(u64::from(self.progress.config.timeout_ms) - used),
                    game,
                )?;
                self.progress.inputs_released = execution.report.released;
                ensure!(
                    self.progress.inputs_released,
                    "日常 Pipeline 尚未确认释放输入"
                );
                if execution.task_id.is_none() {
                    self.progress = before;
                }
                self.steps += 1;
                if self.control.requested() == Some(Interrupt::Stop) {
                    self.interrupt();
                    return Ok(());
                }
                match execution.report.end {
                    PipelineEnd::Succeeded => {
                        let node = execution
                            .nodes
                            .iter()
                            .rev()
                            .find(|n| n.completed)
                            .ok_or_else(|| anyhow::anyhow!("日常缺少完成节点"))?;
                        self.progress
                            .observe(&node.name)
                            .map_err(anyhow::Error::msg)?;
                    }
                    PipelineEnd::Cancelled => {
                        ensure!(self.interrupt(), "日常收到未登记取消");
                        return Ok(());
                    }
                    PipelineEnd::TimedOut => {
                        self.terminal(MailState::TimedOut, "日常步骤超时，输入已释放");
                        return Ok(());
                    }
                    PipelineEnd::Failed => {
                        self.terminal(MailState::Failed, "日常 Pipeline 失败，输入已释放");
                        return Ok(());
                    }
                }
                if self.interrupt() {
                    return Ok(());
                }
            }
        })();
        self.progress.active_ms = self
            .progress
            .active_ms
            .saturating_add(started.elapsed().as_millis() as u64);
        if let Err(error) = &result {
            self.control.finish();
            self.progress.state = MailState::Failed;
            self.message = format!("日常执行失败：{error:#}");
        }
        result
    }
}
