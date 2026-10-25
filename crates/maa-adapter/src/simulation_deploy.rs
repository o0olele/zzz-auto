//! 出战同帧读取、前移和输入释放；不写入计划次数。
pub use crate::simulation::CardsInputEnd as DeployInputEnd;
use crate::{
    NativeSession,
    combat_vision::{CombatFrame, CombatRecognizer},
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::simulation_deploy::{DeployAction as A, DeployFlow, DeployView as V};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
impl NativeSession {
    pub fn load_simulation_deploy_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换出战资源");
        self.release_inputs()?;
        let root = validate_simulation_deploy_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("出战路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "出战资源载入失败"
        );
        self.simulation_deploy_bundle = Some(root);
        Ok(())
    }
    pub fn require_simulation_deploy(&self) -> Result<()> {
        validate_simulation_deploy_bundle(
            self.simulation_deploy_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    fn simulation_deploy_sight(
        &self,
        frame: &CombatFrame,
        recognizer: &CombatRecognizer,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<(V, Option<[i32; 2]>)> {
        let started = Instant::now();
        let r = Reader {
            session: self,
            frame: &frame.image,
            cancel,
            started,
            timeout,
            pipeline: od_core::simulation_deploy_pipeline::pipeline(),
        };
        let capacity = words(&r.read("Deploy.Area.Capacity", None)?)?;
        if point(
            &capacity,
            "驱动盘数量已达到可拥有上限",
            [720, 500, 1210, 555],
        )
        .is_some()
        {
            return Ok((V::InventoryFull, None));
        }
        let modal = words(&r.read("Deploy.Area.Modal", None)?)?;
        if modal
            .iter()
            .any(|w| ["恢复电量", "快捷使用", "获得"].contains(&w.text.as_str()))
        {
            return Ok((V::Blocked, None));
        }
        let confirm = words(&r.read("Deploy.Area.Confirm", None)?)?;
        for name in ["确定并出战", "确定並出战"] {
            if let Some(p) = point(&confirm, name, [1030, 602, 1222, 656]) {
                return Ok((V::LowLevel, Some(p)));
            }
        }
        if modal
            .iter()
            .any(|w| ["确认", "取消", "确定并出战", "确定並出战"].contains(&w.text.as_str()))
        {
            return Ok((V::Blocked, None));
        }
        let deploy = words(&r.read("Deploy.Area.Deploy", None)?)?;
        let team = words(&r.read("Deploy.Area.Team", None)?)?;
        if let Some(p) = point(&deploy, "出战", [1570, 982, 1820, 1044])
            && point(&team, "预备编队", [368, 1002, 563, 1057]).is_some()
        {
            return Ok((V::Deployment, Some(p)));
        }
        if recognizer
            .normal_battle_result(self, frame, cancel)?
            .is_some()
        {
            return Ok((V::Result, None));
        }
        let attack = recognizer
            .matched(
                self,
                frame,
                "battle/btn_normal_attack",
                recognizer.area("按键-普通攻击")?,
                0.7,
            )?
            .is_some();
        ensure!(started.elapsed() < timeout, "出战识别超时");
        Ok((if attack { V::Battle } else { V::Unknown }, None))
    }
    pub fn inspect_simulation_deploy(
        &self,
        frame: &CombatFrame,
        recognizer: &CombatRecognizer,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<V> {
        self.require_simulation_deploy()?;
        Ok(self
            .simulation_deploy_sight(frame, recognizer, cancel, timeout)?
            .0)
    }
    pub fn read_simulation_deploy(
        &self,
        recognizer: &CombatRecognizer,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        let started = Instant::now();
        self.require_simulation_deploy()?;
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let Some(frame) = self.combat_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.simulation_deploy_sight(
            &frame,
            recognizer,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        );
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout || frame.age_ms() > 1500 {
            return Ok(None);
        }
        sight.map(|(view, _)| Some(view))
    }
    pub fn act_simulation_deploy(
        &mut self,
        flow: &DeployFlow,
        action: A,
        recognizer: &CombatRecognizer,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<DeployInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<DeployInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                self.game_window.is_some() == game
                    && self.input_mode == Some(crate::InputMode::Foreground),
                "出战与前移须使用父任务的前台控制器"
            );
            self.require_simulation_deploy()?;
            if cancel.load(Ordering::Acquire) {
                return Ok(DeployInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(DeployInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.combat_frame(cancel)? else {
                return Ok(DeployInputEnd::NotPosted);
            };
            let sight = self.simulation_deploy_sight(
                &frame,
                recognizer,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            );
            if cancel.load(Ordering::Acquire) {
                return Ok(DeployInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(DeployInputEnd::TimedOut);
            }
            let (view, point) = sight?;
            if frame.age_ms() > 1500 || !flow.allows(action, view) {
                return Ok(DeployInputEnd::Changed);
            }
            let input = if action == A::Forward {
                od_core::InputProbeAction::Key { key: 87 }
            } else {
                let Some([x, y]) = point else {
                    return Ok(DeployInputEnd::Changed);
                };
                od_core::InputProbeAction::MouseButton {
                    button: od_core::MouseButton::Left,
                    x,
                    y,
                }
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(DeployInputEnd::NotPosted);
            }
            let held = Instant::now();
            let hold = Duration::from_millis(if action == A::Forward { 1000 } else { 70 });
            while held.elapsed() < hold
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input, false, &AtomicBool::new(false))?;
            Ok(DeployInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|v| release.map(|_| v))
    }
}

pub fn validate_simulation_deploy_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("模拟室出战资源缺失")?;
    let path = root.join("simulation_deploy-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "模拟室出战清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        manifest["schema_version"] == 1
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["simulation_deploy"])
            && manifest["sources"]
                == serde_json::from_str::<Value>(include_str!(
                    "../../../resources/simulation-deploy/source-lock.json"
                ))?,
        "模拟室出战资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let pipeline = serde_json::to_vec_pretty(&od_core::simulation_deploy_pipeline::pipeline())?;
    expected.insert(
        "pipeline/simulation_deploy.json".into(),
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
        .context("模拟室出战文件清单无效")?;
    ensure!(files.len() == expected.len(), "模拟室出战文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("模拟室出战文件路径无效")?;
        let (size, hash) = expected.get(name).context("模拟室出战资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "模拟室出战文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "模拟室出战文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "模拟室出战文件内容不匹配"
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
                name == "simulation_deploy-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "模拟室出战资源含额外文件或目录"
        );
    }
    Ok(root)
}
