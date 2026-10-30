//! 实战模拟室同帧读取和卡片输入；不启动或结算战斗。
pub use crate::suibian_adventure::AdventureInputEnd as CardsInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::simulation_cards::{CardsAction as A, CardsCharge, CardsFlow, SimulationView as V};
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
    save: Option<[i32; 2]>,
    next: Option<[i32; 2]>,
}
fn number(reader: &Reader<'_>, name: &str) -> Result<Option<u32>> {
    let words = words(&reader.read(name, None)?)?;
    Ok(
        if words.len() == 1
            && !words[0].text.is_empty()
            && words[0].text.bytes().all(|b| b.is_ascii_digit())
        {
            words[0].text.parse().ok().filter(|n| *n <= 1_000_000)
        } else {
            None
        },
    )
}
impl NativeSession {
    pub fn load_simulation_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换实战模拟室资源");
        self.release_inputs()?;
        let root = validate_simulation_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("模拟室路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "模拟室资源载入失败"
        );
        self.simulation_bundle = Some(root);
        Ok(())
    }
    pub fn require_simulation(&self) -> Result<()> {
        validate_simulation_bundle(
            self.simulation_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    pub(crate) fn simulation_sight(
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
            let r = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::simulation_pipeline::pipeline(),
            };
            let modal = words(&r.read("Simulation.Area.Modal", None)?)?;
            let mut sight = Sight {
                view: V::Unknown,
                save: None,
                next: None,
            };
            // 实战模拟室专有的“前往 + 确认”领奖版式。恢复和普通弹窗均阻止背景输入。
            let reward = point(&modal, "获得", [919, 395, 999, 451]).is_some()
                && point(&modal, "前往", [788, 702, 866, 756]).is_some()
                && point(&modal, "确认", [1091, 701, 1169, 756]).is_some();
            if reward {
                sight.view = V::Reward;
                return Ok(sight);
            }
            if modal.iter().any(|w| {
                ["恢复电量", "快捷使用", "获得", "确认", "取消", "确定并出战"]
                    .contains(&w.text.as_str())
            }) {
                sight.view = V::Blocked;
                return Ok(sight);
            }
            let save = words(&r.read("Simulation.Area.Save", None)?)?;
            sight.save = point(&save, "保存方案", [1600, 1008, 1834, 1048]);
            if sight.save.is_some() {
                sight.view = V::Cards;
                return Ok(sight);
            }
            sight.next = point(
                &words(&r.read("Simulation.Area.Next", None)?)?,
                "下一步",
                [1590, 1000, 1850, 1055],
            );
            let level = point(
                &words(&r.read("Simulation.Area.Level", None)?)?,
                "挑战等级",
                [1454, 26, 1586, 80],
            )
            .is_some();
            if sight.next.is_some() && level {
                if let (Some(current_charge), Some(required_charge)) = (
                    number(&r, "Simulation.Area.Current")?,
                    number(&r, "Simulation.Area.Required")?,
                ) && (CardsCharge {
                    current_charge,
                    required_charge,
                })
                .validate()
                .is_ok()
                {
                    sight.view = V::Challenge {
                        current_charge,
                        required_charge,
                    };
                }
                return Ok(sight);
            }
            let deploy = words(&r.read("Simulation.Area.Deploy", None)?)?;
            let team = words(&r.read("Simulation.Area.Team", None)?)?;
            if ["出战", "预备出战"]
                .iter()
                .any(|t| point(&deploy, t, [1634, 1000, 1832, 1054]).is_some())
                && point(&team, "预备编队", [368, 1002, 563, 1057]).is_some()
            {
                sight.view = V::Deployment;
                return Ok(sight);
            }
            if point(
                &words(&r.read("Simulation.Area.Finish", None)?)?,
                "完成",
                [1566, 1004, 1832, 1056],
            )
            .is_some()
                && point(
                    &words(&r.read("Simulation.Area.Retry", None)?)?,
                    "再来一次",
                    [1232, 1006, 1508, 1052],
                )
                .is_some()
            {
                sight.view = V::Settlement;
                return Ok(sight);
            }
            let mut types = words(&r.read("Simulation.Area.Types", None)?)?;
            for word in &mut types {
                // 固定 OCR 在组合帧中将“晋升”读为“晉升”；只兼容这一完整类型名。
                if word.text == "代理人晉升" {
                    word.text = "代理人晋升".into();
                }
            }
            let category = od_core::charge_plan::catalog()
                .iter()
                .find(|c| c.category_name == "实战模拟室")
                .unwrap();
            let names: Vec<String> = category
                .mission_type_list
                .iter()
                .filter(|t| point(&types, &t.mission_type_name, [406, 592, 1528, 828]).is_some())
                .map(|t| t.mission_type_name.clone())
                .collect();
            if names.len() >= 3 {
                sight.view = V::Categories { names };
            }
            Ok(sight)
        })();
        if cancel.load(Ordering::Acquire) {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
    /// 固定图片只读识别，不提供当前帧或战斗完成证明。
    pub fn inspect_simulation(
        &self,
        image: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        self.require_simulation()?;
        Ok(self
            .simulation_sight(image, cancel, timeout)?
            .map(|s| s.view))
    }
    pub fn read_simulation(&self, cancel: &AtomicBool, timeout: Duration) -> Result<Option<V>> {
        let started = Instant::now();
        self.require_simulation()?;
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        ensure!(started.elapsed() < timeout, "模拟室识别超时");
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let result = self.simulation_sight(
            &frame.image,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        Ok(result
            .filter(|_| started.elapsed() < timeout && frame.age() <= Duration::from_millis(1500))
            .map(|s| s.view))
    }
    pub fn act_simulation_cards(
        &mut self,
        flow: &CardsFlow,
        action: A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<CardsInputEnd> {
        let started = Instant::now();
        flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "卡片控制器模式不匹配"
        );
        let result = (|| -> Result<CardsInputEnd> {
            self.require_simulation()?;
            if cancel.load(Ordering::Acquire) {
                return Ok(CardsInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(CardsInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(CardsInputEnd::NotPosted);
            };
            let sight = self.simulation_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            );
            if cancel.load(Ordering::Acquire) {
                return Ok(CardsInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(CardsInputEnd::TimedOut);
            }
            let Some(sight) = sight? else {
                return Ok(CardsInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.view) {
                return Ok(CardsInputEnd::Changed);
            }
            let [x, y] = match action {
                A::Open => [282, 440],
                A::Clear(_) => [348, 186],
                A::Add(_) => [652, 310],
                A::Save => match sight.save {
                    Some(p) => p,
                    None => return Ok(CardsInputEnd::Changed),
                },
            };
            let input = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(CardsInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(70)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input, false, &AtomicBool::new(false))?;
            // 沿用旧版每次数量调整后的等待，暂停或超时可提前结束等待；回执仍表示已投递。
            let wait = Instant::now();
            while wait.elapsed() < Duration::from_millis(500)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(CardsInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|v| release.map(|_| v))
    }

    /// 只为实际不足的冻结电量打开恢复弹窗，不点击出战或再来一次。
    pub fn act_simulation_restore_open(
        &mut self,
        flow: &od_core::simulation_preparation::PreparationFlow,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<CardsInputEnd> {
        self.act_simulation_next(
            cancel,
            timeout,
            game,
            |view| {
                matches!(view, V::Challenge { current_charge, required_charge }
                if flow.allows_open(CardsCharge {
                    current_charge: *current_charge, required_charge: *required_charge,
                }))
            },
            |session| {
                flow.validate().map_err(anyhow::Error::msg)?;
                session.require_restore_charge()
            },
        )
    }

    /// 足额详情推进与恢复弹窗共用输入入口，冻结条件各自复验。
    pub fn act_simulation_entry_next(
        &mut self,
        flow: &od_core::simulation_entry::EntryFlow,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<CardsInputEnd> {
        self.act_simulation_next(
            cancel,
            timeout,
            game,
            |view| flow.allows_next(view),
            |_| flow.validate().map_err(anyhow::Error::msg),
        )
    }

    fn act_simulation_next(
        &mut self,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
        allows: impl Fn(&V) -> bool,
        validate: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<CardsInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<CardsInputEnd> {
            validate(self)?;
            ensure!(
                self.game_window.is_some() == game
                    && self.input_mode == Some(crate::InputMode::Foreground),
                "模拟室准备控制器模式不匹配"
            );
            self.require_simulation()?;
            if cancel.load(Ordering::Acquire) {
                return Ok(CardsInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(CardsInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(CardsInputEnd::NotPosted);
            };
            let sight = self.simulation_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            );
            if cancel.load(Ordering::Acquire) {
                return Ok(CardsInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(CardsInputEnd::TimedOut);
            }
            let Some(sight) = sight? else {
                return Ok(CardsInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !allows(&sight.view) {
                return Ok(CardsInputEnd::Changed);
            }
            let Some([x, y]) = sight.next else {
                return Ok(CardsInputEnd::Changed);
            };
            let input = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(CardsInputEnd::NotPosted);
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
            Ok(CardsInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|end| release.map(|_| end))
    }
}

pub fn validate_simulation_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("实战模拟室资源缺失")?;
    let path = root.join("simulation-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "实战模拟室清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        manifest["schema_version"] == 1
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["simulation"])
            && manifest["sources"]
                == serde_json::from_str::<Value>(include_str!(
                    "../../../resources/simulation/source-lock.json"
                ))?,
        "实战模拟室资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let pipeline = serde_json::to_vec_pretty(&od_core::simulation_pipeline::pipeline())?;
    expected.insert(
        "pipeline/simulation.json".into(),
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
        .context("实战模拟室文件清单无效")?;
    ensure!(files.len() == expected.len(), "实战模拟室文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("实战模拟室文件路径无效")?;
        let (size, hash) = expected.get(name).context("实战模拟室资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "实战模拟室文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "实战模拟室文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "实战模拟室文件内容不匹配"
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
                name == "simulation-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "实战模拟室资源含额外文件或目录"
        );
    }
    Ok(root)
}
