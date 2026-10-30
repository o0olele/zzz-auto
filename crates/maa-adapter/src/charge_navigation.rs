//! 仅在新帧确认的体力页面上返回；未知弹窗不使用通用确认或兜底返回。
pub use crate::suibian_adventure::AdventureInputEnd as ChargeNavInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    ether_synthesis_entry::SynthesisEntryObservation,
    suibian_adventure::{Reader, words},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    charge_navigation::{ChargeNavAction as A, ChargeNavFlow, ChargeNavView as V},
    compendium::{TransportAction, TransportFlow, TransportRequest, TransportView},
    ether_synthesis::SynthesisView,
    simulation_cards::SimulationView,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// 动态详情候选只留在内存；任何输入都重新截图，不把坐标写入检查点。
pub struct ChargeNavObservation {
    view: V,
    target: Option<[i32; 2]>,
    detail: Option<SynthesisEntryObservation>,
}
impl ChargeNavObservation {
    pub fn view(&self) -> V {
        self.view
    }
}
fn request() -> TransportRequest {
    TransportRequest {
        mission_type: "基础材料".into(),
        timeout_ms: 30_000,
    }
}
impl NativeSession {
    pub fn load_charge_navigation_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换体力导航资源");
        self.release_inputs()?;
        let root = validate_charge_navigation_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("体力导航路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "体力导航资源载入失败"
        );
        self.charge_navigation_bundle = Some(root);
        Ok(())
    }
    pub fn require_charge_navigation(&self) -> Result<()> {
        validate_charge_navigation_bundle(
            self.charge_navigation_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        self.require_compendium()?;
        self.require_ether_synthesis()?;
        self.require_ether_synthesis_entry()?;
        Ok(())
    }
    fn charge_navigation_sight(
        &self,
        frame: &RecognitionImage,
        detail: Option<&SynthesisEntryObservation>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<ChargeNavObservation>> {
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
                pipeline: od_core::charge_navigation_pipeline::pipeline(),
            };
            let mut s = ChargeNavObservation {
                view: V::Unknown,
                target: None,
                detail: None,
            };
            let template = |node: &str| -> Result<Option<[i32; 2]>> {
                let hit = r.read(node, None)?;
                if hit["hit"] != true {
                    return Ok(None);
                }
                let rect: [i32; 4] = serde_json::from_value(hit["detail"]["best"]["box"].clone())?;
                ensure!(od_core::intel_board::valid_rect(rect), "体力导航模板框无效");
                let [x, y, w, h] = rect;
                Ok(Some([x + w / 2, y + h / 2]))
            };
            let modal = words(&r.read("ChargeNav.Modal", None)?)?;
            if modal.iter().any(|w| {
                w.text.contains("确认使用")
                    || [
                        "取消",
                        "确认",
                        "确定",
                        "获得",
                        "获得奖励",
                        "恢复电量",
                        "快捷使用",
                    ]
                    .contains(&w.text.as_str())
            }) {
                s.view = V::Blocked;
                return Ok(s);
            }
            // 有详情覆盖层时，不能把背后的手册或大世界当作到达证据。
            if let Some(detail) = detail {
                if self.recheck_synthesis_detail(
                    frame,
                    detail,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                )? {
                    s.target = template("ChargeNav.Close")?;
                    s.view = V::SynthesisDetail {
                        close: s.target.is_some(),
                    };
                }
                return Ok(s);
            }
            if modal.iter().any(|w| w.text == "以太电池") {
                s.detail = self.observe_synthesis_detail(
                    frame,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                )?;
                if s.detail.is_some() {
                    s.target = template("ChargeNav.Close")?;
                    s.view = V::SynthesisDetail {
                        close: s.target.is_some(),
                    };
                } else {
                    s.view = V::Blocked;
                }
                return Ok(s);
            }
            if words(&r.read("ChargeNav.Title", None)?)?
                .iter()
                .any(|w| w.text == "道具处理")
            {
                match self.inspect_ether_synthesis(
                    frame,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                )? {
                    Some(SynthesisView::Page { .. }) => {
                        s.target = template("ChargeNav.Back")?;
                        s.view = V::SynthesisPage {
                            back: s.target.is_some(),
                        };
                    }
                    Some(
                        SynthesisView::Blocked
                        | SynthesisView::Confirmation
                        | SynthesisView::Obtained,
                    ) => s.view = V::Blocked,
                    _ => (),
                }
                return Ok(s);
            }
            // 可选区域能力由自己的独立资源提供；旧导航包仍保留原定义。
            if self.deep_hunt_bundle.is_some()
                && self.deep_hunt_navigation_context(
                    frame,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                )?
            {
                s.target = template("ChargeNav.Back")?;
                s.view = V::HuntPage {
                    back: s.target.is_some(),
                };
                return Ok(s);
            }
            if self.expert_challenge_bundle.is_some()
                && let Some(od_core::expert_challenge::ExpertView::Detail { .. }) = self
                    .inspect_expert_challenge(
                        frame,
                        cancel,
                        timeout.saturating_sub(started.elapsed()),
                    )?
            {
                s.target = template("ChargeNav.Back")?;
                s.view = V::ExpertPage {
                    back: s.target.is_some(),
                };
                return Ok(s);
            }
            if self.area_patrol_bundle.is_some()
                && let Some(od_core::area_patrol::PatrolView::Detail(_)) = self
                    .inspect_area_patrol(frame, cancel, timeout.saturating_sub(started.elapsed()))?
            {
                s.target = template("ChargeNav.Back")?;
                s.view = V::PatrolPage {
                    back: s.target.is_some(),
                };
                return Ok(s);
            }
            if let Some(c) = self.compendium_sight(
                frame,
                &request(),
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )? {
                s.view = match c.view {
                    TransportView::World => V::World,
                    TransportView::Handbook {
                        training,
                        simulation_category,
                        types,
                        target_go,
                    } => {
                        s.target = if !training {
                            c.training
                        } else if !(target_go && types.iter().any(|t| t == "基础材料")) {
                            c.category
                        } else {
                            None
                        };
                        V::Handbook {
                            training,
                            simulation_category,
                            simulation_ready: training
                                && target_go
                                && types.iter().any(|t| t == "基础材料"),
                        }
                    }
                    TransportView::Simulation(
                        SimulationView::Categories { .. } | SimulationView::Challenge { .. },
                    ) => {
                        s.target = template("ChargeNav.Back")?;
                        V::SimulationPage {
                            back: s.target.is_some(),
                        }
                    }
                    TransportView::Unknown | TransportView::Simulation(SimulationView::Unknown) => {
                        V::Unknown
                    }
                    _ => V::Blocked,
                };
            }
            Ok(s)
        })();
        if cancel.load(Ordering::Acquire) {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
    /// 离线诊断只报告版式，不能提供已到达或任务完成回执。
    pub fn inspect_charge_navigation(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        self.require_charge_navigation()?;
        Ok(self
            .charge_navigation_sight(frame, None, cancel, timeout)?
            .map(|s| s.view))
    }
    pub fn read_charge_navigation(
        &self,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<ChargeNavObservation>> {
        let started = Instant::now();
        self.require_charge_navigation()?;
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.charge_navigation_sight(
            &frame.image,
            None,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        if cancel.load(Ordering::Acquire)
            || started.elapsed() >= timeout
            || (frame.age() > Duration::from_millis(1500)
                && sight.as_ref().is_none_or(|s| s.detail.is_none()))
        {
            return Ok(None);
        }
        Ok(sight)
    }
    pub fn act_charge_navigation(
        &mut self,
        flow: &ChargeNavFlow,
        action: A,
        observation: &ChargeNavObservation,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<ChargeNavInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<_> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_charge_navigation()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(ChargeNavInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(ChargeNavInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(ChargeNavInputEnd::NotPosted);
            };
            let sight = self.charge_navigation_sight(
                &frame.image,
                if action == A::CloseSynthesisDetail {
                    observation.detail.as_ref()
                } else {
                    None
                },
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(ChargeNavInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(ChargeNavInputEnd::TimedOut);
            }
            let Some(sight) = sight else {
                return Ok(ChargeNavInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, sight.view) {
                return Ok(ChargeNavInputEnd::Changed);
            }
            if action == A::OpenHandbook {
                let mut open = TransportFlow::new(request()).map_err(anyhow::Error::msg)?;
                ensure!(
                    open.next(&TransportView::World)
                        .map_err(anyhow::Error::msg)?
                        == Some(TransportAction::Open),
                    "打开手册步骤不匹配"
                );
                open.prepare(TransportAction::Open)
                    .map_err(anyhow::Error::msg)?;
                return self.act_compendium(
                    &open,
                    TransportAction::Open,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                    game,
                );
            }
            if action == A::CloseSynthesisDetail && observation.detail.is_none() {
                return Ok(ChargeNavInputEnd::Changed);
            }
            let [x, y] = if action == A::ExitHandbook {
                [1760, 268]
            } else {
                sight.target.context("体力导航缺少同帧目标")?
            };
            if cancel.load(Ordering::Acquire) {
                return Ok(ChargeNavInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(ChargeNavInputEnd::TimedOut);
            }
            let input = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(ChargeNavInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(100)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input, false, &AtomicBool::new(false))?;
            let wait = Instant::now();
            while wait.elapsed() < Duration::from_millis(500)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(ChargeNavInputEnd::Posted)
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}

pub fn validate_charge_navigation_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("体力页面导航资源缺失")?;
    let path = root.join("charge_navigation-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "体力页面导航清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sources = match manifest["schema_version"].as_u64() {
        Some(1) => include_str!("../../../resources/charge-navigation/source-lock.json"),
        _ => anyhow::bail!("体力页面导航资源版本不兼容"),
    };
    ensure!(
        manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["charge_navigation"])
            && manifest["sources"] == serde_json::from_str::<Value>(sources)?,
        "体力页面导航资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    for (name, bytes) in od_core::charge_navigation_pipeline::templates() {
        expected.insert(
            format!("image/charge-navigation/{name}.png"),
            (bytes.len() as u64, hex::encode(Sha256::digest(bytes))),
        );
    }
    let pipeline = serde_json::to_vec_pretty(&od_core::charge_navigation_pipeline::pipeline())?;
    expected.insert(
        "pipeline/charge_navigation.json".into(),
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
        .context("体力页面导航文件清单无效")?;
    ensure!(files.len() == expected.len(), "体力页面导航文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("体力页面导航文件路径无效")?;
        let (size, hash) = expected.get(name).context("体力页面导航资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "体力页面导航文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "体力页面导航文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "体力页面导航文件内容不匹配"
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
                name == "charge_navigation-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "体力页面导航资源含额外文件或目录"
        );
    }
    Ok(root)
}
