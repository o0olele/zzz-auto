//! 电池图标与资源详情动态文字位置。页面交接不等于资源消费。
pub use crate::suibian_adventure::AdventureInputEnd as SynthesisEntryInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    compendium::{TransportRequest, TransportView},
    ether_synthesis::SynthesisView,
    ether_synthesis_entry::{
        SynthesisEntryAction as A, SynthesisEntryFlow, SynthesisEntryView as V,
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
    target: Option<[i32; 2]>,
    detail: Option<([i32; 4], [i32; 4])>,
}
/// 仅在本次读帧和动作间保留候选区域，不写入检查点；动作仍重新截图复核。
pub struct SynthesisEntryObservation(Sight);
impl SynthesisEntryObservation {
    pub fn view(&self) -> V {
        self.0.view
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadMode {
    Inspect,
    Handbook,
    Detail,
    Page,
}
fn modal_blocked(all: &[crate::suibian_adventure::Word]) -> bool {
    all.iter().any(|w| {
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
    })
}
fn expanded([x, y, w, h]: [i32; 4]) -> Result<[i32; 4]> {
    ensure!(
        od_core::intel_board::valid_rect([x, y, w, h]),
        "电池详情候选框无效"
    );
    let left = (x - 12).max(0);
    let top = (y - 8).max(0);
    Ok([
        left,
        top,
        (x + w + 12).min(1920) - left,
        (y + h + 8).min(1080) - top,
    ])
}
fn entry_text(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !['[', ']', '【', '】'].contains(c))
        .collect()
}
impl NativeSession {
    pub(crate) fn observe_synthesis_detail(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<SynthesisEntryObservation>> {
        Ok(self
            .synthesis_entry_sight(frame, cancel, timeout, ReadMode::Detail)?
            .filter(|s| matches!(s.view, V::BatteryDetail { recipe: true }))
            .map(SynthesisEntryObservation))
    }
    pub(crate) fn recheck_synthesis_detail(
        &self,
        frame: &RecognitionImage,
        observation: &SynthesisEntryObservation,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<bool> {
        let Some(regions) = observation.0.detail else {
            return Ok(false);
        };
        Ok(self
            .synthesis_detail_recheck(frame, regions, cancel, timeout)?
            .view
            == V::BatteryDetail { recipe: true })
    }
    pub fn load_ether_synthesis_entry_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换合成入口资源");
        self.release_inputs()?;
        let root = validate_ether_synthesis_entry_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("合成入口路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "合成入口资源载入失败"
        );
        self.ether_synthesis_entry_bundle = Some(root);
        Ok(())
    }
    pub fn require_ether_synthesis_entry(&self) -> Result<()> {
        validate_ether_synthesis_entry_bundle(
            self.ether_synthesis_entry_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    fn synthesis_entry_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
        mode: ReadMode,
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
                pipeline: od_core::ether_synthesis_entry_pipeline::pipeline(),
            };
            if mode == ReadMode::Page {
                let page = self.inspect_ether_synthesis(
                    frame,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                )?;
                return Ok(Sight {
                    view: match page {
                        Some(SynthesisView::Page { stock, .. }) => V::Page { stock },
                        Some(SynthesisView::Unknown) | None => V::Unknown,
                        _ => V::Blocked,
                    },
                    target: None,
                    detail: None,
                });
            }
            if mode == ReadMode::Handbook {
                // 使用已登记的快捷手册弹窗区域，先拒绝已知覆盖层，避免整页文字扫描。
                let modal = words(&r.read("SynthesisEntry.Full", Some([570, 290, 740, 520]))?)?;
                if modal_blocked(&modal) || modal.iter().any(|w| w.text == "以太电池") {
                    return Ok(Sight {
                        view: V::Blocked,
                        target: None,
                        detail: None,
                    });
                }
                return self.synthesis_handbook_sight(
                    frame,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                );
            }
            let all = words(&r.read("SynthesisEntry.Full", None)?)?;
            if all.iter().any(|w| {
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
                return Ok(Sight {
                    view: V::Blocked,
                    target: None,
                    detail: None,
                });
            }
            if point(&all, "道具处理", [178, 18, 330, 86]).is_some() {
                let page = self.inspect_ether_synthesis(
                    frame,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                )?;
                return Ok(Sight {
                    view: match page {
                        Some(SynthesisView::Page { stock, .. }) => V::Page { stock },
                        Some(SynthesisView::Unknown) | None => V::Unknown,
                        _ => V::Blocked,
                    },
                    target: None,
                    detail: None,
                });
            }
            let item = all.iter().filter(|w| w.text == "以太电池").count();
            if item > 0 {
                let choices: Vec<_> = all
                    .iter()
                    .filter(|w| entry_text(&w.text) == "获取合成")
                    .collect();
                let target = (item == 1 && choices.len() == 1).then(|| choices[0].center());
                let regions = target.and_then(|_| {
                    all.iter()
                        .find(|w| w.text == "以太电池")
                        .map(|item| (item.rect, choices[0].rect))
                });
                return Ok(Sight {
                    view: V::BatteryDetail {
                        recipe: target.is_some(),
                    },
                    target,
                    detail: regions,
                });
            }
            if let Some(TransportView::Handbook { .. }) = self.inspect_compendium(
                frame,
                &TransportRequest {
                    mission_type: "基础材料".into(),
                    timeout_ms: 30_000,
                },
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )? {
                let battery = r.read("SynthesisEntry.Battery", None)?;
                let target = if battery["hit"] == true {
                    let [x, y, w, h]: [i32; 4] =
                        serde_json::from_value(battery["detail"]["best"]["box"].clone())?;
                    ensure!(
                        x >= 1220 && y >= 35 && w > 0 && h > 0 && x + w <= 1770 && y + h <= 110,
                        "电池模板超出资源栏"
                    );
                    Some([x + w / 2, y + h / 2])
                } else {
                    None
                };
                return Ok(Sight {
                    view: V::Handbook {
                        battery: target.is_some(),
                    },
                    target,
                    detail: None,
                });
            }
            Ok(Sight {
                view: V::Unknown,
                target: None,
                detail: None,
            })
        })();
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
    pub fn inspect_ether_synthesis_entry(
        &self,
        image: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        self.require_ether_synthesis_entry()?;
        Ok(self
            .synthesis_entry_sight(image, cancel, timeout, ReadMode::Inspect)?
            .map(|s| s.view))
    }
    fn synthesis_handbook_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Sight> {
        let started = Instant::now();
        let r = Reader {
            session: self,
            frame,
            cancel,
            started,
            timeout,
            pipeline: od_core::ether_synthesis_entry_pipeline::pipeline(),
        };
        if !matches!(
            self.inspect_compendium(
                frame,
                &TransportRequest {
                    mission_type: "基础材料".into(),
                    timeout_ms: 30_000
                },
                cancel,
                timeout.saturating_sub(started.elapsed())
            )?,
            Some(TransportView::Handbook { .. })
        ) {
            return Ok(Sight {
                view: V::Unknown,
                target: None,
                detail: None,
            });
        }
        let battery = r.read("SynthesisEntry.Battery", None)?;
        let target = if battery["hit"] == true {
            let [x, y, w, h]: [i32; 4] =
                serde_json::from_value(battery["detail"]["best"]["box"].clone())?;
            ensure!(
                x >= 1220 && y >= 35 && w > 0 && h > 0 && x + w <= 1770 && y + h <= 110,
                "电池模板超出资源栏"
            );
            Some([x + w / 2, y + h / 2])
        } else {
            None
        };
        Ok(Sight {
            view: V::Handbook {
                battery: target.is_some(),
            },
            target,
            detail: None,
        })
    }
    fn synthesis_detail_recheck(
        &self,
        frame: &RecognitionImage,
        regions: ([i32; 4], [i32; 4]),
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Sight> {
        let started = Instant::now();
        let r = Reader {
            session: self,
            frame,
            cancel,
            started,
            timeout,
            pipeline: od_core::ether_synthesis_entry_pipeline::pipeline(),
        };
        let item = words(&r.read("SynthesisEntry.Full", Some(expanded(regions.0)?))?)?;
        let recipe = words(&r.read("SynthesisEntry.Full", Some(expanded(regions.1)?))?)?;
        let modal = words(&r.read("SynthesisEntry.Full", Some([570, 290, 740, 520]))?)?;
        let choices: Vec<_> = recipe
            .iter()
            .filter(|w| entry_text(&w.text) == "获取合成")
            .collect();
        let target = (!modal_blocked(&modal)
            && item.iter().filter(|w| w.text == "以太电池").count() == 1
            && choices.len() == 1)
            .then(|| choices[0].center());
        Ok(Sight {
            view: if modal_blocked(&modal) {
                V::Blocked
            } else {
                V::BatteryDetail {
                    recipe: target.is_some(),
                }
            },
            target,
            detail: None,
        })
    }
    pub fn read_ether_synthesis_entry(
        &self,
        flow: &SynthesisEntryFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<SynthesisEntryObservation>> {
        let started = Instant::now();
        self.require_ether_synthesis_entry()?;
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let mode = if flow.awaiting_page() {
            ReadMode::Page
        } else if flow.opening_started() {
            ReadMode::Detail
        } else {
            ReadMode::Handbook
        };
        let sight = self.synthesis_entry_sight(
            &frame.image,
            cancel,
            timeout.saturating_sub(started.elapsed()),
            mode,
        )?;
        if cancel.load(Ordering::Acquire)
            || started.elapsed() >= timeout
            || (frame.age() > Duration::from_millis(1500)
                && !(mode == ReadMode::Detail
                    && sight.as_ref().is_some_and(|s| s.detail.is_some())))
        {
            return Ok(None);
        }
        Ok(sight.map(SynthesisEntryObservation))
    }
    pub fn act_ether_synthesis_entry(
        &mut self,
        flow: &SynthesisEntryFlow,
        action: A,
        observation: Option<&SynthesisEntryObservation>,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<SynthesisEntryInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<_> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_ether_synthesis_entry()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(SynthesisEntryInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(SynthesisEntryInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(SynthesisEntryInputEnd::NotPosted);
            };
            let sight = match action {
                A::OpenBattery => self.synthesis_entry_sight(
                    &frame.image,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                    ReadMode::Handbook,
                ),
                A::SelectRecipe => match observation.and_then(|o| o.0.detail) {
                    Some(regions) => self
                        .synthesis_detail_recheck(
                            &frame.image,
                            regions,
                            cancel,
                            timeout.saturating_sub(started.elapsed()),
                        )
                        .map(Some),
                    None => Ok(None),
                },
            };
            if cancel.load(Ordering::Acquire) {
                return Ok(SynthesisEntryInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(SynthesisEntryInputEnd::TimedOut);
            }
            let Some(sight) = sight? else {
                return Ok(SynthesisEntryInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, sight.view) {
                return Ok(SynthesisEntryInputEnd::Changed);
            }
            let [x, y] = sight.target.context("合成入口缺少同帧目标")?;
            if cancel.load(Ordering::Acquire) {
                return Ok(SynthesisEntryInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(SynthesisEntryInputEnd::TimedOut);
            }
            let input = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(SynthesisEntryInputEnd::NotPosted);
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
            Ok(SynthesisEntryInputEnd::Posted)
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}
pub fn validate_ether_synthesis_entry_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("以太电池合成入口资源缺失")?;
    let path = root.join("ether_synthesis_entry-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "以太电池合成入口清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sources = match manifest["schema_version"].as_u64() {
        Some(1) => include_str!("../../../resources/ether-synthesis-entry/source-lock.json"),
        _ => anyhow::bail!("以太电池合成入口资源版本不兼容"),
    };
    ensure!(
        manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["ether_synthesis_entry"])
            && manifest["sources"] == serde_json::from_str::<Value>(sources)?,
        "以太电池合成入口资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let battery = od_core::ether_synthesis_entry_pipeline::battery_template();
    expected.insert(
        "image/ether-synthesis-entry/battery.png".into(),
        (battery.len() as u64, hex::encode(Sha256::digest(battery))),
    );
    let pipeline = serde_json::to_vec_pretty(&od_core::ether_synthesis_entry_pipeline::pipeline())?;
    expected.insert(
        "pipeline/ether_synthesis_entry.json".into(),
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
        .context("以太电池合成入口文件清单无效")?;
    ensure!(
        files.len() == expected.len(),
        "以太电池合成入口文件数量不匹配"
    );
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"]
            .as_str()
            .context("以太电池合成入口文件路径无效")?;
        let (size, hash) = expected
            .get(name)
            .context("以太电池合成入口资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "以太电池合成入口文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "以太电池合成入口文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "以太电池合成入口文件内容不匹配"
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
                name == "ether_synthesis_entry-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "以太电池合成入口资源含额外文件或目录"
        );
    }
    Ok(root)
}
