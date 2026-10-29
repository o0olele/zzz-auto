//! 培养详情按参考标记定位；这里只提供未验证的选择投递回执。
pub use crate::suibian_adventure::AdventureInputEnd as AgentMissionInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    agent_plan::{AgentMarkerArea, inspect_agent_markers},
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    agent_mission::{
        AgentMissionAction as A, AgentMissionFlow, AgentMissionRequest, AgentMissionTarget,
        AgentMissionView as V,
    },
    simulation_cards::{CardsCharge, SimulationView},
    simulation_mission::MissionLabel,
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
    back: Option<[i32; 2]>,
    kind: Option<[i32; 2]>,
    mission: Option<[i32; 2]>,
}
impl NativeSession {
    pub fn load_agent_mission_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换培养详情选关资源");
        self.release_inputs()?;
        let root = validate_agent_mission_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("选关资源路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "培养详情资源载入失败"
        );
        self.agent_mission_bundle = Some(root);
        Ok(())
    }
    pub fn require_agent_mission(&self) -> Result<()> {
        validate_agent_mission_bundle(
            self.agent_mission_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    fn agent_mission_sight(
        &self,
        frame: &RecognitionImage,
        request: &AgentMissionRequest,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let result = (|| -> Result<Sight> {
            let mut s = Sight {
                view: V::Unknown,
                back: None,
                kind: None,
                mission: None,
            };
            let Some(base) = self.simulation_sight(frame, cancel, timeout)? else {
                return Ok(s);
            };
            let r = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::agent_mission_pipeline::pipeline(),
            };
            match base.view {
                SimulationView::Unknown => (),
                SimulationView::Blocked => s.view = V::Blocked,
                SimulationView::Categories { names } => {
                    let mut list = words(&r.read("Simulation.Area.Types", None)?)?;
                    for w in &mut list {
                        if w.text == "代理人晉升" {
                            w.text = "代理人晋升".into();
                        }
                    }
                    s.kind = point(&list, &request.mission_type, [406, 592, 1528, 828]);
                    s.view = V::Categories { names };
                }
                SimulationView::Challenge {
                    current_charge,
                    required_charge,
                } => {
                    let labels: Vec<_> = words(&r.read("AgentMission.Names", None)?)?
                        .into_iter()
                        .map(|w| MissionLabel {
                            text: w.text,
                            rect: w.rect,
                        })
                        .collect();
                    let mut candidate = None;
                    if request.known_list(&labels).unwrap_or(false) {
                        // 只使用参考轮廓顺序的第一个标记；名字按横向列对应，位置只留当前帧。
                        if let Some(marker) =
                            inspect_agent_markers(frame, AgentMarkerArea::Simulation)?.first()
                        {
                            let [x, y] = marker.target;
                            let names = request.names().map_err(anyhow::Error::msg)?;
                            let paired: Vec<_> = labels
                                .iter()
                                .filter(|l| {
                                    names.contains(&l.text.as_str())
                                        && l.rect[0] <= x
                                        && x < l.rect[0] + l.rect[2]
                                })
                                .collect();
                            if paired.len() == 1
                                && (128..1796).contains(&x)
                                && (724..882).contains(&y)
                            {
                                let t = AgentMissionTarget {
                                    mission_name: paired[0].text.clone(),
                                    marker_sha256: marker.fingerprint.clone(),
                                };
                                t.validate(request).map_err(anyhow::Error::msg)?;
                                s.mission = Some([x, y]);
                                candidate = Some(t);
                            }
                        }
                    }
                    let hit = r.read("AgentMission.Back", None)?;
                    if hit["hit"] == true {
                        let [x, y, w, h]: [i32; 4] =
                            serde_json::from_value(hit["detail"]["best"]["box"].clone())?;
                        ensure!(
                            x >= 82 && y >= 13 && w > 0 && h > 0 && x + w <= 150 && y + h <= 90,
                            "培养详情返回模板越界"
                        );
                        s.back = Some([x + w / 2, y + h / 2]);
                    }
                    s.view = V::Challenge {
                        labels,
                        charge: CardsCharge {
                            current_charge,
                            required_charge,
                        },
                        back: s.back.is_some(),
                        candidate,
                    };
                }
                _ => s.view = V::Other,
            }
            Ok(s)
        })();
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
    pub fn inspect_agent_mission(
        &self,
        frame: &RecognitionImage,
        request: &AgentMissionRequest,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        request.validate().map_err(anyhow::Error::msg)?;
        self.require_agent_mission()?;
        Ok(self
            .agent_mission_sight(frame, request, cancel, timeout)?
            .map(|s| s.view))
    }
    pub fn read_agent_mission(
        &self,
        request: &AgentMissionRequest,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        let started = Instant::now();
        request.validate().map_err(anyhow::Error::msg)?;
        self.require_agent_mission()?;
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let s = self.agent_mission_sight(
            &frame.image,
            request,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        Ok(
            s.filter(|_| frame.age() <= Duration::from_millis(1500) && started.elapsed() < timeout)
                .map(|s| s.view),
        )
    }
    pub fn act_agent_mission(
        &mut self,
        flow: &AgentMissionFlow,
        action: A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<AgentMissionInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<_> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_agent_mission()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(AgentMissionInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(AgentMissionInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(AgentMissionInputEnd::NotPosted);
            };
            let sight = self.agent_mission_sight(
                &frame.image,
                flow.request(),
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(AgentMissionInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(AgentMissionInputEnd::TimedOut);
            }
            let Some(sight) = sight else {
                return Ok(AgentMissionInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.view) {
                return Ok(AgentMissionInputEnd::Changed);
            }
            let position = match action {
                A::Back => sight.back,
                A::ChooseType => sight.kind,
                A::ChooseMission => sight.mission,
                A::Scroll(_) => Some([863, 758]),
            };
            let Some([x, y]) = position else {
                return Ok(AgentMissionInputEnd::Changed);
            };
            let input = |x, y| od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input(x, y), true, cancel)? {
                return Ok(AgentMissionInputEnd::NotPosted);
            }
            let scrolling = matches!(action, A::Scroll(_));
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(if scrolling { 300 } else { 100 })
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                if scrolling {
                    self.combat_input(
                        input(
                            x - 400 * held.elapsed().as_millis().min(300) as i32 / 300,
                            y,
                        ),
                        true,
                        cancel,
                    )?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let complete =
                if scrolling && !cancel.load(Ordering::Acquire) && started.elapsed() < timeout {
                    self.combat_input(input(x - 400, y), true, cancel)?
                } else {
                    !scrolling
                };
            self.combat_input(input(x, y), false, &AtomicBool::new(false))?;
            if !complete {
                return Ok(AgentMissionInputEnd::Changed);
            }
            let wait = Instant::now();
            while wait.elapsed() < Duration::from_millis(500)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(AgentMissionInputEnd::Posted)
        })();
        self.release_inputs()?;
        result
    }
}
pub fn validate_agent_mission_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("模拟室培养选关资源缺失")?;
    let path = root.join("agent_mission-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "模拟室培养选关清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sources = match manifest["schema_version"].as_u64() {
        Some(1) => include_str!("../../../resources/agent-mission/source-lock.json"),
        _ => anyhow::bail!("模拟室培养选关资源版本不兼容"),
    };
    ensure!(
        manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["agent_mission"])
            && manifest["sources"] == serde_json::from_str::<Value>(sources)?,
        "模拟室培养选关资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    for (name, bytes) in od_core::agent_mission_pipeline::templates() {
        expected.insert(
            format!("image/agent-mission/{name}.png"),
            (bytes.len() as u64, hex::encode(Sha256::digest(bytes))),
        );
    }
    let pipeline = serde_json::to_vec_pretty(&od_core::agent_mission_pipeline::pipeline())?;
    expected.insert(
        "pipeline/agent_mission.json".into(),
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
        .context("模拟室培养选关文件清单无效")?;
    ensure!(
        files.len() == expected.len(),
        "模拟室培养选关文件数量不匹配"
    );
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"]
            .as_str()
            .context("模拟室培养选关文件路径无效")?;
        let (size, hash) = expected.get(name).context("模拟室培养选关资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "模拟室培养选关文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "模拟室培养选关文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "模拟室培养选关文件内容不匹配"
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
                name == "agent_mission-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "模拟室培养选关资源含额外文件或目录"
        );
    }
    Ok(root)
}
