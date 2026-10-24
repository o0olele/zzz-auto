//! 模拟室结算的同帧复验与输入；不点击再来一次或写入完成次数。
pub use crate::suibian_adventure::AdventureInputEnd as SettlementInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    simulation_cards::{CardsCharge, SimulationView},
    simulation_settlement::{SettlementAction as A, SettlementFlow, SettlementView as V},
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
    finish: Option<[i32; 2]>,
    confirm: Option<[i32; 2]>,
}
impl NativeSession {
    pub fn load_simulation_settlement_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换模拟室结算资源");
        self.release_inputs()?;
        let root = validate_simulation_settlement_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("结算资源路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "结算资源载入失败"
        );
        self.simulation_settlement_bundle = Some(root);
        Ok(())
    }
    pub fn require_simulation_settlement(&self) -> Result<()> {
        validate_simulation_settlement_bundle(
            self.simulation_settlement_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    fn simulation_settlement_sight(
        &self,
        frame: &RecognitionImage,
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
                finish: None,
                confirm: None,
            };
            let r = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::simulation_settlement_pipeline::pipeline(),
            };
            let Some(base) = self.simulation_sight(frame, cancel, timeout)? else {
                return Ok(sight);
            };
            // 弹窗先于背景的失败与结算按钮。
            match base.view {
                SimulationView::Blocked => {
                    sight.view = V::Blocked;
                    return Ok(sight);
                }
                SimulationView::Reward => {
                    let modal = words(&r.read("Simulation.Area.Modal", None)?)?;
                    sight.confirm = point(&modal, "确认", [1091, 701, 1169, 756]);
                    if sight.confirm.is_some() {
                        sight.view = V::Reward;
                    }
                    return Ok(sight);
                }
                _ => {}
            }
            for (name, text, roi) in [
                ("Settlement.Retreat", "撤退", [526, 848, 746, 912]),
                ("Settlement.Rewind", "倒带", [874, 850, 1088, 914]),
                ("Settlement.Exit", "退出", [1566, 1004, 1832, 1056]),
            ] {
                if point(&words(&r.read(name, None)?)?, text, roi).is_some() {
                    sight.view = V::Failed;
                    return Ok(sight);
                }
            }
            sight.view = match base.view {
                SimulationView::Settlement => {
                    sight.finish = point(
                        &words(&r.read("Simulation.Area.Finish", None)?)?,
                        "完成",
                        [1566, 1004, 1832, 1056],
                    );
                    if sight.finish.is_some() {
                        V::Settlement
                    } else {
                        V::Unknown
                    }
                }
                SimulationView::Challenge {
                    current_charge,
                    required_charge,
                } => V::Returned(CardsCharge {
                    current_charge,
                    required_charge,
                }),
                SimulationView::Unknown => V::Unknown,
                _ => V::Other,
            };
            Ok(sight)
        })();
        if cancel.load(Ordering::Acquire) {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
    pub fn inspect_simulation_settlement(
        &self,
        image: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        self.require_simulation_settlement()?;
        Ok(self
            .simulation_settlement_sight(image, cancel, timeout)?
            .map(|s| s.view))
    }
    pub fn read_simulation_settlement(
        &self,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        let started = Instant::now();
        self.require_simulation_settlement()?;
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        ensure!(started.elapsed() < timeout, "结算识别超时");
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.simulation_settlement_sight(
            &frame.image,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        Ok(sight
            .filter(|_| started.elapsed() < timeout && frame.age() <= Duration::from_millis(1500))
            .map(|s| s.view))
    }
    pub fn act_simulation_settlement(
        &mut self,
        flow: &SettlementFlow,
        action: A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<SettlementInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<SettlementInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                self.game_window.is_some() == game
                    && self.input_mode == Some(crate::InputMode::Foreground),
                "结算控制器模式不匹配"
            );
            self.require_simulation_settlement()?;
            if cancel.load(Ordering::Acquire) {
                return Ok(SettlementInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(SettlementInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(SettlementInputEnd::NotPosted);
            };
            let sight = self.simulation_settlement_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            );
            if cancel.load(Ordering::Acquire) {
                return Ok(SettlementInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(SettlementInputEnd::TimedOut);
            }
            let Some(sight) = sight? else {
                return Ok(SettlementInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.view) {
                return Ok(SettlementInputEnd::Changed);
            }
            let location = match action {
                A::Finish => sight.finish,
                A::ConfirmReward => sight.confirm,
            };
            let Some([x, y]) = location else {
                return Ok(SettlementInputEnd::Changed);
            };
            let input = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(SettlementInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(70)
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
            Ok(SettlementInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|end| release.map(|_| end))
    }
}

pub fn validate_simulation_settlement_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("模拟室结算资源缺失")?;
    let path = root.join("simulation_settlement-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "模拟室结算清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        manifest["schema_version"] == 1
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["simulation_settlement"])
            && manifest["sources"]
                == serde_json::from_str::<Value>(include_str!(
                    "../../../resources/simulation-settlement/source-lock.json"
                ))?,
        "模拟室结算资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let pipeline = serde_json::to_vec_pretty(&od_core::simulation_settlement_pipeline::pipeline())?;
    expected.insert(
        "pipeline/simulation_settlement.json".into(),
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
        .context("模拟室结算文件清单无效")?;
    ensure!(files.len() == expected.len(), "模拟室结算文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("模拟室结算文件路径无效")?;
        let (size, hash) = expected.get(name).context("模拟室结算资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "模拟室结算文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "模拟室结算文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "模拟室结算文件内容不匹配"
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
                name == "simulation_settlement-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "模拟室结算资源含额外文件或目录"
        );
    }
    Ok(root)
}
