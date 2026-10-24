//! 固定模拟室关卡列表和同帧输入。选择回执始终 selection_verified=false。
pub use crate::suibian_adventure::AdventureInputEnd as MissionInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    simulation_cards::{CardsCharge, SimulationView},
    simulation_mission::{
        MissionAction as A, MissionFlow, MissionLabel, MissionRequest, MissionView as V,
    },
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
    pub fn load_simulation_mission_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换关卡资源");
        self.release_inputs()?;
        let root = validate_simulation_mission_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("关卡资源路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "关卡资源载入失败"
        );
        self.simulation_mission_bundle = Some(root);
        Ok(())
    }
    pub fn require_simulation_mission(&self) -> Result<()> {
        validate_simulation_mission_bundle(
            self.simulation_mission_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    fn simulation_mission_sight(
        &self,
        frame: &RecognitionImage,
        request: &MissionRequest,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let started = Instant::now();
        let result = (|| -> Result<Sight> {
            let mut sight = Sight {
                view: V::Unknown,
                back: None,
                kind: None,
                mission: None,
            };
            let Some(base) = self.simulation_sight(frame, cancel, timeout)? else {
                return Ok(sight);
            };
            let r = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::simulation_mission_pipeline::pipeline(),
            };
            match base.view {
                SimulationView::Unknown => {}
                SimulationView::Blocked => sight.view = V::Blocked,
                SimulationView::Categories { names } => {
                    let mut list = words(&r.read("Simulation.Area.Types", None)?)?;
                    for w in &mut list {
                        if w.text == "代理人晉升" {
                            w.text = "代理人晋升".into();
                        }
                    }
                    sight.kind = point(&list, &request.mission_type, [406, 592, 1528, 828]);
                    sight.view = V::Categories { names };
                }
                SimulationView::Challenge {
                    current_charge,
                    required_charge,
                } => {
                    let labels = words(&r.read("Mission.Names", None)?)?
                        .into_iter()
                        .map(|w| MissionLabel {
                            text: w.text,
                            rect: w.rect,
                        })
                        .collect::<Vec<_>>();
                    // 使用旧版 MatchResult 左上角 + (0,50)，点击前再确认完整名称及列表边界。
                    let candidates: Vec<_> = labels
                        .iter()
                        .filter(|w| w.text == request.mission_name)
                        .collect();
                    if candidates.len() == 1
                        && candidates[0].validate().is_ok()
                        && candidates[0].rect[1] + 50 < 882
                    {
                        sight.mission = Some([candidates[0].rect[0], candidates[0].rect[1] + 50]);
                    }
                    let back = r.read("Mission.Back", None)?;
                    if back["hit"] == true {
                        let [x, y, w, h]: [i32; 4] =
                            serde_json::from_value(back["detail"]["best"]["box"].clone())?;
                        sight.back = Some([x + w / 2, y + h / 2]);
                    }
                    sight.view = V::Challenge {
                        labels,
                        charge: CardsCharge {
                            current_charge,
                            required_charge,
                        },
                        back: sight.back.is_some(),
                    };
                }
                _ => sight.view = V::Other,
            }
            Ok(sight)
        })();
        if cancel.load(Ordering::Acquire) {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
    pub fn inspect_simulation_mission(
        &self,
        image: &RecognitionImage,
        request: &MissionRequest,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        request.validate().map_err(anyhow::Error::msg)?;
        self.require_simulation_mission()?;
        Ok(self
            .simulation_mission_sight(image, request, cancel, timeout)?
            .map(|s| s.view))
    }
    pub fn read_simulation_mission(
        &self,
        request: &MissionRequest,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        request.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        self.require_simulation_mission()?;
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        ensure!(started.elapsed() < timeout, "关卡读帧超时");
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.simulation_mission_sight(
            &frame.image,
            request,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        ensure!(started.elapsed() < timeout, "关卡读帧超时");
        if frame.age() > Duration::from_millis(1500) {
            return Ok(None);
        }
        Ok(sight.map(|s| s.view))
    }
    pub fn act_simulation_mission(
        &mut self,
        flow: &MissionFlow,
        action: A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<MissionInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<MissionInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_simulation_mission()?;
            ensure!(
                !self
                    .input_mode
                    .context("关卡控制器无输入方式")?
                    .is_gamepad(),
                "关卡选择须交接到键鼠控制器"
            );
            if cancel.load(Ordering::Acquire) {
                return Ok(MissionInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(MissionInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(MissionInputEnd::NotPosted);
            };
            let sight = self.simulation_mission_sight(
                &frame.image,
                flow.request(),
                cancel,
                timeout.saturating_sub(started.elapsed()),
            );
            if cancel.load(Ordering::Acquire) {
                return Ok(MissionInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(MissionInputEnd::TimedOut);
            }
            let Some(sight) = sight? else {
                return Ok(MissionInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.view) {
                return Ok(MissionInputEnd::Changed);
            }
            let location = match action {
                A::Back => sight.back,
                A::ChooseType => sight.kind,
                A::ChooseMission => sight.mission,
                A::PanLeft(_) | A::PanRight(_) => Some([962, 803]),
            };
            let Some([x, y]) = location else {
                return Ok(MissionInputEnd::Changed);
            };
            let input = |x, y| od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input(x, y), true, cancel)? {
                return Ok(MissionInputEnd::NotPosted);
            }
            let held = Instant::now();
            let duration = if matches!(action, A::PanLeft(_) | A::PanRight(_)) {
                300
            } else {
                100
            };
            while held.elapsed() < Duration::from_millis(duration)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                if matches!(action, A::PanLeft(_) | A::PanRight(_)) {
                    let offset = (held.elapsed().as_millis().min(300) * 400 / 300) as i32;
                    self.combat_input(
                        input(
                            if matches!(action, A::PanLeft(_)) {
                                x - offset
                            } else {
                                x + offset
                            },
                            y,
                        ),
                        true,
                        cancel,
                    )?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input(x, y), false, &AtomicBool::new(false))?;
            let wait = Instant::now();
            while wait.elapsed() < Duration::from_millis(500)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(MissionInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|end| release.map(|_| end))
    }
}
pub fn validate_simulation_mission_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("模拟室关卡选择资源缺失")?;
    let path = root.join("simulation_mission-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "模拟室关卡选择清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        manifest["schema_version"] == 1
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["simulation_mission"])
            && manifest["sources"]
                == serde_json::from_str::<Value>(include_str!(
                    "../../../resources/simulation-mission/source-lock.json"
                ))?,
        "模拟室关卡选择资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let pipeline = serde_json::to_vec_pretty(&od_core::simulation_mission_pipeline::pipeline())?;
    expected.insert(
        "pipeline/simulation_mission.json".into(),
        (
            pipeline.len() as u64,
            hex::encode(Sha256::digest(&pipeline)),
        ),
    );
    for (name, bytes) in od_core::simulation_mission_pipeline::templates() {
        expected.insert(
            format!("image/simulation_mission/{name}.png"),
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
        .context("模拟室关卡选择文件清单无效")?;
    ensure!(
        files.len() == expected.len(),
        "模拟室关卡选择文件数量不匹配"
    );
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"]
            .as_str()
            .context("模拟室关卡选择文件路径无效")?;
        let (size, hash) = expected.get(name).context("模拟室关卡选择资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "模拟室关卡选择文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "模拟室关卡选择文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "模拟室关卡选择文件内容不匹配"
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
                name == "simulation_mission-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "模拟室关卡选择资源含额外文件或目录"
        );
    }
    Ok(root)
}
