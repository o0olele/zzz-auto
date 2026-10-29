//! 同帧核对手册页签、条目和前往按钮；确认后须重新读到模拟室入口。
pub use crate::suibian_adventure::AdventureInputEnd as TransportInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, Word, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::compendium::{
    TransportAction as A, TransportFlow, TransportRequest, TransportView as V,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(crate) struct Sight {
    pub(crate) view: V,
    pub(crate) training: Option<[i32; 2]>,
    pub(crate) category: Option<[i32; 2]>,
    go: Option<[i32; 2]>,
    confirm: Option<[i32; 2]>,
    icon: Option<[i32; 2]>,
}
fn normalize_type(w: &mut Word) {
    if w.text == "代理人晉升" {
        w.text = "代理人晋升".into();
    }
}
fn paired_go(types: &[Word], buttons: &[Word], target: &str) -> Option<[i32; 2]> {
    let titles: Vec<_> = types.iter().filter(|w| w.text == target).collect();
    if titles.len() != 1 {
        return None;
    }
    let y = titles[0].center()[1];
    let bottom = types
        .iter()
        .map(|w| w.center()[1])
        .filter(|other| *other > y)
        .min()
        .unwrap_or(890)
        .min(y + 180);
    let candidates: Vec<_> = buttons
        .iter()
        .filter(|w| w.text == "前往" && w.center()[1] > y && w.center()[1] < bottom)
        .collect();
    (candidates.len() == 1).then(|| candidates[0].center())
}
impl NativeSession {
    pub fn load_compendium_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换快捷手册资源");
        self.release_inputs()?;
        let root = validate_compendium_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("快捷手册路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "快捷手册资源载入失败"
        );
        self.compendium_bundle = Some(root);
        Ok(())
    }
    pub fn require_compendium(&self) -> Result<()> {
        validate_compendium_bundle(
            self.compendium_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    pub(crate) fn compendium_sight(
        &self,
        frame: &RecognitionImage,
        request: &TransportRequest,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let started = Instant::now();
        let result = (|| -> Result<Sight> {
            let r = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::compendium_pipeline::pipeline(),
            };
            let mut s = Sight {
                view: V::Unknown,
                training: None,
                category: None,
                go: None,
                confirm: None,
                icon: None,
            };
            let modal = words(&r.read("Compendium.Area.Modal", None)?)?;
            let prompt = modal
                .iter()
                .any(|w| w.text.trim_end_matches(['?', '？', '。']) == "是否立即传送");
            s.confirm = point(
                &words(&r.read("Compendium.Area.Confirm", None)?)?,
                "确认",
                [1030, 602, 1230, 650],
            );
            if prompt && s.confirm.is_some() {
                s.view = V::Confirm;
                return Ok(s);
            }
            if modal.iter().any(|w| {
                [
                    "确认",
                    "确定",
                    "取消",
                    "获得",
                    "获得奖励",
                    "恢复电量",
                    "快捷使用",
                ]
                .contains(&w.text.as_str())
            }) || s.confirm.is_some()
                || prompt
            {
                s.view = V::Blocked;
                return Ok(s);
            }
            let tabs = words(&r.read("Compendium.Area.Tabs", None)?)?;
            let tab_count = ["目标", "日常", "训练", "作战", "战术"]
                .iter()
                .filter(|t| point(&tabs, t, [708, 140, 1636, 220]).is_some())
                .count();
            s.training = point(&tabs, "训练", [708, 140, 1636, 220]);
            if tab_count >= 4 && s.training.is_some() {
                let selected = words(&r.read("Compendium.Selected", None)?)?;
                let mut training = point(&selected, "训练", [708, 140, 1636, 220]).is_some()
                    && selected.len() == 1;
                if !training {
                    // 黄底掩码可能破坏黑字。原色完整文字定位后，复核同位置黄底，
                    // 不降低 OCR 阈值，也不把过滤失败当作未选中或借用邻页签的颜色。
                    let labels: Vec<_> = tabs.iter().filter(|w| w.text == "训练").collect();
                    if labels.len() == 1 {
                        let [x, y, w, h] = labels[0].rect;
                        if x >= 708
                            && y >= 140
                            && w > 0
                            && h > 0
                            && x.checked_add(w).is_some_and(|r| r <= 1636)
                            && y.checked_add(h).is_some_and(|b| b <= 220)
                        {
                            let left = (x - 12).max(708);
                            let top = (y - 10).max(140);
                            let width = (x + w + 12).min(1636) - left;
                            let height = (y + h + 10).min(220) - top;
                            let mut params = r.pipeline["Compendium.TabColor"].clone();
                            params
                                .as_object_mut()
                                .context("训练颜色节点无效")?
                                .remove("recognition");
                            params["roi"] = json!([left, top, width, height]);
                            params["count"] = json!((width * height / 5).max(1));
                            training = self
                                .recognize_prepared(frame, "ColorMatch", &params)?
                                .detail["hit"]
                                == true;
                            ensure!(!cancel.load(Ordering::Acquire), "快捷手册识别已取消");
                            ensure!(started.elapsed() < timeout, "快捷手册识别超时");
                        }
                    }
                }
                s.category = point(
                    &words(&r.read("Compendium.Area.Categories", None)?)?,
                    "实战模拟室",
                    [164, 287, 429, 870],
                );
                let mut types = words(&r.read("Compendium.Area.Types", None)?)?;
                for w in &mut types {
                    normalize_type(w);
                }
                let names = od_core::charge_plan::catalog()[0]
                    .mission_type_list
                    .iter()
                    .filter(|m| types.iter().any(|w| w.text == m.mission_type_name))
                    .map(|m| m.mission_type_name.clone())
                    .collect();
                s.go = if training {
                    paired_go(
                        &types,
                        &words(&r.read("Compendium.Area.Go", None)?)?,
                        request.transport_type(),
                    )
                } else {
                    None
                };
                s.view = V::Handbook {
                    training,
                    simulation_category: s.category.is_some(),
                    types: names,
                    target_go: s.go.is_some(),
                };
                return Ok(s);
            }
            if r.hit("Compendium.WorldFunction", None)? && r.hit("Compendium.WorldPreset", None)? {
                let value = r.read("Compendium.Icon", None)?;
                if value["hit"] == true {
                    let [x, y, w, h]: [i32; 4] =
                        serde_json::from_value(value["detail"]["best"]["box"].clone())?;
                    s.icon = Some([x + w / 2, y + h / 2]);
                }
                s.view = V::World;
                return Ok(s);
            }
            if let Some(simulation) =
                self.simulation_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?
            {
                s.view = match simulation.view {
                    od_core::simulation_cards::SimulationView::Unknown => V::Unknown,
                    od_core::simulation_cards::SimulationView::Blocked => V::Blocked,
                    view => V::Simulation(view),
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
    /// 固定图只报告可见版式，不提供传送或关卡完成证明。
    pub fn inspect_compendium(
        &self,
        image: &RecognitionImage,
        request: &TransportRequest,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        request.validate().map_err(anyhow::Error::msg)?;
        self.require_compendium()?;
        Ok(self
            .compendium_sight(image, request, cancel, timeout)?
            .map(|s| s.view))
    }
    pub fn read_compendium(
        &self,
        request: &TransportRequest,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        request.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        self.require_compendium()?;
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        ensure!(started.elapsed() < timeout, "快捷手册读帧超时");
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.compendium_sight(
            &frame.image,
            request,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        ensure!(started.elapsed() < timeout, "快捷手册读帧超时");
        if frame.age() > Duration::from_millis(1500) {
            return Ok(None);
        }
        Ok(sight.map(|s| s.view))
    }
    pub fn act_compendium(
        &mut self,
        flow: &TransportFlow,
        action: A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<TransportInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<TransportInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_compendium()?;
            ensure!(
                !self
                    .input_mode
                    .context("快捷手册控制器无输入方式")?
                    .is_gamepad(),
                "快捷手册传送须交接到键鼠控制器"
            );
            if cancel.load(Ordering::Acquire) {
                return Ok(TransportInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(TransportInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(TransportInputEnd::NotPosted);
            };
            let sight = self.compendium_sight(
                &frame.image,
                flow.request(),
                cancel,
                timeout.saturating_sub(started.elapsed()),
            );
            if cancel.load(Ordering::Acquire) {
                return Ok(TransportInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(TransportInputEnd::TimedOut);
            }
            let Some(sight) = sight? else {
                return Ok(TransportInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.view) {
                return Ok(TransportInputEnd::Changed);
            }
            let location = match action {
                // Alt 显示按钮后单独获取新帧，不能使用显示前的坐标。
                A::Open => Some([0, 0]),
                A::Training => sight.training,
                A::Category => sight.category,
                A::Go => sight.go,
                A::Confirm => sight.confirm,
                A::Scroll(_) => Some([573, 596]),
            };
            let Some([mut x, mut y]) = location else {
                return Ok(TransportInputEnd::Changed);
            };
            let alt = od_core::InputProbeAction::Key { key: 18 };
            if action == A::Open {
                if !self.combat_input(alt, true, cancel)? {
                    return Ok(TransportInputEnd::NotPosted);
                }
                let Some(revealed) = self.navigation_frame(cancel)? else {
                    return Ok(TransportInputEnd::NotPosted);
                };
                let sight = self.compendium_sight(
                    &revealed.image,
                    flow.request(),
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                );
                if cancel.load(Ordering::Acquire) {
                    return Ok(TransportInputEnd::NotPosted);
                }
                if started.elapsed() >= timeout {
                    return Ok(TransportInputEnd::TimedOut);
                }
                let Some(sight) = sight? else {
                    return Ok(TransportInputEnd::NotPosted);
                };
                if revealed.age() > Duration::from_millis(1500) || sight.view != V::World {
                    return Ok(TransportInputEnd::Changed);
                }
                let Some(location) = sight.icon else {
                    return Ok(TransportInputEnd::Changed);
                };
                [x, y] = location;
            }
            let input = |x, y| od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input(x, y), true, cancel)? {
                return Ok(TransportInputEnd::NotPosted);
            }
            let held = Instant::now();
            let duration = if matches!(action, A::Scroll(_)) {
                300
            } else {
                100
            };
            while held.elapsed() < Duration::from_millis(duration)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                if matches!(action, A::Scroll(_)) {
                    let offset = (held.elapsed().as_millis().min(300) * 300 / 300) as i32;
                    self.combat_input(input(x, y - offset), true, cancel)?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input(x, y), false, &AtomicBool::new(false))?;
            if action == A::Open {
                self.combat_input(alt, false, &AtomicBool::new(false))?;
            }
            let wait = Instant::now();
            while wait.elapsed() < Duration::from_millis(500)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(TransportInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|end| release.map(|_| end))
    }
}
pub fn validate_compendium_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("快捷手册传送资源缺失")?;
    let path = root.join("compendium-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "快捷手册传送清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sources = match manifest["schema_version"].as_u64() {
        Some(1) => include_str!("../../../resources/compendium/source-lock-v1.json"),
        Some(2) => include_str!("../../../resources/compendium/source-lock.json"),
        _ => anyhow::bail!("快捷手册传送资源版本不兼容"),
    };
    ensure!(
        manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["compendium"])
            && manifest["sources"] == serde_json::from_str::<Value>(sources)?,
        "快捷手册传送资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let pipeline = serde_json::to_vec_pretty(&od_core::compendium_pipeline::pipeline())?;
    expected.insert(
        "pipeline/compendium.json".into(),
        (
            pipeline.len() as u64,
            hex::encode(Sha256::digest(&pipeline)),
        ),
    );
    for (name, bytes) in od_core::compendium_pipeline::templates() {
        expected.insert(
            format!("image/compendium/{name}.png"),
            (bytes.len() as u64, hex::encode(Sha256::digest(bytes))),
        );
    }
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
        .context("快捷手册传送文件清单无效")?;
    ensure!(files.len() == expected.len(), "快捷手册传送文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("快捷手册传送文件路径无效")?;
        let (size, hash) = expected.get(name).context("快捷手册传送资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "快捷手册传送文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "快捷手册传送文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "快捷手册传送文件内容不匹配"
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
                name == "compendium-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "快捷手册传送资源含额外文件或目录"
        );
    }
    Ok(root)
}
