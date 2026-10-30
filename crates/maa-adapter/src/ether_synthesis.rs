//! 以太电池合成页、消费确认和获得弹窗的同帧身份核对。
pub use crate::suibian_adventure::AdventureInputEnd as SynthesisInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::ether_synthesis::{
    SynthesisAction as A, SynthesisFlow, SynthesisStock, SynthesisView as V,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

fn compact(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .replace(['×', 'x', 'X', '*'], "")
}
struct Sight {
    view: V,
    target: Option<[i32; 2]>,
}
impl NativeSession {
    pub fn check_ether_synthesis_target(&self, game: bool) -> Result<()> {
        self.check_combat_input_mode(od_core::InputMode::Foreground)?;
        ensure!(
            self.game_window.is_some() == game,
            "合成运行目标与实际原生会话不一致；回放不能冒充游戏会话"
        );
        Ok(())
    }
    pub fn load_ether_synthesis_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换电池合成资源");
        self.release_inputs()?;
        let root = validate_ether_synthesis_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("合成路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "电池合成资源载入失败"
        );
        self.ether_synthesis_bundle = Some(root);
        Ok(())
    }
    pub fn require_ether_synthesis(&self) -> Result<()> {
        validate_ether_synthesis_bundle(
            self.ether_synthesis_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    fn synthesis_sight(
        &self,
        frame: &RecognitionImage,
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
                pipeline: od_core::ether_synthesis_pipeline::pipeline(),
            };
            let read = |name: &str| -> Result<Vec<crate::suibian_adventure::Word>> {
                words(&r.read(&format!("Synthesis.{name}"), None)?)
            };
            let confirm = read("ConfirmTitle")?;
            let normalized =
                |s: &str| compact(s).replace(['[', ']', '【', '】', '「', '」', '？', '?'], "");
            if confirm.len() == 1
                && normalized(&confirm[0].text) == "确认使用下列素材合成1个以太电池吗"
            {
                let costs = read("Costs")?;
                let cost_ok = [
                    ("60", [750, 580, 880, 645]),
                    ("电量", [750, 580, 880, 645]),
                    ("1", [890, 580, 1025, 645]),
                    ("储值电卡", [890, 580, 1025, 645]),
                    ("100", [1030, 580, 1180, 645]),
                    ("丁尼", [1030, 580, 1180, 645]),
                ]
                .into_iter()
                .all(|(text, roi)| point(&costs, text, roi).is_some());
                let target = point(&read("Confirm")?, "确认", [980, 700, 1336, 790]);
                let cancel_point = point(&read("Cancel")?, "取消", [674, 700, 960, 790]);
                return Ok(Sight {
                    view: if cost_ok && target.is_some() && cancel_point.is_some() {
                        V::Confirmation
                    } else {
                        V::Blocked
                    },
                    target,
                });
            }
            let reward = read("RewardTitle")?;
            if point(&reward, "获得", [860, 386, 1060, 460]).is_some() {
                let target = point(&read("Collect")?, "确认", [820, 700, 1160, 790]);
                let count = read("RewardCount")?;
                let item = read("RewardItem")?;
                return Ok(Sight {
                    view: if target.is_some()
                        && count.len() == 1
                        && compact(&count[0].text) == "1"
                        && point(&item, "以太电池", [860, 613, 1060, 653]).is_some()
                    {
                        V::Obtained
                    } else {
                        V::Blocked
                    },
                    target,
                });
            }
            // 非目标数量、其他道具确认或通用弹窗不能透过背景采用合成页。
            let modal = read("Modal")?;
            if modal.iter().any(|w| {
                w.text.contains("确认使用") || ["取消", "确认", "获得"].contains(&w.text.as_str())
            }) {
                return Ok(Sight {
                    view: V::Blocked,
                    target: None,
                });
            }
            let title = read("Title")?;
            let item = read("Item")?;
            let quantity = read("Quantity")?;
            let charge = read("Charge")?;
            let stock = (|| {
                if point(&title, "道具处理", [178, 18, 330, 86]).is_none()
                    || item.len() != 1
                    || quantity.len() != 1
                    || charge.len() != 1
                    || compact(&quantity[0].text) != "合成数量1"
                {
                    return None;
                }
                let item = compact(&item[0].text);
                let owned = item.strip_prefix("以太电池")?;
                if owned.is_empty() || !owned.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let charge = compact(&charge[0].text);
                let current = charge.strip_suffix("/60")?;
                if current.is_empty() || !current.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let stock = SynthesisStock {
                    charge: current.parse().ok()?,
                    ether: owned.parse().ok()?,
                };
                stock.validate().ok()?;
                Some(stock)
            })();
            let Some(stock) = stock else {
                return Ok(Sight {
                    view: V::Unknown,
                    target: None,
                });
            };
            let shortage = read("Shortage")?;
            let missing_materials = point(&shortage, "合成素材不足", [1455, 890, 1635, 950])
                .is_some()
                || stock.charge < 60;
            let target = point(&read("Synthesize")?, "合成", [1540, 1000, 1840, 1062]);
            Ok(Sight {
                view: if target.is_some() {
                    V::Page {
                        stock,
                        missing_materials,
                    }
                } else {
                    V::Unknown
                },
                target,
            })
        })();
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
    pub fn inspect_ether_synthesis(
        &self,
        image: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        self.require_ether_synthesis()?;
        Ok(self
            .synthesis_sight(image, cancel, timeout)?
            .map(|s| s.view))
    }
    pub fn read_ether_synthesis(
        &self,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        let started = Instant::now();
        self.require_ether_synthesis()?;
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.synthesis_sight(
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
        Ok(sight.map(|s| s.view))
    }
    pub fn act_ether_synthesis(
        &mut self,
        flow: &SynthesisFlow,
        action: A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<SynthesisInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<SynthesisInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_ether_synthesis()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(SynthesisInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(SynthesisInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(SynthesisInputEnd::NotPosted);
            };
            let sight = self.synthesis_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            );
            if cancel.load(Ordering::Acquire) {
                return Ok(SynthesisInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(SynthesisInputEnd::TimedOut);
            }
            let Some(sight) = sight? else {
                return Ok(SynthesisInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, sight.view) {
                return Ok(SynthesisInputEnd::Changed);
            }
            let [x, y] = sight.target.context("合成动作缺少同帧按钮")?;
            if cancel.load(Ordering::Acquire) {
                return Ok(SynthesisInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(SynthesisInputEnd::TimedOut);
            }
            let input = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(SynthesisInputEnd::NotPosted);
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
            Ok(SynthesisInputEnd::Posted)
        })();
        let released = self.release_inputs();
        released?;
        result
    }
}
pub fn validate_ether_synthesis_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("以太电池合成资源缺失")?;
    let path = root.join("ether_synthesis-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "以太电池合成清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sources = match manifest["schema_version"].as_u64() {
        Some(1) => include_str!("../../../resources/ether-synthesis/source-lock.json"),
        _ => anyhow::bail!("以太电池合成资源版本不兼容"),
    };
    ensure!(
        manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["ether_synthesis"])
            && manifest["sources"] == serde_json::from_str::<Value>(sources)?,
        "以太电池合成资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let pipeline = serde_json::to_vec_pretty(&od_core::ether_synthesis_pipeline::pipeline())?;
    expected.insert(
        "pipeline/ether_synthesis.json".into(),
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
        .context("以太电池合成文件清单无效")?;
    ensure!(files.len() == expected.len(), "以太电池合成文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("以太电池合成文件路径无效")?;
        let (size, hash) = expected.get(name).context("以太电池合成资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "以太电池合成文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "以太电池合成文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "以太电池合成文件内容不匹配"
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
                name == "ether_synthesis-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "以太电池合成资源含额外文件或目录"
        );
    }
    Ok(root)
}
