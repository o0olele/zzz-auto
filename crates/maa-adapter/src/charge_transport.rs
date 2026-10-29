//! 同帧核对训练分类与完整副本名称，按分类读取入口；不提供消费许可。
pub use crate::suibian_adventure::AdventureInputEnd as TrainingTransportInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, Word, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::charge_transport::{
    TrainingArrival as Arrival, TrainingTransportAction as A, TrainingTransportFlow,
    TrainingTransportRequest, TrainingTransportView as V, canonical_name,
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
    view: V,
    training: Option<[i32; 2]>,
    category: Option<[i32; 2]>,
    go: Option<[i32; 2]>,
    confirm: Option<[i32; 2]>,
    icon: Option<[i32; 2]>,
}
fn numeric(words: &[Word]) -> Option<u32> {
    if words.len() != 1 {
        return None;
    }
    let s = &words[0].text;
    (!s.is_empty() && s.len() <= 7 && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse::<u32>().ok())
        .flatten()
        .filter(|n| *n <= 1_000_000)
}
fn ratio(words: &[Word]) -> Option<(u32, u32)> {
    if words.len() != 1 {
        return None;
    }
    let (left, right) = words[0].text.split_once('/')?;
    let number = |s: &str| {
        (!s.is_empty() && s.len() <= 7 && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    let (current, required) = (number(left)?, number(right)?);
    (current <= 1_000_000 && (1..=1000).contains(&required)).then_some((current, required))
}
fn paired_go(types: &[Word], buttons: &[Word], target: &str) -> Option<[i32; 2]> {
    let title: Vec<_> = types.iter().filter(|w| w.text == target).collect();
    if title.len() != 1 {
        return None;
    }
    let y = title[0].center()[1];
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
    pub fn load_charge_transport_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换体力传送资源");
        self.release_inputs()?;
        let root = validate_charge_transport_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("体力传送路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "体力传送资源载入失败"
        );
        self.charge_transport_bundle = Some(root);
        Ok(())
    }
    pub fn require_charge_transport(&self) -> Result<()> {
        validate_charge_transport_bundle(
            self.charge_transport_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    fn training_transport_sight(
        &self,
        frame: &RecognitionImage,
        request: &TrainingTransportRequest,
        arrival_only: bool,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let result = (|| -> Result<Sight> {
            let r = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::charge_transport_pipeline::pipeline(),
            };
            let read = |name: &str| words(&r.read(&format!("TrainingTransport.{name}"), None)?);
            let mut s = Sight {
                view: V::Unknown,
                training: None,
                category: None,
                go: None,
                confirm: None,
                icon: None,
            };
            let modal = read("Area.Modal")?;
            let prompt = modal
                .iter()
                .any(|w| w.text.trim_end_matches(['?', '？', '。']) == "是否立即传送");
            s.confirm = point(&read("Area.Confirm")?, "确认", [1030, 602, 1230, 650]);
            if prompt && s.confirm.is_some() {
                s.view = V::Confirm;
                return Ok(s);
            }
            if prompt
                || s.confirm.is_some()
                || modal.iter().any(|w| {
                    [
                        "确认",
                        "确定",
                        "取消",
                        "获得",
                        "获得奖励",
                        "恢复电量",
                        "快捷使用",
                        "以太电池",
                    ]
                    .contains(&w.text.as_str())
                })
            {
                s.view = V::Blocked;
                return Ok(s);
            }
            if !arrival_only {
                let tabs = read("Area.Tabs")?;
                let count = ["目标", "日常", "训练", "作战", "战术"]
                    .iter()
                    .filter(|t| point(&tabs, t, [708, 140, 1636, 220]).is_some())
                    .count();
                s.training = point(&tabs, "训练", [708, 140, 1636, 220]);
                if count >= 4 && s.training.is_some() {
                    let selected = read("Selected")?;
                    let training = selected.len() == 1
                        && point(&selected, "训练", [708, 140, 1636, 220]).is_some();
                    let categories = read("Area.Categories")?;
                    s.category = point(
                        &categories,
                        &request.plan.category_name,
                        [164, 287, 429, 870],
                    );
                    // 蓝底过滤会破坏白字轮廓。用原色完整文字定位，再在同帧、同位置确认蓝底。
                    let candidates: Vec<_> = categories
                        .iter()
                        .filter(|w| w.text == request.plan.category_name)
                        .collect();
                    let category_selected =
                        if training && s.category.is_some() && candidates.len() == 1 {
                            let [x, y, w, h] = candidates[0].rect;
                            ensure!(
                                x >= 164 && y >= 287 && x + w <= 429 && y + h <= 870,
                                "分类文字框超出已登记列表"
                            );
                            let left = (x - 12).max(164);
                            let top = (y - 10).max(287);
                            let width = (x + w + 12).min(429) - left;
                            let height = (y + h + 10).min(870) - top;
                            let mut params = r.pipeline["TrainingTransport.CategoryColor"].clone();
                            params
                                .as_object_mut()
                                .context("分类颜色节点无效")?
                                .remove("recognition");
                            params["roi"] = json!([left, top, width, height]);
                            params["count"] = json!((width * height / 5).max(1));
                            self.recognize_prepared(frame, "ColorMatch", &params)?
                                .detail["hit"]
                                == true
                        } else {
                            false
                        };
                    let mut types = Vec::new();
                    if category_selected {
                        for mut w in read("Area.Types")? {
                            if let Some(name) = canonical_name(&request.plan.category_name, &w.text)
                            {
                                w.text = name.into();
                                types.push(w);
                            }
                        }
                        s.go = paired_go(&types, &read("Area.Go")?, request.mission_type());
                    }
                    s.view = V::Handbook {
                        training,
                        category_visible: s.category.is_some(),
                        category_selected,
                        types: types.iter().map(|w| w.text.clone()).collect(),
                        target_go: s.go.is_some(),
                    };
                    return Ok(s);
                }
                if r.hit("TrainingTransport.WorldFunction", None)?
                    && r.hit("TrainingTransport.WorldPreset", None)?
                {
                    let hit = r.read("TrainingTransport.Icon", None)?;
                    if hit["hit"] == true {
                        let rect: [i32; 4] =
                            serde_json::from_value(hit["detail"]["best"]["box"].clone())?;
                        ensure!(od_core::intel_board::valid_rect(rect), "训练手册图标框无效");
                        let [x, y, w, h] = rect;
                        s.icon = Some([x + w / 2, y + h / 2]);
                    }
                    s.view = V::World;
                    return Ok(s);
                }
            }
            let arrival = match request.plan.category_name.as_str() {
                "区域巡防" | "专业挑战室" => {
                    if point(&read("DetailLevel")?, "挑战等级", [1454, 26, 1586, 80]).is_none()
                        || point(&read("DetailNext")?, "下一步", [1590, 1000, 1850, 1055]).is_none()
                    {
                        None
                    } else if request.plan.category_name == "区域巡防" {
                        ratio(&read("PatrolCharge")?)
                            .map(|(current, required)| Arrival::PatrolDetail { current, required })
                    } else {
                        let current = numeric(&read("ExpertCurrent")?);
                        let required = numeric(&read("ExpertRequired")?);
                        match (current, required) {
                            (Some(current), Some(required)) if (1..=1000).contains(&required) => {
                                Some(Arrival::ExpertDetail { current, required })
                            }
                            _ => None,
                        }
                    }
                }
                "恶名狩猎" => {
                    if point(&read("HuntStreet")?, "街区", [238, 24, 388, 82]).is_some() {
                        let names: Vec<_> = read("HuntTitle")?
                            .into_iter()
                            .filter_map(|w| canonical_name("恶名狩猎", &w.text))
                            .collect();
                        (names.len() == 1).then(|| Arrival::HuntDetail {
                            mission_type: names[0].into(),
                        })
                    } else if point(
                        &read("HuntRewards")?,
                        "当期剩余奖励次数",
                        [192, 992, 368, 1025],
                    )
                    .is_some()
                        && read("HuntNames")?
                            .iter()
                            .any(|w| canonical_name("恶名狩猎", &w.text).is_some())
                    {
                        Some(Arrival::HuntList)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(a) = arrival {
                s.view = V::Arrival(a);
            }
            Ok(s)
        })();
        if cancel.load(Ordering::Acquire) {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
    pub fn inspect_charge_transport(
        &self,
        frame: &RecognitionImage,
        request: &TrainingTransportRequest,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        request.validate().map_err(anyhow::Error::msg)?;
        self.require_charge_transport()?;
        Ok(self
            .training_transport_sight(frame, request, false, cancel, timeout)?
            .map(|s| s.view))
    }
    pub fn read_charge_transport(
        &self,
        flow: &TrainingTransportFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        let started = Instant::now();
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_charge_transport()?;
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.training_transport_sight(
            &frame.image,
            flow.request(),
            flow.awaiting_arrival(),
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        if cancel.load(Ordering::Acquire)
            || started.elapsed() >= timeout
            || frame.age() > Duration::from_millis(1500)
        {
            return Ok(None);
        }
        Ok(sight.map(|s| s.view))
    }
    pub fn act_charge_transport(
        &mut self,
        flow: &TrainingTransportFlow,
        action: A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<TrainingTransportInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<_> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_charge_transport()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(TrainingTransportInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(TrainingTransportInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(TrainingTransportInputEnd::NotPosted);
            };
            let sight = self.training_transport_sight(
                &frame.image,
                flow.request(),
                flow.awaiting_arrival(),
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(TrainingTransportInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(TrainingTransportInputEnd::TimedOut);
            }
            let Some(sight) = sight else {
                return Ok(TrainingTransportInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.view) {
                return Ok(TrainingTransportInputEnd::Changed);
            }
            let target = match action {
                A::Open => Some([0, 0]),
                A::Training => sight.training,
                A::Category => sight.category,
                A::Scroll(_) => Some([573, 596]),
                A::Go => sight.go,
                A::Confirm => sight.confirm,
            };
            let Some([mut x, mut y]) = target else {
                return Ok(TrainingTransportInputEnd::Changed);
            };
            let alt = od_core::InputProbeAction::Key { key: 18 };
            if action == A::Open {
                if !self.combat_input(alt, true, cancel)? {
                    return Ok(TrainingTransportInputEnd::NotPosted);
                }
                let Some(revealed) = self.navigation_frame(cancel)? else {
                    return Ok(TrainingTransportInputEnd::NotPosted);
                };
                let sight = self.training_transport_sight(
                    &revealed.image,
                    flow.request(),
                    false,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                )?;
                if cancel.load(Ordering::Acquire) {
                    return Ok(TrainingTransportInputEnd::NotPosted);
                }
                if started.elapsed() >= timeout {
                    return Ok(TrainingTransportInputEnd::TimedOut);
                }
                let Some(sight) = sight else {
                    return Ok(TrainingTransportInputEnd::NotPosted);
                };
                if revealed.age() > Duration::from_millis(1500) || sight.view != V::World {
                    return Ok(TrainingTransportInputEnd::Changed);
                }
                let Some(point) = sight.icon else {
                    return Ok(TrainingTransportInputEnd::Changed);
                };
                [x, y] = point;
            }
            let input = |x, y| od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input(x, y), true, cancel)? {
                return Ok(TrainingTransportInputEnd::NotPosted);
            }
            let held = Instant::now();
            let hold_ms = if matches!(action, A::Scroll(_)) {
                300
            } else {
                100
            };
            while held.elapsed() < Duration::from_millis(hold_ms)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                if matches!(action, A::Scroll(_)) {
                    let offset = held.elapsed().as_millis().min(300) as i32;
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
            Ok(TrainingTransportInputEnd::Posted)
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}

pub fn validate_charge_transport_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("体力副本传送资源缺失")?;
    let path = root.join("charge_transport-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "体力副本传送清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sources = match manifest["schema_version"].as_u64() {
        Some(1) => include_str!("../../../resources/charge-transport/source-lock.json"),
        _ => anyhow::bail!("体力副本传送资源版本不兼容"),
    };
    ensure!(
        manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["charge_transport"])
            && manifest["sources"] == serde_json::from_str::<Value>(sources)?,
        "体力副本传送资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    for (name, bytes) in od_core::charge_transport_pipeline::templates() {
        expected.insert(
            format!("image/charge-transport/{name}.png"),
            (bytes.len() as u64, hex::encode(Sha256::digest(bytes))),
        );
    }
    let pipeline = serde_json::to_vec_pretty(&od_core::charge_transport_pipeline::pipeline())?;
    expected.insert(
        "pipeline/charge_transport.json".into(),
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
        .context("体力副本传送文件清单无效")?;
    ensure!(files.len() == expected.len(), "体力副本传送文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("体力副本传送文件路径无效")?;
        let (size, hash) = expected.get(name).context("体力副本传送资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "体力副本传送文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "体力副本传送文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "体力副本传送文件内容不匹配"
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
                name == "charge_transport-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "体力副本传送资源含额外文件或目录"
        );
    }
    Ok(root)
}
