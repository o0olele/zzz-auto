//! 悬赏面板检查及返回；保存、资源、释放输入与活动预算共同守卫交接。
use anyhow::{Result, ensure};
use maa_adapter::{NativeSession, lost_void_bounty::BountyInputEnd as LostVoidBountyInputEnd};
use od_core::{
    lost_void_bounty::{BountyFlow, BountyRequest},
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostVoidBountyEnd {
    Ready,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidBountySnapshot {
    pub flow: BountyFlow,
    pub active_ms: u64,
    pub inputs_released: bool,
    pub record_context: Option<LostVoidClaimContext>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub bounty_read_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub bounty_read_finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub bounty_observation: Option<od_core::lost_void_scene::BountyProgress>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidClaimContext {
    pub instance_id: u32,
    pub region: od_core::daily::GameRegion,
    pub source_ids: Vec<uuid::Uuid>,
}
pub struct LostVoidBountyRun {
    snapshot: LostVoidBountySnapshot,
    control: PipelineControl,
    end: Option<LostVoidBountyEnd>,
    steps: u32,
    scene: PathBuf,
    scene_manifest_sha256: Vec<u8>,
    pending_bounty: Option<(
        od_core::lost_void_scene::BountyProgress,
        chrono::DateTime<chrono::Utc>,
    )>,
}
impl LostVoidBountyRun {
    pub fn snapshot(&self) -> LostVoidBountySnapshot {
        self.snapshot.clone()
    }
    /// 保存、释放及控制边界均正常之后才交回领取检查；不提供挑战通关证明。
    pub fn receipt(&self) -> Option<&od_core::lost_void_bounty::BountyReceipt> {
        if self.control.requested().is_some() {
            return None;
        }
        self.receipt_for_parent()
    }
    pub(crate) fn receipt_for_parent(&self) -> Option<&od_core::lost_void_bounty::BountyReceipt> {
        if self.end != Some(LostVoidBountyEnd::Ready) || !self.snapshot.inputs_released {
            return None;
        }
        self.snapshot.flow.receipt()
    }
    pub fn uses_control(&self, control: &PipelineControl) -> bool {
        std::sync::Arc::ptr_eq(&self.control.cancellation(), &control.cancellation())
    }
    pub fn new(request: BountyRequest, scene: &Path, control: PipelineControl) -> Result<Self> {
        let flow = BountyFlow::new(request).map_err(anyhow::Error::msg)?;
        let scene = maa_adapter::lost_void_scene::validate_lost_void_scene_bundle(scene)?;
        let scene_manifest_sha256 =
            Sha256::digest(std::fs::read(scene.join("lost_void_scene-manifest.json"))?).to_vec();
        Ok(Self {
            snapshot: LostVoidBountySnapshot {
                flow,
                active_ms: 0,
                inputs_released: true,
                record_context: None,
                finished_at: None,
                bounty_read_started_at: None,
                bounty_read_finished_at: None,
                bounty_observation: None,
            },
            control,
            end: None,
            steps: 0,
            scene,
            scene_manifest_sha256,
            pending_bounty: None,
        })
    }
    pub fn with_record_context(
        mut self,
        instance_id: u32,
        region: od_core::daily::GameRegion,
        mut source_ids: Vec<uuid::Uuid>,
    ) -> Result<Self> {
        ensure!(
            self.end.is_none()
                && self.steps == 0
                && self.snapshot.active_ms == 0
                && self.snapshot.record_context.is_none(),
            "领取记账上下文必须在首次观察及输入前冻结"
        );
        source_ids.sort_unstable();
        ensure!(
            (1..=9999).contains(&instance_id)
                && source_ids.len() <= od_core::lost_void_claim_record::MAX_CLAIM_SOURCES
                && !source_ids.iter().any(uuid::Uuid::is_nil)
                && source_ids.windows(2).all(|w| w[0] < w[1])
                && (source_ids.is_empty()
                    || source_ids.contains(&self.snapshot.flow.request().source_id)),
            "领取账号或绑定结算无效"
        );
        self.snapshot.record_context = Some(LostVoidClaimContext {
            instance_id,
            region,
            source_ids,
        });
        Ok(self)
    }
    pub fn claim_record(&self) -> Result<od_core::lost_void_claim_record::LostVoidClaimRecord> {
        use od_core::{
            daily::GameDay,
            lost_void_claim_record::{LostVoidClaimRecord, same_claim_week},
        };
        let receipt = self
            .receipt()
            .ok_or_else(|| anyhow::anyhow!("领取检查尚未安全交回"))?;
        self.verify_scene()?;
        let context = self
            .snapshot
            .record_context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("领取输入前未冻结记账上下文"))?;
        let finished = self
            .snapshot
            .finished_at
            .ok_or_else(|| anyhow::anyhow!("领取缺少实际最终返回时刻"))?;
        let day = GameDay::at(context.region, finished).map_err(anyhow::Error::msg)?;
        let observed_day = match (
            self.snapshot.bounty_read_started_at,
            self.snapshot.bounty_read_finished_at,
        ) {
            (Some(start), Some(end))
                if start <= end
                    && self.snapshot.bounty_observation == receipt.bounty
                    && receipt.bounty.is_some() =>
            {
                let a = GameDay::at(context.region, start).map_err(anyhow::Error::msg)?;
                let b = GameDay::at(context.region, end).map_err(anyhow::Error::msg)?;
                if same_claim_week(&a, &b).map_err(anyhow::Error::msg)?
                    && same_claim_week(&b, &day).map_err(anyhow::Error::msg)?
                {
                    Some(b)
                } else {
                    None
                }
            }
            _ => None,
        };
        let record = LostVoidClaimRecord::from_flow(
            context.instance_id,
            day,
            &self.snapshot.flow,
            context.source_ids.clone(),
            observed_day,
        )
        .map_err(anyhow::Error::msg)?;
        ensure!(self.receipt().is_some(), "领取记账提取期间控制状态发生变化");
        Ok(record)
    }
    fn verify_scene(&self) -> Result<()> {
        ensure!(
            maa_adapter::lost_void_scene::validate_lost_void_scene_bundle(&self.scene)?
                == self.scene,
            "悬赏场景根目录发生变化"
        );
        ensure!(
            Sha256::digest(std::fs::read(
                self.scene.join("lost_void_scene-manifest.json")
            )?)
            .as_slice()
                == self.scene_manifest_sha256,
            "悬赏场景清单发生变化"
        );
        Ok(())
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(LostVoidBountyEnd::Paused),
            "迷失之地悬赏准备尚未暂停"
        );
        self.resume_after_parent_pause()
    }
    /// 父流程在子流程完成之后才暂停时，同样撤销旧到达证明。
    pub(crate) fn resume_after_parent_pause(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.end,
                Some(LostVoidBountyEnd::Paused | LostVoidBountyEnd::Ready)
            ) && self.snapshot.inputs_released,
            "迷失之地悬赏准备尚未安全暂停"
        );
        let mut flow = self.snapshot.flow.clone();
        flow.resume().map_err(anyhow::Error::msg)?;
        ensure!(self.control.resume(), "停止请求不能被继续覆盖");
        self.snapshot.flow = flow;
        self.pending_bounty = None;
        self.end = None;
        Ok(())
    }
    fn interrupted(&self) -> Option<LostVoidBountyEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => LostVoidBountyEnd::Paused,
            Interrupt::Stop => LostVoidBountyEnd::Stopped,
        })
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(LostVoidBountySnapshot) -> Result<()>,
    ) -> Result<LostVoidBountyEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(LostVoidBountyEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "迷失之地悬赏子流程不能直接重跑"
        );
        self.snapshot.flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.flow.request().timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<LostVoidBountyEnd> {
            if let Some(end) = self.interrupted() {
                return Ok(end);
            }
            if remaining().is_zero() {
                return Ok(LostVoidBountyEnd::TimedOut);
            }
            self.verify_scene()?;
            session.check_ether_synthesis_target(game)?;
            session.load_lost_void_scene_bundle(&self.scene)?;
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidBountyEnd::TimedOut);
                }
                if self.snapshot.flow.completed() {
                    return Ok(LostVoidBountyEnd::Ready);
                }
                ensure!(self.steps < 4096, "迷失之地悬赏准备步骤达到上限");
                self.steps += 1;
                let read_started = chrono::Utc::now();
                let read = session.read_lost_void_bounty(
                    &self.snapshot.flow,
                    &self.control.cancellation(),
                    remaining(),
                );
                let read_finished = chrono::Utc::now();
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "迷失之地悬赏读帧后尚未释放输入"
                );
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(LostVoidBountyEnd::TimedOut);
                }
                let Some(view) = read? else {
                    self.snapshot.flow.invalidate_observation();
                    self.pending_bounty = None;
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                if let Some(progress) = view.bounty {
                    if self.pending_bounty.is_none_or(|(old, _)| old != progress) {
                        self.pending_bounty = Some((progress, read_started));
                    }
                } else {
                    self.pending_bounty = None;
                }
                let action = self
                    .snapshot
                    .flow
                    .next(&view, self.snapshot.active_ms)
                    .map_err(anyhow::Error::msg)?;
                if let Some(progress) = view.bounty
                    && self.snapshot.flow.bounty_progress() == Some(progress)
                    && self.snapshot.bounty_observation != Some(progress)
                {
                    self.snapshot.bounty_observation = Some(progress);
                    self.snapshot.bounty_read_started_at = self.pending_bounty.map(|(_, t)| t);
                    self.snapshot.bounty_read_finished_at = Some(read_finished);
                }
                if self.snapshot.flow.completed() && self.snapshot.finished_at.is_none() {
                    self.snapshot.finished_at = Some(chrono::Utc::now());
                }
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
                    return Ok(self.interrupted().unwrap_or(LostVoidBountyEnd::TimedOut));
                }
                self.verify_scene()?;
                if remaining().is_zero() || self.interrupted().is_some() {
                    self.snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?;
                    return Ok(self.interrupted().unwrap_or(LostVoidBountyEnd::TimedOut));
                }
                self.snapshot.inputs_released = false;
                let input = session.act_lost_void_bounty(
                    &self.snapshot.flow,
                    action,
                    &self.control.cancellation(),
                    remaining(),
                    game,
                );
                self.snapshot.inputs_released = session.release_inputs().is_ok();
                ensure!(
                    self.snapshot.inputs_released,
                    "迷失之地悬赏输入后尚未释放输入"
                );
                match input {
                    Ok(LostVoidBountyInputEnd::Posted) => self
                        .snapshot
                        .flow
                        .posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(LostVoidBountyInputEnd::NotPosted | LostVoidBountyInputEnd::Changed) => self
                        .snapshot
                        .flow
                        .not_posted(action)
                        .map_err(anyhow::Error::msg)?,
                    Ok(LostVoidBountyInputEnd::TimedOut) => {
                        self.snapshot
                            .flow
                            .not_posted(action)
                            .map_err(anyhow::Error::msg)?;
                        return Ok(LostVoidBountyEnd::TimedOut);
                    }
                    Ok(LostVoidBountyInputEnd::Unknown) => {
                        self.snapshot
                            .flow
                            .unknown(action)
                            .map_err(anyhow::Error::msg)?;
                        publish(self.snapshot.clone())?;
                        if self.control.requested().is_none() {
                            self.control.pause();
                        }
                        return Ok(self.interrupted().unwrap_or(LostVoidBountyEnd::Paused));
                    }
                    Err(error) => {
                        // 原生错误不证明未发送。保留意图，安全暂停后从新画面重新核对。
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        if remaining().is_zero() {
                            return Ok(LostVoidBountyEnd::TimedOut);
                        }
                        return Err(error);
                    }
                }
                self.snapshot.active_ms =
                    initial.saturating_add(started.elapsed().as_millis() as u64);
                publish(self.snapshot.clone())?;
            }
        })();
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if result.is_err() || !self.snapshot.inputs_released {
            self.end = Some(LostVoidBountyEnd::Failed);
        }
        ensure!(
            self.snapshot.inputs_released,
            "迷失之地悬赏收尾无法确认释放输入"
        );
        let mut end = result?;
        self.verify_scene()
            .inspect_err(|_| self.end = Some(LostVoidBountyEnd::Failed))?;
        if let Err(error) = publish(self.snapshot.clone()) {
            self.end = Some(LostVoidBountyEnd::Failed);
            return Err(error);
        }
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        self.verify_scene()
            .inspect_err(|_| self.end = Some(LostVoidBountyEnd::Failed))?;
        // 保存终态期间仍可能收到暂停或停止；回交父流程前再次按共享控制句柄判定。
        if let Some(requested) = self.interrupted() {
            end = requested;
        } else if remaining().is_zero() {
            end = LostVoidBountyEnd::TimedOut;
        }
        if end == LostVoidBountyEnd::Paused && !self.control.acknowledge_pause() {
            end = LostVoidBountyEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
