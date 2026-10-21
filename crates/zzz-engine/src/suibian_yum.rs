//! 饮茶仙子流程共用外层控制与原生会话；保存提交意图后才发送 Maa 输入。
pub use crate::suibian_temple::HostingEnd as YumEnd;
use anyhow::{Context, Result, ensure};
use maa_adapter::{NativeSession, suibian_yum::YumInputEnd};
use od_core::{
    pipeline::{Interrupt, PipelineControl},
    suibian_yum::TempleYum,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct YumSnapshot {
    pub flow: TempleYum,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct YumRun {
    pub snapshot: YumSnapshot,
    control: PipelineControl,
    end: Option<YumEnd>,
    steps: u32,
}
impl YumRun {
    pub fn new(flow: TempleYum, control: PipelineControl) -> Result<Self> {
        flow.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: YumSnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
            },
            control,
            end: None,
            steps: 0,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(YumEnd::Paused) && self.snapshot.inputs_released,
            "饮茶仙尚未安全暂停"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<YumEnd> {
        match self.control.requested() {
            Some(Interrupt::Stop) => Some(YumEnd::Stopped),
            Some(Interrupt::Pause) => Some(YumEnd::Paused),
            None => None,
        }
    }
    pub fn execute(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        publish: impl FnMut(YumSnapshot) -> Result<()>,
    ) -> Result<YumEnd> {
        self.execute_with_budget(session, game, Duration::MAX, publish)
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(YumSnapshot) -> Result<()>,
    ) -> Result<YumEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(YumEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "饮茶仙子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial_ms = self.snapshot.active_ms;
        let budget = u64::from(self.snapshot.flow.config.timeout_ms);
        let remaining = || {
            let elapsed = started.elapsed();
            Duration::from_millis(
                budget.saturating_sub(initial_ms.saturating_add(elapsed.as_millis() as u64)),
            )
            .min(parent_remaining.saturating_sub(elapsed))
        };
        let result = (|| -> Result<YumEnd> {
            session.require_suibian_yum()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if self.snapshot.flow.completed() {
                    return Ok(YumEnd::Completed);
                }
                if remaining().is_zero() {
                    return Ok(YumEnd::TimedOut);
                }
                ensure!(self.steps < 4096, "饮茶仙步骤达到上限");
                self.steps += 1;
                match self.snapshot.flow.phase {
                    od_core::suibian_yum::YumPhase::Craft => {
                        let i = usize::from(
                            self.snapshot
                                .flow
                                .current_material
                                .context("没有当前材料")?,
                        );
                        let m = &self.snapshot.flow.materials[i];
                        let mut child = crate::suibian_craft::CraftRun::new(
                            *m.craft.clone().context("没有补料制造")?,
                            self.control.clone(),
                        )?;
                        child.snapshot.active_ms = m.craft_ms;
                        let end = child.execute_with_budget(session, game, remaining(), |snap| {
                            let mut candidate = self.snapshot.clone();
                            candidate
                                .flow
                                .update_craft(snap.flow, snap.active_ms)
                                .map_err(anyhow::Error::msg)?;
                            candidate.active_ms =
                                initial_ms.saturating_add(started.elapsed().as_millis() as u64);
                            candidate.inputs_released = snap.inputs_released;
                            publish(candidate.clone())?;
                            self.snapshot = candidate;
                            Ok(())
                        });
                        self.snapshot.inputs_released = child.snapshot.inputs_released;
                        let end = end?;
                        if end != YumEnd::Completed {
                            return Ok(end);
                        }
                        self.snapshot
                            .flow
                            .finish_child()
                            .map_err(anyhow::Error::msg)?;
                        publish(self.snapshot.clone())?;
                        continue;
                    }
                    od_core::suibian_yum::YumPhase::Adventure => {
                        let i = usize::from(
                            self.snapshot
                                .flow
                                .current_material
                                .context("没有当前材料")?,
                        );
                        let m = &self.snapshot.flow.materials[i];
                        let mut child = crate::suibian_adventure::AdventureRun::new(
                            *m.adventure.clone().context("没有补料游历")?,
                            self.control.clone(),
                        )?;
                        child.snapshot.active_ms = m.adventure_ms;
                        let end = child.execute_with_budget(session, game, remaining(), |snap| {
                            let mut candidate = self.snapshot.clone();
                            candidate
                                .flow
                                .update_adventure(snap.flow, snap.active_ms)
                                .map_err(anyhow::Error::msg)?;
                            candidate.active_ms =
                                initial_ms.saturating_add(started.elapsed().as_millis() as u64);
                            candidate.inputs_released = snap.inputs_released;
                            publish(candidate.clone())?;
                            self.snapshot = candidate;
                            Ok(())
                        });
                        self.snapshot.inputs_released = child.snapshot.inputs_released;
                        let end = end?;
                        if end != YumEnd::Completed {
                            return Ok(end);
                        }
                        self.snapshot
                            .flow
                            .finish_child()
                            .map_err(anyhow::Error::msg)?;
                        publish(self.snapshot.clone())?;
                        continue;
                    }
                    _ => {}
                }
                let read = session.read_suibian_yum(&self.control.cancellation(), remaining());
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "饮茶仙读帧后尚未释放输入");
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(YumEnd::TimedOut);
                }
                let Some(view) = read? else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let action = self.snapshot.flow.next(&view).map_err(anyhow::Error::msg)?;
                self.snapshot.active_ms =
                    initial_ms.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                let Some(action) = action else {
                    continue;
                };
                let before = self.snapshot.flow.clone();
                self.snapshot
                    .flow
                    .prepare(&action)
                    .map_err(anyhow::Error::msg)?;
                if self.snapshot.flow != before
                    && let Err(error) = publish(self.snapshot.clone())
                {
                    self.snapshot.flow = before;
                    return Err(error);
                }
                if remaining().is_zero() || self.interrupted().is_some() {
                    self.snapshot
                        .flow
                        .not_posted()
                        .map_err(anyhow::Error::msg)?;
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(YumEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_suibian_yum(
                    &self.snapshot.flow,
                    &action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(self.snapshot.inputs_released, "饮茶仙输入后尚未释放输入");
                match input {
                    Ok(YumInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted_navigation(&action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(YumInputEnd::NotPosted | YumInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted()
                        .map_err(anyhow::Error::msg)?,
                    Ok(YumInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted()
                            .map_err(anyhow::Error::msg)?;
                        publish(self.snapshot.clone())?;
                        return Ok(YumEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不能证明未投递；保留意图，不能用重试猜测是否生效。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(YumEnd::TimedOut);
                        }
                        return Err(error);
                    }
                }
                self.snapshot.active_ms =
                    initial_ms.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.snapshot.active_ms = initial_ms.saturating_add(started.elapsed().as_millis() as u64);
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        if result.is_err() || !self.snapshot.inputs_released {
            self.end = Some(YumEnd::Failed);
        }
        ensure!(self.snapshot.inputs_released, "饮茶仙收尾无法确认释放输入");
        let mut end = result?;
        if let Some(requested) = self.interrupted() {
            end = requested;
        }
        if end == YumEnd::Paused && !self.control.acknowledge_pause() {
            end = YumEnd::Stopped;
        }
        self.end = Some(end);
        publish(self.snapshot.clone())?;
        // 不终结共享控制句柄，售卖等后继仍需接受暂停和停止。
        Ok(end)
    }
}
