//! NPC 返回移动子流程，共用父控制、活动预算和输入收尾，不出战或记次数。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, lost_void_npc::NpcInputEnd as LostVoidNpcInputEnd};
use od_core::{
    lost_void_npc::{NpcMoveFlow, NpcMoveRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidNpcEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
    ObservedPage(od_core::lost_void_scene::LostVoidScreen),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidNpcSnapshot {
    pub flow: NpcMoveFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
    #[serde(default)]
    pub observed_page: Option<od_core::lost_void_scene::LostVoidScreen>,
}
pub struct LostVoidNpcRun {
    snapshot: LostVoidNpcSnapshot,
    control: PipelineControl,
    end: Option<LostVoidNpcEnd>,
    steps: u32,
    page_handoff: bool,
    page_reads: Option<(od_core::lost_void_scene::LostVoidScreen, u8)>,
}
impl LostVoidNpcRun {
    /// 事件已经保存、释放且控制句柄一致时，才生成本层返回移动。
    pub fn after_event(
        memory: &mut od_core::lost_void_npc::NpcMemory,
        event: &crate::lost_void_event::LostVoidEventRun,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(
            event.uses_control(&control),
            "NPC 返回移动不能更换事件父控制"
        );
        ensure!(
            control.requested().is_none(),
            "尚有暂停或停止请求，不能交接 NPC 移动"
        );
        let receipt = event
            .receipt()
            .ok_or_else(|| anyhow::anyhow!("事件尚未安全交回返回页面"))?;
        let mut next = memory.clone();
        let request = next
            .finish_event(receipt, timeout_ms)
            .map_err(anyhow::Error::msg)?;
        let run = Self::new(request, control)?;
        *memory = next;
        Ok(run)
    }
    pub fn snapshot(&self) -> LostVoidNpcSnapshot {
        self.snapshot.clone()
    }
    /// 保存、释放及控制边界均正常之后才交回；不能作为挑战消费许可。
    pub fn receipt(&self) -> Option<&od_core::lost_void_npc::NpcMoveReceipt> {
        if self.control.requested().is_some() {
            return None;
        }
        self.receipt_for_parent()
    }
    pub(crate) fn receipt_for_parent(&self) -> Option<&od_core::lost_void_npc::NpcMoveReceipt> {
        if self.end != Some(LostVoidNpcEnd::Ready) || !self.snapshot.inputs_released {
            return None;
        }
        self.snapshot.flow.receipt()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn new(request: NpcMoveRequest, control: PipelineControl) -> Result<Self> {
        let flow = NpcMoveFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: LostVoidNpcSnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
                observed_page: None,
            },
            control,
            end: None,
            steps: 0,
            page_handoff: false,
            page_reads: None,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidNpcEnd::Paused),
            "NPC 返回移动尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    pub fn with_page_handoff(mut self) -> Result<Self> {
        ensure!(
            self.steps == 0 && self.end.is_none(),
            "移动运行后不能更改页面交接策略"
        );
        self.page_handoff = true;
        Ok(self)
    }
    pub(crate) fn continue_after_page(&mut self) -> Result<()> {
        ensure!(
            matches!(self.end, Some(LostVoidNpcEnd::ObservedPage(_)))
                && self.snapshot.inputs_released
                && self.control.requested().is_none(),
            "移动尚未安全交回内容页"
        );
        self.snapshot.flow.resume().map_err(anyhow::Error::msg)?;
        self.snapshot.observed_page = None;
        self.page_reads = None;
        self.end = None;
        Ok(())
    }
    /// 父流程在子流程完成之后才暂停时，同样撤销旧到达证明。
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(
                    LostVoidNpcEnd::Paused
                        | LostVoidNpcEnd::Ready
                        | LostVoidNpcEnd::ObservedPage(_)
                )
            ) && self.snapshot.inputs_released,
            "NPC 返回移动尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.snapshot.observed_page = None;
        self.page_reads = None;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidNpcEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidNpcEnd::Paused,
            Interrupt::Stop => LostVoidNpcEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidNpcSnapshot) -> Result<()>,
    ) -> Result<LostVoidNpcEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(LostVoidNpcEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "NPC 返回移动子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<LostVoidNpcEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_lost_void_scene()?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidNpcEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(LostVoidNpcEnd::Ready);
                }
                ensure!(self.steps < 4096, "NPC 返回移动步骤达到上限");
                self.steps += 1;
                let read = session.read_lost_void_npc(
                    &self.snapshot.flow,
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "NPC 返回移动读帧后尚未释放输入"
                );
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidNpcEnd::TimedOut);
                }
                let Some(view) = read? else {
                    self.page_reads = None;
                    self.snapshot.flow.invalidate_observation();
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                if self.page_handoff && od_core::lost_void_scene::level_content(view.screen) {
                    self.snapshot.flow.invalidate_observation();
                    let reads = if self.page_reads.is_some_and(|(s, _)| s == view.screen) {
                        (self.page_reads.unwrap().1 + 1).min(2)
                    } else {
                        1
                    };
                    self.page_reads = Some((view.screen, reads));
                    if reads == 2 {
                        self.snapshot.observed_page = Some(view.screen);
                        return Ok(LostVoidNpcEnd::ObservedPage(view.screen));
                    }
                    continue;
                }
                self.page_reads = None;
                let action = self.snapshot.flow.next(&view).map_err(anyhow::Error::msg)?;
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                let Some(action) = action else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let before = self.snapshot.flow.clone();
                self.snapshot
                    .flow
                    .prepare(action)
                    .map_err(anyhow::Error::msg)?;
                if let Err(error) = publish(self.snapshot.clone()) {
                    self.snapshot.flow = before;
                    return Err(error);
                }
                if remaining().is_zero() || self.interrupted().is_some() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    publish(self.snapshot.clone())?;
                    return Ok(self.interrupted().unwrap_or(LostVoidNpcEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_lost_void_npc(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "NPC 返回移动输入后尚未释放输入"
                );
                match input {
                    Ok(LostVoidNpcInputEnd::Held(ms)) => self
                        .snapshot
                        .flow
                        .held(action, ms)
                        .map_err(anyhow::Error::msg)?,
                    Ok(LostVoidNpcInputEnd::NotPosted | LostVoidNpcInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(LostVoidNpcInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(LostVoidNpcEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(LostVoidNpcEnd::TimedOut);
                        }
                        return Err(error);
                    }
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                let settle = Instant::now();
                while settle.elapsed() < Duration::from_millis(200)
                    && !remaining().is_zero()
                    && self.interrupted().is_none()
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        })();
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() || !self.snapshot.inputs_released {
            self.end = Some(LostVoidNpcEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "NPC 返回移动收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(LostVoidNpcEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = LostVoidNpcEnd::TimedOut;
        }
        if end == LostVoidNpcEnd::Paused && !self.control.acknowledge_pause() {
            end = LostVoidNpcEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
