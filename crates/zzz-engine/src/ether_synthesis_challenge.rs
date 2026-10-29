//! 同一父任务内完成快捷手册导航与单次电池兑换。
use crate::{
    ether_synthesis::{SynthesisEnd, SynthesisRun, SynthesisSnapshot},
    ether_synthesis_entry::{SynthesisEntryEnd, SynthesisEntryRun, SynthesisEntrySnapshot},
};
use anyhow::{Result, ensure};
use maa_adapter::NativeSession;
use od_core::{
    ether_synthesis::SynthesisRequest,
    pipeline::{Interrupt, PipelineControl},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisStage {
    Entry,
    Exchange,
    Completed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthesisChallengeEnd {
    Completed,
    Paused,
    Stopped,
    TimedOut,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesisChallengeSnapshot {
    pub id: Uuid,
    pub request: SynthesisRequest,
    pub stage: SynthesisStage,
    pub entry: SynthesisEntrySnapshot,
    pub exchange: Option<SynthesisSnapshot>,
    pub active_ms: u64,
    pub inputs_released: bool,
}
struct Resources {
    compendium: PathBuf,
    entry: PathBuf,
    exchange: PathBuf,
    hashes: Vec<(PathBuf, Vec<u8>)>,
}
impl Resources {
    fn load(root: &Path) -> Result<Self> {
        od_core::config::reject_links(root)?;
        let compendium = maa_adapter::compendium::validate_compendium_bundle(
            &root.join("generated/compendium"),
        )?;
        let entry = maa_adapter::ether_synthesis_entry::validate_ether_synthesis_entry_bundle(
            &root.join("generated/ether-synthesis-entry"),
        )?;
        let exchange = maa_adapter::ether_synthesis::validate_ether_synthesis_bundle(
            &root.join("generated/ether-synthesis"),
        )?;
        let hashes = [
            compendium.join("compendium-manifest.json"),
            entry.join("ether_synthesis_entry-manifest.json"),
            exchange.join("ether_synthesis-manifest.json"),
        ]
        .into_iter()
        .map(|path| Ok((path.clone(), Sha256::digest(std::fs::read(path)?).to_vec())))
        .collect::<Result<_>>()?;
        Ok(Self {
            compendium,
            entry,
            exchange,
            hashes,
        })
    }
    fn verify(&self) -> Result<()> {
        for (path, hash) in &self.hashes {
            od_core::config::reject_links(path)?;
            ensure!(
                Sha256::digest(std::fs::read(path)?).as_slice() == hash,
                "合成父任务冻结资源已变化"
            );
        }
        maa_adapter::compendium::validate_compendium_bundle(&self.compendium)?;
        maa_adapter::ether_synthesis_entry::validate_ether_synthesis_entry_bundle(&self.entry)?;
        maa_adapter::ether_synthesis::validate_ether_synthesis_bundle(&self.exchange)?;
        Ok(())
    }
}
pub struct SynthesisChallengeRun {
    snapshot: SynthesisChallengeSnapshot,
    resources: Resources,
    control: PipelineControl,
    entry: SynthesisEntryRun,
    exchange: Option<SynthesisRun>,
    entry_started: bool,
    exchange_started: bool,
    loaded: bool,
    end: Option<SynthesisChallengeEnd>,
}
impl SynthesisChallengeRun {
    pub fn new(
        id: Uuid,
        request: SynthesisRequest,
        resources: &Path,
        control: PipelineControl,
    ) -> Result<Self> {
        ensure!(!id.is_nil(), "合成父任务缺少运行身份");
        request.validate().map_err(anyhow::Error::msg)?;
        let resources = Resources::load(resources)?;
        let entry = SynthesisEntryRun::new(request.clone(), control.clone())?;
        Ok(Self {
            snapshot: SynthesisChallengeSnapshot {
                id,
                request,
                stage: SynthesisStage::Entry,
                entry: entry.snapshot(),
                exchange: None,
                active_ms: 0,
                inputs_released: true,
            },
            resources,
            control,
            entry,
            exchange: None,
            entry_started: false,
            exchange_started: false,
            loaded: false,
            end: None,
        })
    }
    pub fn snapshot(&self) -> SynthesisChallengeSnapshot {
        self.snapshot.clone()
    }
    pub fn verified_exchange(&self) -> Result<&SynthesisRun> {
        ensure!(
            self.end == Some(SynthesisChallengeEnd::Completed)
                && self.snapshot.stage == SynthesisStage::Completed
                && self.snapshot.inputs_released
                && self.control.requested().is_none(),
            "合成父任务尚无安全保存的本次终态"
        );
        let exchange = self
            .exchange
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("合成父任务没有兑换子流程"))?;
        exchange.proof()?;
        Ok(exchange)
    }
    /// 只供体力父任务登记已保存的正常结果；暂停或停止不能丢弃已经取得的消费回执。
    /// 此方法不交接输入，仍要求父、子均正常结束且输入已释放。
    pub(crate) fn exchange_for_registration(&self) -> Result<&SynthesisRun> {
        ensure!(
            self.end == Some(SynthesisChallengeEnd::Completed)
                && self.snapshot.stage == SynthesisStage::Completed
                && self.snapshot.inputs_released,
            "合成父任务没有已保存的正常登记结果"
        );
        let exchange = self
            .exchange
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("合成子流程缺失"))?;
        exchange.result_for_parent()?;
        Ok(exchange)
    }
    fn interrupted(&self) -> Option<SynthesisChallengeEnd> {
        self.control.requested().map(|i| match i {
            Interrupt::Pause => SynthesisChallengeEnd::Paused,
            Interrupt::Stop => SynthesisChallengeEnd::Stopped,
        })
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.end == Some(SynthesisChallengeEnd::Paused)
                && self.snapshot.inputs_released
                && self.control.requested() == Some(Interrupt::Pause),
            "合成父任务尚未安全暂停"
        );
        self.resources.verify()?;
        match self.snapshot.stage {
            SynthesisStage::Entry if self.entry_started => {
                self.entry.resume_after_parent_pause()?
            }
            SynthesisStage::Exchange | SynthesisStage::Completed if self.exchange_started => {
                self.exchange
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("合成子流程丢失"))?
                    .resume_after_parent_pause()?;
                self.snapshot.stage = SynthesisStage::Exchange;
            }
            _ => {
                ensure!(self.control.resume(), "停止不能被继续覆盖");
            }
        }
        self.end = None;
        Ok(())
    }
    pub fn execute_with_budget(
        &mut self,
        session: &mut NativeSession,
        game: bool,
        parent_remaining: Duration,
        mut publish: impl FnMut(SynthesisChallengeSnapshot) -> Result<()>,
    ) -> Result<SynthesisChallengeEnd> {
        ensure!(
            self.end.is_none()
                || (self.end == Some(SynthesisChallengeEnd::Paused)
                    && self.control.requested() == Some(Interrupt::Stop)),
            "合成父任务不能直接重跑"
        );
        let started = Instant::now();
        let initial = self.snapshot.active_ms;
        let budget = Duration::from_millis(
            u64::from(self.snapshot.request.timeout_ms).saturating_sub(initial),
        )
        .min(parent_remaining);
        let remaining = || budget.saturating_sub(started.elapsed());
        let result = (|| -> Result<SynthesisChallengeEnd> {
            session.check_ether_synthesis_target(game)?;
            session.release_inputs()?;
            self.resources.verify()?;
            if let Some(end) = self.interrupted() {
                return Ok(end);
            }
            if remaining().is_zero() {
                return Ok(SynthesisChallengeEnd::TimedOut);
            }
            if !self.loaded {
                session.load_compendium_bundle(&self.resources.compendium)?;
                session.load_ether_synthesis_entry_bundle(&self.resources.entry)?;
                session.load_ether_synthesis_bundle(&self.resources.exchange)?;
                self.loaded = true;
            }
            loop {
                if let Some(end) = self.interrupted() {
                    return Ok(end);
                }
                if remaining().is_zero() {
                    return Ok(SynthesisChallengeEnd::TimedOut);
                }
                self.resources.verify()?;
                match self.snapshot.stage {
                    SynthesisStage::Entry => {
                        self.entry_started = true;
                        let end = self.entry.execute_with_budget(
                            session,
                            game,
                            remaining(),
                            |entry| {
                                self.snapshot.entry = entry;
                                self.snapshot.inputs_released = self.snapshot.entry.inputs_released;
                                self.snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                publish(self.snapshot.clone())
                            },
                        )?;
                        self.snapshot.entry = self.entry.snapshot();
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        match end {
                            SynthesisEntryEnd::Ready => {
                                ensure!(
                                    self.snapshot.entry.inputs_released
                                        && self.snapshot.entry.flow.ready().is_some(),
                                    "合成导航缺少已保存返回证明"
                                );
                                self.exchange = Some(SynthesisRun::new(
                                    self.snapshot.id,
                                    self.snapshot.request.clone(),
                                    self.control.clone(),
                                )?);
                                self.snapshot.exchange =
                                    self.exchange.as_ref().map(SynthesisRun::snapshot);
                                self.snapshot.stage = SynthesisStage::Exchange;
                                self.snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                publish(self.snapshot.clone())?;
                            }
                            SynthesisEntryEnd::Paused => return Ok(SynthesisChallengeEnd::Paused),
                            SynthesisEntryEnd::Stopped => {
                                return Ok(SynthesisChallengeEnd::Stopped);
                            }
                            SynthesisEntryEnd::TimedOut => {
                                return Ok(SynthesisChallengeEnd::TimedOut);
                            }
                            SynthesisEntryEnd::Failed => return Ok(SynthesisChallengeEnd::Failed),
                        }
                    }
                    SynthesisStage::Exchange => {
                        self.exchange_started = true;
                        let exchange = self
                            .exchange
                            .as_mut()
                            .ok_or_else(|| anyhow::anyhow!("合成子流程丢失"))?;
                        let end = exchange.execute_with_budget(
                            session,
                            game,
                            remaining(),
                            |exchange| {
                                self.snapshot.inputs_released = exchange.inputs_released;
                                self.snapshot.exchange = Some(exchange);
                                self.snapshot.active_ms =
                                    initial.saturating_add(started.elapsed().as_millis() as u64);
                                publish(self.snapshot.clone())
                            },
                        )?;
                        self.snapshot.exchange = Some(exchange.snapshot());
                        if let Some(end) = self.interrupted() {
                            return Ok(end);
                        }
                        match end {
                            SynthesisEnd::Completed => {
                                self.exchange
                                    .as_ref()
                                    .ok_or_else(|| anyhow::anyhow!("合成子流程丢失"))?
                                    .result_for_parent()?;
                                self.snapshot.stage = SynthesisStage::Completed;
                            }
                            SynthesisEnd::Paused => return Ok(SynthesisChallengeEnd::Paused),
                            SynthesisEnd::Stopped => return Ok(SynthesisChallengeEnd::Stopped),
                            SynthesisEnd::TimedOut => return Ok(SynthesisChallengeEnd::TimedOut),
                            SynthesisEnd::Failed => return Ok(SynthesisChallengeEnd::Failed),
                        }
                    }
                    SynthesisStage::Completed => return Ok(SynthesisChallengeEnd::Completed),
                }
            }
        })();
        self.snapshot.inputs_released = session.release_inputs().is_ok();
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        self.end = Some(SynthesisChallengeEnd::Failed);
        ensure!(self.snapshot.inputs_released, "合成父任务收尾无法释放输入");
        let mut end = result?;
        publish(self.snapshot.clone())?;
        self.snapshot.active_ms = initial.saturating_add(started.elapsed().as_millis() as u64);
        if let Some(i) = self.interrupted() {
            end = i;
        } else if remaining().is_zero() {
            end = SynthesisChallengeEnd::TimedOut;
        }
        if end == SynthesisChallengeEnd::Paused && !self.control.acknowledge_pause() {
            end = SynthesisChallengeEnd::Stopped;
        }
        self.end = Some(end);
        Ok(end)
    }
}
