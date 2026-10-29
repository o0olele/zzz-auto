//! 深度追猎独立电量坐标，完成 / 奖励与返回必须分别在新帧确认。
pub use crate::suibian_adventure::AdventureInputEnd as HuntInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    deep_hunt::{
        DeepState, HuntEntryAction, HuntEntryFlow, HuntSettlementFlow, HuntView, WeeklyState,
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
    view: HuntView,
    next: Option<[i32; 2]>,
    finish: Option<[i32; 2]>,
    confirm: Option<[i32; 2]>,
    mode: Option<[i32; 2]>,
    confirm_mode: Option<[i32; 2]>,
    select: Option<[i32; 2]>,
    back: Option<[i32; 2]>,
    levels: BTreeMap<String, [i32; 2]>,
}
impl Sight {
    fn settlement(&self, expected: &str) -> SettlementView {
        match &self.view {
            HuntView::Unknown => SettlementView::Unknown,
            HuntView::Levels { .. } => SettlementView::Blocked,
            HuntView::Detail {
                mission_type,
                weekly: WeeklyState::Exhausted,
                mode: DeepState::On,
                charge: Some(c),
                no_reward: false,
                ..
            } if od_core::deep_hunt::same_mission(expected, mission_type)
                && od_core::deep_hunt::valid_charge(*c) =>
            {
                SettlementView::Returned(*c)
            }
            HuntView::Settlement => SettlementView::Settlement,
            HuntView::Reward => SettlementView::Reward,
            HuntView::Failed => SettlementView::Failed,
            HuntView::Deployment => SettlementView::Other,
            _ => SettlementView::Blocked,
        }
    }
}
impl NativeSession {
    pub fn load_deep_hunt_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换深度追猎资源");
        self.release_inputs()?;
        let root = validate_deep_hunt_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("深度追猎路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "深度追猎资源载入失败"
        );
        self.deep_hunt_bundle = Some(root);
        Ok(())
    }
    pub fn require_deep_hunt(&self) -> Result<()> {
        validate_deep_hunt_bundle(
            self.deep_hunt_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    /// 返回导航只核对本页身份；费用、模式、达成和战斗判定由各自消费流程读取。
    pub(crate) fn deep_hunt_navigation_context(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<bool> {
        self.require_deep_hunt()?;
        let started = Instant::now();
        let r = Reader {
            session: self,
            frame,
            cancel,
            started,
            timeout,
            pipeline: od_core::deep_hunt_pipeline::pipeline(),
        };
        let read = |name: &str| words(&r.read(&format!("Hunt.{name}"), None)?);
        let known =
            |text: &str| od_core::charge_transport::canonical_name("恶名狩猎", text).is_some();
        let valid = if point(&read("Street")?, "街区", [238, 24, 388, 82]).is_some() {
            let titles = read("Title")?;
            if titles.len() != 1 || !known(&titles[0].text) {
                return Ok(false);
            }
            if self.charge_level_bundle.is_some() {
                match self.hunt_level_sight(
                    frame,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                )? {
                    Some(crate::charge_level::LevelSight {
                        view: od_core::charge_level::LevelView::Page(reading),
                        ..
                    }) if reading.choices.is_empty() => (),
                    _ => return Ok(false),
                }
            }
            true
        } else if point(
            &read("ListRewards")?,
            "当期剩余奖励次数",
            [192, 992, 368, 1025],
        )
        .is_some()
        {
            read("Names")?.iter().any(|w| known(&w.text))
        } else {
            false
        };
        Ok(valid && !cancel.load(Ordering::Acquire) && started.elapsed() < timeout)
    }
    fn hunt_sight(
        &self,
        frame: &RecognitionImage,
        expected: &str,
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
                pipeline: od_core::deep_hunt_pipeline::pipeline(),
            };
            let read = |n: &str| words(&r.read(&format!("Hunt.{n}"), None)?);
            let mut s = Sight {
                goal: None,
                view: HuntView::Unknown,
                next: None,
                finish: None,
                confirm: None,
                mode: None,
                confirm_mode: None,
                select: None,
                back: None,
                levels: BTreeMap::new(),
            };
            let modal = read("Modal")?;
            if modal.iter().any(|w| w.text == "恢复电量") {
                s.view = HuntView::RestorePopup;
                return Ok(s);
            }
            let prompt = modal.iter().map(|w| w.text.as_str()).collect::<String>();
            let prompt = od_core::charge_transport::normalized_name(&prompt);
            let closing = prompt.contains("深度追猎") && prompt.contains("开启");
            if closing && !prompt.contains("关闭") {
                s.confirm_mode = point(&read("ModeConfirm")?, "确认", [1030, 602, 1226, 652]);
                if s.confirm_mode.is_some() {
                    s.view = HuntView::ConfirmEnableDeep;
                    return Ok(s);
                }
            }
            // 仅处理已知战斗奖励版式；其他“获得”弹窗由其自己的子流程处理。
            if point(&modal, "获得", [919, 395, 999, 451]).is_some()
                && point(&modal, "前往", [788, 702, 866, 756]).is_some()
            {
                s.confirm = point(&modal, "确认", [1091, 701, 1169, 756]);
                if s.confirm.is_some() {
                    s.view = HuntView::Reward;
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
                s.view = HuntView::Blocked;
                return Ok(s);
            }
            s.next = point(&read("Next")?, "下一步", [1590, 1000, 1850, 1055]);
            let number = |name: &str| -> Result<Option<u32>> {
                let values = read(name)?;
                Ok((values.len() == 1).then(|| &values[0].text).and_then(|v| {
                    (!v.is_empty() && v.len() <= 7 && v.bytes().all(|b| b.is_ascii_digit()))
                        .then(|| v.parse().ok())
                        .flatten()
                }))
            };
            if point(&read("Street")?, "街区", [238, 24, 388, 82]).is_some() {
                let titles = read("Title")?;
                if titles.len() != 1
                    || od_core::charge_transport::canonical_name("恶名狩猎", &titles[0].text)
                        .is_none()
                {
                    return Ok(s);
                }
                let has_label = point(
                    &read("RewardsLabel")?,
                    "剩余奖励次数",
                    [1380, 1004, 1506, 1050],
                )
                .is_some();
                let info = r.hit("Hunt.Info", None)?;
                let weekly = if has_label {
                    match number("RewardsCount")? {
                        Some(0) if info => WeeklyState::Exhausted,
                        Some(n @ 1..=3) if !info => WeeklyState::Remaining(n as u8),
                        _ => WeeklyState::Unknown,
                    }
                } else if info {
                    WeeklyState::Exhausted
                } else {
                    WeeklyState::Unknown
                };
                let no_reward =
                    point(&read("NoReward")?, "无报酬模式", [1602, 1000, 1830, 1055]).is_some();
                let modes = read("Mode")?;
                let mode = if modes.len() == 1
                    && !no_reward
                    && point(&modes, "ON", [1658, 672, 1754, 722]).is_some()
                    && point(&read("ModeOn")?, "ON", [1658, 672, 1754, 722]).is_some()
                {
                    DeepState::On
                } else if no_reward
                    && modes.len() <= 1
                    && (modes.is_empty() || point(&modes, "OFF", [1658, 672, 1754, 722]).is_some())
                {
                    DeepState::Off
                } else {
                    DeepState::Unknown
                };
                if mode == DeepState::Off {
                    s.mode = Some([1706, 697]);
                }
                let charge = match (number("Current")?, number("Required")?) {
                    (Some(current_charge), Some(required_charge))
                        if current_charge <= 1_000_000 && (1..=1000).contains(&required_charge) =>
                    {
                        Some(CardsCharge {
                            current_charge,
                            required_charge,
                        })
                    }
                    _ => None,
                };
                let back = r.read("Hunt.Back", None)?;
                if back["hit"] == true {
                    let [x, y, w, h]: [i32; 4] =
                        serde_json::from_value(back["detail"]["best"]["box"].clone())?;
                    ensure!(
                        x >= 82 && y >= 13 && w > 0 && h > 0 && x + w <= 150 && y + h <= 90,
                        "追猎返回模板框超出登记区域"
                    );
                    s.back = Some([x + w / 2, y + h / 2]);
                }
                let level = if self.charge_level_bundle.is_some() {
                    match self.hunt_level_sight(
                        frame,
                        cancel,
                        timeout.saturating_sub(started.elapsed()),
                    )? {
                        Some(crate::charge_level::LevelSight {
                            view: od_core::charge_level::LevelView::Page(reading),
                            choices,
                        }) => {
                            if !reading.choices.is_empty() {
                                if !od_core::deep_hunt::same_mission(expected, &titles[0].text)
                                    || weekly != WeeklyState::Exhausted
                                    || mode != DeepState::On
                                    || no_reward
                                {
                                    s.view = HuntView::Blocked;
                                    return Ok(s);
                                }
                                s.levels = choices;
                                s.view = HuntView::Levels {
                                    names: reading.choices,
                                };
                                return Ok(s);
                            }
                            reading.current
                        }
                        Some(crate::charge_level::LevelSight {
                            view: od_core::charge_level::LevelView::Blocked,
                            ..
                        }) => {
                            s.view = HuntView::Blocked;
                            return Ok(s);
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                s.view = HuntView::Detail {
                    mission_type: titles[0].text.clone(),
                    weekly,
                    mode,
                    charge,
                    no_reward,
                    back: s.back.is_some(),
                    level,
                };
                return Ok(s);
            }
            if point(
                &read("ListRewards")?,
                "当期剩余奖励次数",
                [192, 992, 368, 1025],
            )
            .is_some()
            {
                let names = read("Names")?;
                if !names.iter().any(|w| {
                    od_core::charge_transport::canonical_name("恶名狩猎", &w.text).is_some()
                }) {
                    return Ok(s);
                }
                let candidates: Vec<_> = names
                    .iter()
                    .filter(|w| {
                        w.inside([240, 216, 1752, 466])
                            && od_core::deep_hunt::same_mission(expected, &w.text)
                    })
                    .collect();
                if candidates.len() == 1 {
                    let [x, y] = candidates[0].center();
                    if (240..1752).contains(&x) && (216..566).contains(&(y + 100)) {
                        s.select = Some([x, y + 100]);
                    }
                }
                s.view = HuntView::List {
                    matches: candidates.len().min(255) as u8,
                };
                return Ok(s);
            }
            if point(&read("Deploy")?, "出战", [1634, 1000, 1832, 1054]).is_some()
                && point(&read("Team")?, "预备编队", [368, 1002, 563, 1057]).is_some()
            {
                s.view = HuntView::Deployment;
                return Ok(s);
            }
            for (n, text, roi) in [
                ("Retreat", "撤退", [526, 848, 746, 912]),
                ("Rewind", "倒带", [874, 850, 1088, 914]),
                ("Exit", "退出", [1566, 1004, 1832, 1056]),
            ] {
                if point(&read(n)?, text, roi).is_some() {
                    s.view = HuntView::Failed;
                    return Ok(s);
                }
            }
            s.finish = point(&read("Finish")?, "完成", [1566, 1004, 1832, 1056]);
            // 上游单场收尾直接选“完成”，并不要求同时存在可重试按钮。
            // 父任务仍须先取得本次正常战斗结果，不能从既有结果页登记。
            if s.finish.is_some() {
                s.view = HuntView::Settlement;
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
    pub fn inspect_deep_hunt(
        &self,
        frame: &RecognitionImage,
        expected: &str,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<HuntView>> {
        self.require_deep_hunt()?;
        Ok(self
            .hunt_sight(frame, expected, cancel, timeout)?
            .map(|s| s.view))
    }
    fn read_hunt_sight(
        &self,
        expected: &str,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        let started = Instant::now();
        self.require_deep_hunt()?;
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let s = self.hunt_sight(
            &frame.image,
            expected,
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
    pub fn read_deep_hunt(
        &self,
        expected: &str,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<HuntView>> {
        Ok(self
            .read_hunt_sight(expected, cancel, timeout)?
            .map(|s| s.view))
    }
    pub fn read_deep_hunt_settlement(
        &self,
        expected: &str,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<SettlementView>> {
        Ok(self
            .read_hunt_sight(expected, cancel, timeout)?
            .map(|s| s.settlement(expected)))
    }
    pub fn read_deep_hunt_agent_settlement(
        &self,
        expected: &str,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<od_core::agent_goal::AgentSettlementObservation>> {
        self.require_agent_goal()?;
        Ok(self.read_hunt_sight(expected, cancel, timeout)?.map(|s| {
            od_core::agent_goal::AgentSettlementObservation {
                view: s.settlement(expected),
                goal: s.goal,
            }
        }))
    }
    pub fn act_deep_hunt_agent_finish(
        &mut self,
        flow: &HuntSettlementFlow,
        goal: od_core::agent_goal::AgentGoalStatus,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<HuntInputEnd> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_agent_goal()?;
        self.deep_hunt_click(cancel, timeout, game, flow.mission_type(), |s| {
            if s.goal != Some(goal)
                || !flow.allows(SettlementAction::Finish, &s.settlement(flow.mission_type()))
            {
                return None;
            }
            s.finish
        })
    }
    pub fn act_deep_hunt_entry(
        &mut self,
        flow: &HuntEntryFlow,
        a: HuntEntryAction,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<HuntInputEnd> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.deep_hunt_click(
            cancel,
            timeout,
            game,
            flow.request().plan.execution_mission_type(),
            |s| {
                flow.allows(a, &s.view)
                    .then_some(match a {
                        HuntEntryAction::OpenLevel => Some([1689, 48]),
                        HuntEntryAction::SelectLevel => {
                            s.levels.get(&flow.request().plan.level).copied()
                        }
                        HuntEntryAction::BackToList => s.back,
                        HuntEntryAction::EnableDeep => s.mode,
                        HuntEntryAction::SelectMission => s.select,
                        HuntEntryAction::ConfirmEnableDeep => s.confirm_mode,
                        HuntEntryAction::Next | HuntEntryAction::OpenRestore => s.next,
                    })
                    .flatten()
            },
        )
    }
    pub fn act_deep_hunt_settlement(
        &mut self,
        flow: &HuntSettlementFlow,
        a: SettlementAction,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<HuntInputEnd> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.deep_hunt_click(cancel, timeout, game, flow.mission_type(), |s| {
            if !flow.allows(a, &s.settlement(flow.mission_type())) {
                return None;
            }
            match a {
                SettlementAction::Finish => s.finish,
                SettlementAction::ConfirmReward => s.confirm,
            }
        })
    }
    fn deep_hunt_click(
        &mut self,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
        expected: &str,
        target: impl FnOnce(&Sight) -> Option<[i32; 2]>,
    ) -> Result<HuntInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<_> {
            self.require_deep_hunt()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(HuntInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(HuntInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(HuntInputEnd::NotPosted);
            };
            let sight = self.hunt_sight(
                &frame.image,
                expected,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(HuntInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(HuntInputEnd::TimedOut);
            }
            let Some(s) = sight else {
                return Ok(HuntInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) {
                return Ok(HuntInputEnd::Changed);
            }
            let Some([x, y]) = target(&s) else {
                return Ok(HuntInputEnd::Changed);
            };
            let input = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(HuntInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(100)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input, false, &AtomicBool::new(false))?;
            Ok(HuntInputEnd::Posted)
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}

pub fn validate_deep_hunt_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("深度追猎资源缺失")?;
    let path = root.join("deep_hunt-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "深度追猎清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sources = match manifest["schema_version"].as_u64() {
        Some(1) => include_str!("../../../resources/deep-hunt/source-lock.json"),
        _ => anyhow::bail!("深度追猎资源版本不兼容"),
    };
    ensure!(
        manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["deep_hunt"])
            && manifest["sources"] == serde_json::from_str::<Value>(sources)?,
        "深度追猎资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let back = od_core::deep_hunt_pipeline::back_template();
    expected.insert(
        "image/deep-hunt/back.png".into(),
        (back.len() as u64, hex::encode(Sha256::digest(back))),
    );
    let info = od_core::deep_hunt_pipeline::info_template();
    expected.insert(
        "image/deep-hunt/info.png".into(),
        (info.len() as u64, hex::encode(Sha256::digest(info))),
    );
    let pipeline = serde_json::to_vec_pretty(&od_core::deep_hunt_pipeline::pipeline())?;
    expected.insert(
        "pipeline/deep_hunt.json".into(),
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
        .context("深度追猎文件清单无效")?;
    ensure!(files.len() == expected.len(), "深度追猎文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("深度追猎文件路径无效")?;
        let (size, hash) = expected.get(name).context("深度追猎资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "深度追猎文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "深度追猎文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "深度追猎文件内容不匹配"
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
                name == "deep_hunt-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "深度追猎资源含额外文件或目录"
        );
    }
    Ok(root)
}
