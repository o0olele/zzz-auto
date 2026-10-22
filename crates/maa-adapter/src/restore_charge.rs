//! 恢复弹窗的同帧识别与输入复验；父副本负责正向识别返回画面。
pub use crate::suibian_adventure::AdventureInputEnd as RestoreInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::restore_charge::{
    RestoreAction as A, RestoreFlow, RestoreReturn, RestoreSource as S, RestoreView as V,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub fn validate_restore_charge_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("恢复电量资源缺失")?;
    let path = root.join("restore-charge-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "恢复清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        manifest["schema_version"] == 1
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["restore_charge"])
            && manifest["sources"]
                == serde_json::from_str::<Value>(include_str!(
                    "../../../resources/restore-charge/source-lock.json"
                ))?,
        "恢复资源版本或来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let pipeline = serde_json::to_vec_pretty(&od_core::restore_charge_pipeline::pipeline())?;
    expected.insert(
        "pipeline/restore-charge.json".into(),
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
    let files = manifest["files"].as_array().context("恢复文件清单无效")?;
    ensure!(files.len() == expected.len(), "恢复文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("恢复文件路径无效")?;
        let (size, hash) = expected.get(name).context("恢复资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "恢复文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "恢复文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "恢复文件内容不匹配"
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
                name == "restore-charge-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "恢复资源含额外文件或目录"
        );
    }
    Ok(root)
}
struct Sight {
    view: V,
    confirm: Option<[i32; 2]>,
    cancel: Option<[i32; 2]>,
}
fn number(s: &str) -> Option<u32> {
    (!s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
        .filter(|n| *n <= 1_000_000)
}
fn single_number(r: &Reader<'_>, name: &str) -> Result<Option<u32>> {
    let all = words(&r.read(name, None)?)?;
    Ok(if all.len() == 1 {
        number(&all[0].text)
    } else {
        None
    })
}
impl NativeSession {
    pub fn load_restore_charge_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换恢复资源");
        self.release_inputs()?;
        let root = validate_restore_charge_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("恢复路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "恢复资源载入失败"
        );
        self.restore_charge_bundle = Some(root);
        Ok(())
    }
    pub fn require_restore_charge(&self) -> Result<()> {
        validate_restore_charge_bundle(
            self.restore_charge_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    fn restore_sight(
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
                pipeline: od_core::restore_charge_pipeline::pipeline(),
            };
            let titles = words(&r.read("Restore.Area.Titles", None)?)?;
            let main = point(&titles, "恢复电量", [824, 348, 1076, 390]).is_some();
            let quick = point(&titles, "快捷使用", [584, 300, 727, 353]).is_some();
            let reward = point(&titles, "获得", [860, 386, 1060, 456]).is_some();
            if !main && !quick && !reward {
                return Ok(Sight {
                    view: V::Unknown,
                    confirm: None,
                    cancel: None,
                });
            }
            let buttons = words(&r.read("Restore.Area.Buttons", None)?)?;
            let confirm = point(&buttons, "确认", [720, 700, 1200, 805]);
            let cancel_point = point(&buttons, "取消", [720, 700, 1200, 805]);
            let mut view = V::UnreadablePopup;
            if u8::from(main) + u8::from(quick) + u8::from(reward) == 1 && confirm.is_some() {
                if main {
                    let t = words(&r.read("Restore.Area.Sources", None)?)?;
                    let counts = |source: S, x: i32| -> Option<u32> {
                        let label = t
                            .iter()
                            .filter(|w| w.inside([x, 605, x + 140, 640]) && w.text == source.name())
                            .count()
                            == 1;
                        let nums: Vec<_> = t
                            .iter()
                            .filter(|w| w.inside([x, 575, x + 140, 605]))
                            .collect();
                        if label && nums.len() == 1 {
                            number(&nums[0].text)
                        } else {
                            None
                        }
                    };
                    if let (Some(backup), Some(ether)) =
                        (counts(S::Backup, 750), counts(S::Ether, 890))
                    {
                        let a = r.hit("Restore.Area.BackupSelected", None)?;
                        let b = r.hit("Restore.Area.EtherSelected", None)?;
                        if !(a && b) && cancel_point.is_some() {
                            view = V::Sources {
                                backup,
                                ether,
                                selected: if a {
                                    Some(S::Backup)
                                } else if b {
                                    Some(S::Ether)
                                } else {
                                    None
                                },
                            };
                        }
                    }
                } else if quick {
                    let t = words(&r.read("Restore.Area.QuickSource", None)?)?;
                    if t.len() == 1 {
                        for source in [S::Backup, S::Ether] {
                            if let Some(owned) = t[0]
                                .text
                                .strip_prefix(source.name())
                                .and_then(|s| s.strip_prefix(['×', 'x', 'X']))
                                .and_then(number)
                                && let Some(quantity) = single_number(&r, "Restore.Area.Quantity")?
                            {
                                view = V::QuickUse {
                                    source,
                                    owned,
                                    quantity,
                                };
                            }
                        }
                    }
                } else {
                    let t = words(&r.read("Restore.Area.Reward", None)?)?;
                    if point(&t, "电量", [890, 610, 1030, 650]).is_some() {
                        let nums: Vec<_> = t
                            .iter()
                            .filter(|w| w.inside([890, 575, 1030, 610]))
                            .collect();
                        if nums.len() == 1
                            && let Some(charge) = number(&nums[0].text)
                        {
                            view = V::Reward { charge };
                        }
                    }
                }
            }
            Ok(Sight {
                view,
                confirm,
                cancel: cancel_point,
            })
        })();
        if cancel.load(Ordering::Acquire) {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
    /// 仅检查实际弹窗，适合离线核对；不消费任何输入。
    pub fn inspect_restore_charge(
        &self,
        image: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        self.require_restore_charge()?;
        Ok(self.restore_sight(image, cancel, timeout)?.map(|s| s.view))
    }
    /// 父读取器必须正向识别自身无遮挡的返回画面；只允许消费传入的同一帧。
    pub fn read_restore_charge(
        &self,
        cancel: &AtomicBool,
        timeout: Duration,
        parent: &mut impl FnMut(
            &NativeSession,
            &RecognitionImage,
            &AtomicBool,
            Duration,
        ) -> Result<Option<RestoreReturn>>,
    ) -> Result<Option<V>> {
        let started = Instant::now();
        self.require_restore_charge()?;
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        ensure!(started.elapsed() < timeout, "恢复识别超时");
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let result = (|| -> Result<Option<V>> {
            let Some(mut sight) = self.restore_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            else {
                return Ok(None);
            };
            if sight.view == V::Unknown
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
                && let Some(proof) = parent(
                    self,
                    &frame.image,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                )?
            {
                sight.view = V::Returned(proof);
            }
            ensure!(started.elapsed() < timeout, "恢复识别超时");
            Ok((frame.age() <= Duration::from_millis(1500)).then_some(sight.view))
        })();
        if cancel.load(Ordering::Acquire) {
            Ok(None)
        } else {
            result
        }
    }
    pub fn act_restore_charge(
        &mut self,
        flow: &RestoreFlow,
        action: &A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<RestoreInputEnd> {
        let started = Instant::now();
        flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "恢复控制器模式不匹配"
        );
        let result = (|| -> Result<RestoreInputEnd> {
            self.require_restore_charge()?;
            if cancel.load(Ordering::Acquire) {
                return Ok(RestoreInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(RestoreInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(RestoreInputEnd::NotPosted);
            };
            let sight = self.restore_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            );
            if cancel.load(Ordering::Acquire) {
                return Ok(RestoreInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(RestoreInputEnd::TimedOut);
            }
            let Some(sight) = sight? else {
                return Ok(RestoreInputEnd::NotPosted);
            };
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &sight.view) {
                return Ok(RestoreInputEnd::Changed);
            }
            let p = match action {
                A::Select(S::Backup) => Some([820, 510]),
                A::Select(S::Ether) => Some([960, 510]),
                A::OpenQuick(_) | A::Submit(_) | A::ConfirmReward => sight.confirm,
                A::CancelSources => sight.cancel,
                A::CloseQuick => Some([1307, 320]),
            };
            let Some([x, y]) = p else {
                return Ok(RestoreInputEnd::Changed);
            };
            if started.elapsed() >= timeout {
                return Ok(RestoreInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) {
                return Ok(RestoreInputEnd::Changed);
            }
            let input = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(RestoreInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(70)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input, false, &AtomicBool::new(false))?;
            Ok(RestoreInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|v| release.map(|_| v))
    }
}
