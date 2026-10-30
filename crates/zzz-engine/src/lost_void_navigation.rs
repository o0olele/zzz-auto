//! 迷失之地导航子流程，共用父控制、活动预算和输入收尾，不出战或记次数。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, lost_void_navigation::NavInputEnd as LostVoidNavigationInputEnd};
use od_core::{
    lost_void_navigation::{NavFlow, NavRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidNavigationEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidNavigationSnapshot {
    pub flow: NavFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
}
pub struct LostVoidNavigationRun {
    snapshot: LostVoidNavigationSnapshot,
    control: PipelineControl,
    end: Option<LostVoidNavigationEnd>,
    steps: u32,
}
impl LostVoidNavigationRun {
    pub(crate) fn own_timeout(&self) -> bool {
        self.end == Some(LostVoidNavigationEnd::TimedOut)
            && self.snapshot.inputs_released
            && self.control.requested().is_none()
            && self.snapshot.active_ms >= u64::from(self.snapshot.flow.request().timeout_ms)
            && self.snapshot.flow.can_abandon_after_timeout()
    }
    pub fn with_obstacle_attack(mut self, input: od_core::InputProbeAction) -> Result<Self> {
        self.snapshot.flow = self
            .snapshot
            .flow
            .with_obstacle_attack(input)
            .map_err(anyhow::Error::msg)?;
        Ok(self)
    }
    /// 仅把已观察到的事件页交给事件执行器，提交 F 本身不提供事件完成证明。
    pub fn event_request(
        &self,
        members: u8,
        options: od_core::lost_void::ArtifactSelection,
        timeout_ms: u32,
    ) -> Result<od_core::lost_void_event::LostVoidEventRequest> {
        let receipt = self
            .receipt()
            .ok_or_else(|| anyhow::anyhow!("导航尚未安全交回"))?;
        use od_core::lost_void_scene::LostVoidScreen as S;
        ensure!(
            matches!(
                receipt.screen,
                S::Event
                    | S::CommonChoice
                    | S::Gear
                    | S::GearConfirm
                    | S::Store
                    | S::Lottery
                    | S::RouteChange
            ),
            "尚未观察到可交接的事件页面"
        );
        let memory = self.snapshot.flow.memory();
        let (id, target) = memory
            .pending_event()
            .ok_or_else(|| anyhow::anyhow!("导航尚无匹配事件身份"))?;
        ensure!(id == receipt.interaction_id, "导航事件身份不一致");
        let request = od_core::lost_void_event::LostVoidEventRequest {
            run_id: receipt.run_id,
            interaction_id: id,
            challenge: self.snapshot.flow.request().challenge.clone(),
            options,
            entry: memory.region() == "入口",
            completed_first: memory.completed_first().to_vec(),
            members,
            allow_challenge_confirm: matches!(target, od_core::lost_void_npc::NpcTarget::Entry(n) if n.starts_with("挑战-")),
            allow_gear_notice: false,
            timeout_ms,
        };
        request.validate().map_err(anyhow::Error::msg)?;
        Ok(request)
    }
    pub fn snapshot(&self) -> LostVoidNavigationSnapshot {
        self.snapshot.clone()
    }
    /// 保存、释放及控制边界均正常之后才交回；不能作为挑战消费许可。
    pub fn receipt(&self) -> Option<&od_core::lost_void_navigation::NavReceipt> {
        if self.control.requested().is_some() {
            return None;
        }
        self.receipt_for_parent()
    }
    pub(crate) fn receipt_for_parent(&self) -> Option<&od_core::lost_void_navigation::NavReceipt> {
        if self.end != Some(LostVoidNavigationEnd::Ready) || !self.snapshot.inputs_released {
            return None;
        }
        self.snapshot.flow.receipt()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn new(request: NavRequest, control: PipelineControl) -> Result<Self> {
        let flow = NavFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(Self {
            snapshot: LostVoidNavigationSnapshot {
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
            self.end == Some(LostVoidNavigationEnd::Paused),
            "迷失之地导航尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    /// 父流程在子流程完成之后才暂停时，同样撤销旧到达证明。
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidNavigationEnd::Paused | LostVoidNavigationEnd::Ready)
            ) && self.snapshot.inputs_released,
            "迷失之地导航尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidNavigationEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidNavigationEnd::Paused,
            Interrupt::Stop => LostVoidNavigationEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        recognizer: &mut maa_adapter::combat_vision::CombatRecognizer,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidNavigationSnapshot) -> Result<()>,
    ) -> Result<LostVoidNavigationEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(LostVoidNavigationEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "迷失之地导航子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<LostVoidNavigationEnd> {
            session.check_ether_synthesis_target(game)?;
            session.require_lost_void_navigation(recognizer)?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidNavigationEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(LostVoidNavigationEnd::Ready);
                }
                ensure!(self.steps < 4096, "迷失之地导航步骤达到上限");
                self.steps += 1;
                let read = session.read_lost_void_navigation(
                    &self.snapshot.flow,
                    recognizer,
                    &self.control.cancellation(),
                    remaining(),
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "迷失之地导航读帧后尚未释放输入"
                );
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidNavigationEnd::TimedOut);
                }
                let Some(view) = read? else {
                    self.snapshot.flow.invalidate_observation();
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
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
                    .prepare(action.clone())
                    .map_err(anyhow::Error::msg)?;
                if let Err(error) = publish(self.snapshot.clone()) {
                    self.snapshot.flow = before;
                    return Err(error);
                }
                if remaining().is_zero() || self.interrupted().is_some() {
                    self.snapshot
                        .flow
                        .not_posted(&action)
                        .map_err(anyhow::Error::msg)?;
                    publish(self.snapshot.clone())?;
                    return Ok(self
                        .interrupted()
                        .unwrap_or(LostVoidNavigationEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_lost_void_navigation(
                    &self.snapshot.flow,
                    &action,
                    recognizer,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "迷失之地导航输入后尚未释放输入"
                );
                match input {
                    Ok(LostVoidNavigationInputEnd::MotionObserved { result }) => {
                        let od_core::lost_void_navigation::NavAction::Motion {
                            action: od_core::lost_void_motion::MotionAction::Probe { direction },
                        } = action
                        else {
                            anyhow::bail!("背景采样结果缺少对应动作");
                        };
                        self.snapshot
                            .flow
                            .motion_observed(direction, result)
                            .map_err(anyhow::Error::msg)?;
                    }
                    Ok(LostVoidNavigationInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(&action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(LostVoidNavigationInputEnd::InterruptedMovement) => self
                        .snapshot
                        .flow
                        .interrupted_movement(&action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(
                        LostVoidNavigationInputEnd::NotPosted | LostVoidNavigationInputEnd::Changed,
                    ) => self
                        .snapshot
                        .flow
                        .not_posted(&action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(LostVoidNavigationInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(&action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(LostVoidNavigationEnd::TimedOut);
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(LostVoidNavigationEnd::TimedOut);
                        }
                        return Err(error);
                    }
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
                let settle = Instant::now();
                let settle_ms = if matches!(
                    action,
                    od_core::lost_void_navigation::NavAction::Motion {
                        action: od_core::lost_void_motion::MotionAction::ClearObstacle
                    }
                ) {
                    1000
                } else {
                    200
                };
                while settle.elapsed() < Duration::from_millis(settle_ms)
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
            self.end = Some(LostVoidNavigationEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "迷失之地导航收尾无法确认释放输入"
        );
        let mut end = result?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(LostVoidNavigationEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = LostVoidNavigationEnd::TimedOut;
        }
        if end == LostVoidNavigationEnd::Paused && !self.control.acknowledge_pause() {
            end = LostVoidNavigationEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
