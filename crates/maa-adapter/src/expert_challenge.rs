//! 专业挑战室独立电量坐标，完成 / 奖励与返回必须分别在新帧确认。
pub use crate::suibian_adventure::AdventureInputEnd as ExpertInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    expert_challenge::{
        BurnoutState, ExpertEntryAction, ExpertEntryFlow, ExpertSettlementFlow, ExpertView,
    },
    simulation_cards::CardsCharge,
    simulation_settlement::{SettlementAction, SettlementView},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
struct Sight {
    goal: Option<od_core::agent_goal::AgentGoalStatus>,
    view: ExpertView,
    next: Option<[i32; 2]>,
    finish: Option<[i32; 2]>,
    confirm: Option<[i32; 2]>,
    mode: Option<[i32; 2]>,
    confirm_mode: Option<[i32; 2]>,
}
impl Sight {
    fn settlement(&self) -> SettlementView {
        match self.view {
            ExpertView::Unknown => SettlementView::Unknown,
            ExpertView::Blocked | ExpertView::RestorePopup | ExpertView::ConfirmBurnoutOff => {
                SettlementView::Blocked
            }
            ExpertView::Detail {
                charge,
                burnout: BurnoutState::Off,
            } if od_core::expert_challenge::valid_charge(charge) => {
                SettlementView::Returned(charge)
            }
            ExpertView::Detail { .. } => SettlementView::Blocked,
            ExpertView::Settlement => SettlementView::Settlement,
            ExpertView::Reward => SettlementView::Reward,
            ExpertView::Failed => SettlementView::Failed,
            ExpertView::Deployment => SettlementView::Other,
        }
    }
}
impl NativeSession {
    pub fn load_expert_challenge_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换专业挑战室资源");
        self.release_inputs()?;
        let root = validate_expert_challenge_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("专业挑战室路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "专业挑战室资源载入失败"
        );
        self.expert_challenge_bundle = Some(root);
        Ok(())
    }
    pub fn require_expert_challenge(&self) -> Result<()> {
        validate_expert_challenge_bundle(
            self.expert_challenge_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    fn expert_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let result = (|| -> Result<_> {
            let r = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::expert_challenge_pipeline::pipeline(),
            };
            let read = |n: &str| words(&r.read(&format!("Expert.{n}"), None)?);
            let mut s = Sight {
                goal: None,
                view: ExpertView::Unknown,
                next: None,
                finish: None,
                confirm: None,
                mode: None,
                confirm_mode: None,
            };
            let modal = read("Modal")?;
            if modal.iter().any(|w| w.text == "恢复电量") {
                s.view = ExpertView::RestorePopup;
                return Ok(s);
            }
            let prompt = modal.iter().map(|w| w.text.as_str()).collect::<String>();
            let prompt = od_core::charge_transport::normalized_name(&prompt);
            let closing = prompt.contains("燃竭模式") && prompt.contains("关闭");
            if closing && !prompt.contains("开启") {
                s.confirm_mode = point(&read("ModeConfirm")?, "确认", [1030, 602, 1226, 652]);
                if s.confirm_mode.is_some() {
                    s.view = ExpertView::ConfirmBurnoutOff;
                    return Ok(s);
                }
            }
            // 仅处理已知战斗奖励版式；其他“获得”弹窗由其自己的子流程处理。
            if point(&modal, "获得", [919, 395, 999, 451]).is_some()
                && point(&modal, "前往", [788, 702, 866, 756]).is_some()
            {
                s.confirm = point(&modal, "确认", [1091, 701, 1169, 756]);
                if s.confirm.is_some() {
                    s.view = ExpertView::Reward;
                    return Ok(s);
                }
            }
            if modal.iter().any(|w| {
                [
                    "确认",
                    "确定",
                    "取消",
                    "获得",
                    "快捷使用",
                    "以太电池",
                    "确定并出战",
                    "确定並出战",
                ]
                .contains(&w.text.as_str())
            }) {
                s.view = ExpertView::Blocked;
                return Ok(s);
            }
            s.next = point(&read("Next")?, "下一步", [1590, 1000, 1850, 1055]);
            if s.next.is_some()
                && point(&read("Level")?, "挑战等级", [1454, 26, 1586, 80]).is_some()
            {
                let number = |name: &str| -> Result<Option<u32>> {
                    let values = read(name)?;
                    Ok((values.len() == 1).then(|| &values[0].text).and_then(|v| {
                        (!v.is_empty() && v.len() <= 7 && v.bytes().all(|b| b.is_ascii_digit()))
                            .then(|| v.parse().ok())
                            .flatten()
                    }))
                };
                if let (Some(current_charge), Some(required_charge)) =
                    (number("Current")?, number("Required")?)
                    && current_charge <= 1_000_000
                    && (1..=1000).contains(&required_charge)
                {
                    let modes = read("Mode")?;
                    let burnout = if modes.len() == 1
                        && point(&modes, "OFF", [1658, 672, 1754, 722]).is_some()
                    {
                        BurnoutState::Off
                    } else if modes.len() == 1
                        && point(&modes, "ON", [1658, 672, 1754, 722]).is_some()
                        && point(&read("ModeOn")?, "ON", [1658, 672, 1754, 722]).is_some()
                    {
                        s.mode = point(&modes, "ON", [1658, 672, 1754, 722]);
                        BurnoutState::On
                    } else {
                        BurnoutState::Unknown
                    };
                    s.view = ExpertView::Detail {
                        charge: CardsCharge {
                            current_charge,
                            required_charge,
                        },
                        burnout,
                    };
                }
                return Ok(s);
            }
            if point(&read("Deploy")?, "出战", [1634, 1000, 1832, 1054]).is_some()
                && point(&read("Team")?, "预备编队", [368, 1002, 563, 1057]).is_some()
            {
                s.view = ExpertView::Deployment;
                return Ok(s);
            }
            for (n, text, roi) in [
                ("Retreat", "撤退", [526, 848, 746, 912]),
                ("Rewind", "倒带", [874, 850, 1088, 914]),
                ("Exit", "退出", [1566, 1004, 1832, 1056]),
            ] {
                if point(&read(n)?, text, roi).is_some() {
                    s.view = ExpertView::Failed;
                    return Ok(s);
                }
            }
            s.finish = point(&read("Finish")?, "完成", [1566, 1004, 1832, 1056]);
            // 上游单场收尾直接选“完成”，并不要求同时存在可重试按钮。
            // 父任务仍须先取得本次正常战斗结果，不能从既有结果页登记。
            if s.finish.is_some() {
                s.view = ExpertView::Settlement;
                if self.agent_goal_bundle.is_some() {
                    s.goal = self.agent_goal_on_settlement(
                        frame,
                        cancel,
                        timeout.saturating_sub(started.elapsed()),
                    )?;
                }
            }
            Ok(s)
        })();
        if cancel.load(Ordering::Acquire) {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
    pub fn inspect_expert_challenge(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<ExpertView>> {
        self.require_expert_challenge()?;
        Ok(self.expert_sight(frame, cancel, timeout)?.map(|s| s.view))
    }
    fn read_expert_sight(&self, cancel: &AtomicBool, timeout: Duration) -> Result<Option<Sight>> {
        let started = Instant::now();
        self.require_expert_challenge()?;
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let s = self.expert_sight(
            &frame.image,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        if cancel.load(Ordering::Acquire)
            || started.elapsed() >= timeout
            || frame.age() > Duration::from_millis(1500)
        {
            return Ok(None);
        }
        Ok(s)
    }
    pub fn read_expert_challenge(
        &self,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<ExpertView>> {
        Ok(self.read_expert_sight(cancel, timeout)?.map(|s| s.view))
    }
    pub fn read_expert_challenge_settlement(
        &self,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<SettlementView>> {
        Ok(self
            .read_expert_sight(cancel, timeout)?
            .map(|s| s.settlement()))
    }
    pub fn read_expert_challenge_agent_settlement(
        &self,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<od_core::agent_goal::AgentSettlementObservation>> {
        self.require_agent_goal()?;
        Ok(self.read_expert_sight(cancel, timeout)?.map(|s| {
            od_core::agent_goal::AgentSettlementObservation {
                view: s.settlement(),
                goal: s.goal,
            }
        }))
    }
    pub fn act_expert_challenge_agent_finish(
        &mut self,
        flow: &ExpertSettlementFlow,
        goal: od_core::agent_goal::AgentGoalStatus,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<ExpertInputEnd> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_agent_goal()?;
        self.expert_challenge_click(cancel, timeout, game, |s| {
            if s.goal != Some(goal) || !flow.allows(SettlementAction::Finish, &s.settlement()) {
                return None;
            }
            s.finish
        })
    }
    pub fn act_expert_challenge_entry(
        &mut self,
        flow: &ExpertEntryFlow,
        a: ExpertEntryAction,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<ExpertInputEnd> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.expert_challenge_click(cancel, timeout, game, |s| {
            flow.allows(a, &s.view)
                .then_some(match a {
                    ExpertEntryAction::DisableBurnout => s.mode,
                    ExpertEntryAction::ConfirmBurnoutOff => s.confirm_mode,
                    ExpertEntryAction::Next | ExpertEntryAction::OpenRestore => s.next,
                })
                .flatten()
        })
    }
    pub fn act_expert_challenge_settlement(
        &mut self,
        flow: &ExpertSettlementFlow,
        a: SettlementAction,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<ExpertInputEnd> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.expert_challenge_click(cancel, timeout, game, |s| {
            if !flow.allows(a, &s.settlement()) {
                return None;
            }
            match a {
                SettlementAction::Finish => s.finish,
                SettlementAction::ConfirmReward => s.confirm,
            }
        })
    }
    fn expert_challenge_click(
        &mut self,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
        target: impl FnOnce(&Sight) -> Option<[i32; 2]>,
    ) -> Result<ExpertInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<_> {
            self.require_expert_challenge()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(ExpertInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(ExpertInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(ExpertInputEnd::NotPosted);
            };
            let sight = self.expert_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(ExpertInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(ExpertInputEnd::TimedOut);
            }
            let Some(s) = sight else {
                return Ok(ExpertInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) {
                return Ok(ExpertInputEnd::Changed);
            }
            let Some([x, y]) = target(&s) else {
                return Ok(ExpertInputEnd::Changed);
            };
            let input = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(ExpertInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(100)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input, false, &AtomicBool::new(false))?;
            Ok(ExpertInputEnd::Posted)
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}

pub fn validate_expert_challenge_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("专业挑战室资源缺失")?;
    let path = root.join("expert_challenge-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "专业挑战室清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sources = match manifest["schema_version"].as_u64() {
        Some(1) => include_str!("../../../resources/expert-challenge/source-lock.json"),
        _ => anyhow::bail!("专业挑战室资源版本不兼容"),
    };
    ensure!(
        manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["expert_challenge"])
            && manifest["sources"] == serde_json::from_str::<Value>(sources)?,
        "专业挑战室资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let pipeline = serde_json::to_vec_pretty(&od_core::expert_challenge_pipeline::pipeline())?;
    expected.insert(
        "pipeline/expert_challenge.json".into(),
        (
            pipeline.len() as u64,
            hex::encode(Sha256::digest(&pipeline)),
        ),
    );
    for file in ocr["files"].as_array().context("OCR 清单无效")? {
        if let Some(name) = file["bundle_path"].as_str() {
            expected.insert(
                name.into(),
                (
                    file["bytes"].as_u64().context("OCR 大小无效")?,
                    file["sha256"].as_str().context("OCR 哈希无效")?.into(),
                ),
            );
        }
    }
    let files = manifest["files"]
        .as_array()
        .context("专业挑战室文件清单无效")?;
    ensure!(files.len() == expected.len(), "专业挑战室文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("专业挑战室文件路径无效")?;
        let (size, hash) = expected.get(name).context("专业挑战室资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "专业挑战室文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "专业挑战室文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "专业挑战室文件内容不匹配"
        );
    }
    for entry in walkdir::WalkDir::new(&root).min_depth(1) {
        let entry = entry?;
        od_core::config::reject_links(entry.path())?;
        let name = entry
            .path()
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        ensure!(
            if entry.file_type().is_file() {
                name == "expert_challenge-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "专业挑战室资源含额外文件或目录"
        );
    }
    Ok(root)
}
