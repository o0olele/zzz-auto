use anyhow::{Context, Result, ensure};
use od_core::suibian_temple_auto::TempleHostingProgress;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

pub fn validate_suibian_temple_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("随便观资源缺失")?;
    let manifest_path = root.join("suibian-temple-manifest.json");
    od_core::config::reject_links(&manifest_path)?;
    ensure!(
        std::fs::metadata(&manifest_path)?.len() <= 1024 * 1024,
        "随便观清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
    ensure!(
        matches!(manifest["schema_version"].as_u64(), Some(1..=8))
            && manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"]
                == if manifest["schema_version"] == 1 {
                    json!(["auto_manage"])
                } else if manifest["schema_version"] == 2 {
                    json!(["auto_manage", "adventure"])
                } else if manifest["schema_version"] == 3 {
                    json!(["auto_manage", "adventure", "craft"])
                } else if manifest["schema_version"] == 4 {
                    json!(["auto_manage", "adventure", "craft", "yum_cha_sin"])
                } else if manifest["schema_version"] == 5 {
                    json!(["auto_manage", "adventure", "craft", "yum_cha_sin", "sales"])
                } else if manifest["schema_version"] == 6 {
                    json!([
                        "auto_manage",
                        "adventure",
                        "craft",
                        "yum_cha_sin",
                        "sales",
                        "good_goods"
                    ])
                } else if manifest["schema_version"] == 7 {
                    json!([
                        "auto_manage",
                        "adventure",
                        "craft",
                        "yum_cha_sin",
                        "sales",
                        "good_goods",
                        "boo_box"
                    ])
                } else {
                    json!([
                        "auto_manage",
                        "adventure",
                        "craft",
                        "yum_cha_sin",
                        "sales",
                        "good_goods",
                        "boo_box",
                        "full_run"
                    ])
                }
            && manifest["sources"]
                == serde_json::from_str::<Value>(include_str!(
                    "../../../resources/suibian-temple/source-lock.json"
                ))?,
        "随便观资源版本或能力不兼容"
    );
    let pipeline = if manifest["schema_version"] == 1 {
        od_core::suibian_temple_auto::pipeline()
    } else {
        ensure!(
            manifest["adventure_sources"]
                == serde_json::from_str::<Value>(include_str!(
                    "../../../resources/suibian-temple/adventure-source-lock.json"
                ))?
                && manifest["image_sources"]
                    == serde_json::from_str::<Value>(include_str!(
                        "../../../resources/suibian-temple/adventure-image-sources.json"
                    ))?,
            "游历来源版本不匹配"
        );
        if manifest["schema_version"] == 2 {
            od_core::suibian_adventure_pipeline::pipeline()
        } else {
            ensure!(
                manifest["craft_sources"]
                    == serde_json::from_str::<Value>(include_str!(
                        "../../../resources/suibian-temple/craft-source-lock.json"
                    ))?,
                "制造来源版本不匹配"
            );
            if manifest["schema_version"] == 3 {
                od_core::suibian_craft_pipeline::pipeline()
            } else {
                ensure!(
                    manifest["yum_sources"]
                        == serde_json::from_str::<Value>(include_str!(
                            "../../../resources/suibian-temple/yum-source-lock.json"
                        ))?,
                    "饮茶仙来源版本不匹配"
                );
                if manifest["schema_version"] == 4 {
                    od_core::suibian_yum_pipeline::pipeline()
                } else {
                    ensure!(
                        manifest["sales_sources"]
                            == serde_json::from_str::<Value>(include_str!(
                                "../../../resources/suibian-temple/sales-source-lock.json"
                            ))?,
                        "售卖来源版本不匹配"
                    );
                    if manifest["schema_version"] == 5 {
                        od_core::suibian_sales_pipeline::pipeline()
                    } else {
                        ensure!(
                            manifest["good_goods_sources"]
                                == serde_json::from_str::<Value>(include_str!(
                                    "../../../resources/suibian-temple/good-goods-source-lock.json"
                                ))?,
                            "好物铺来源版本不匹配"
                        );
                        if manifest["schema_version"] == 6 {
                            od_core::suibian_good_goods_pipeline::pipeline()
                        } else {
                            ensure!(
                                manifest["boo_box_sources"]
                                    == serde_json::from_str::<Value>(include_str!(
                                        "../../../resources/suibian-temple/boo-box-source-lock.json"
                                    ))?,
                                "邦巢来源版本不匹配"
                            );
                            if manifest["schema_version"] == 7 {
                                od_core::suibian_boo_box_pipeline::pipeline()
                            } else {
                                od_core::suibian_entry_pipeline::pipeline()
                            }
                        }
                    }
                }
            }
        }
    };
    if manifest["schema_version"] == 8 {
        ensure!(
            manifest["entry_sources"]
                == serde_json::from_str::<Value>(include_str!(
                    "../../../resources/suibian-temple/entry-source-lock.json"
                ))?,
            "随便观入口来源不匹配"
        );
    }
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut pinned = BTreeMap::<String, Vec<u8>>::new();
    let back = od_core::basic_daily::templates()
        .into_iter()
        .find(|(name, _)| *name == "back")
        .context("缺少固定返回模板")?
        .1;
    pinned.insert("image/suibian-temple/back.png".into(), back.to_vec());
    if manifest["schema_version"].as_u64().is_some_and(|v| v >= 2) {
        for (name, bytes) in od_core::suibian_adventure_pipeline::templates() {
            pinned.insert(name.into(), bytes.to_vec());
        }
    }
    if manifest["schema_version"] == 8 {
        for (name, bytes) in od_core::suibian_entry_pipeline::templates() {
            pinned.insert(format!("image/suibian-temple/{name}.png"), bytes.to_vec());
        }
    }
    let mut allowed: BTreeSet<String> = pinned.keys().cloned().collect();
    allowed.insert("pipeline/suibian-temple.json".into());
    for file in ocr["files"].as_array().context("OCR 清单无效")? {
        if let Some(path) = file["bundle_path"].as_str() {
            allowed.insert(path.into());
        }
    }
    let mut seen = BTreeSet::new();
    for file in manifest["files"].as_array().context("随便观文件清单无效")? {
        let name = file["path"].as_str().context("随便观路径无效")?;
        ensure!(
            allowed.contains(name) && seen.insert(name.to_owned()),
            "随便观文件未登记或重复"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            path.canonicalize()?.starts_with(&root),
            "随便观资源指向包外"
        );
        let metadata = std::fs::metadata(&path)?;
        let limit = if name.starts_with("model/") {
            128 * 1024 * 1024
        } else {
            1024 * 1024
        };
        ensure!(
            metadata.is_file() && metadata.len() <= limit && file["bytes"] == metadata.len(),
            "随便观文件长度无效"
        );
        let bytes = std::fs::read(&path)?;
        let hash = hex::encode(Sha256::digest(&bytes));
        ensure!(file["sha256"] == hash, "随便观文件哈希不匹配");
        if let Some(expected) = pinned.remove(name) {
            ensure!(bytes == expected, "随便观模板与固定资源不匹配");
        } else if name.starts_with("model/") {
            let model = ocr["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["bundle_path"] == name)
                .context("随便观模型未锁定")?;
            ensure!(model["sha256"] == hash, "随便观模型或字典不匹配");
        } else {
            ensure!(
                serde_json::from_slice::<Value>(&bytes)? == pipeline,
                "随便观执行节点与引擎不匹配"
            );
        }
    }
    ensure!(seen == allowed, "随便观资源不完整");
    for entry in walkdir::WalkDir::new(&root).min_depth(1) {
        let entry = entry?;
        od_core::config::reject_links(entry.path())?;
        let relative = entry
            .path()
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        ensure!(
            if entry.file_type().is_file() {
                relative == "suibian-temple-manifest.json" || allowed.contains(&relative)
            } else {
                allowed
                    .iter()
                    .any(|name| name.starts_with(&format!("{relative}/")))
            },
            "随便观资源含额外文件或目录"
        );
    }
    Ok(root)
}

impl crate::NativeSession {
    pub fn load_suibian_temple_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换随便观资源");
        self.release_inputs()?;
        let root = validate_suibian_temple_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("随便观路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "随便观资源载入失败"
        );
        self.suibian_temple_bundle = Some(root);
        Ok(())
    }
    pub fn suibian_temple_hosting_step(
        &mut self,
        progress: &TempleHostingProgress,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
        game: bool,
    ) -> Result<crate::PipelineExecution> {
        progress.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "随便观托管控制器模式不匹配"
        );
        validate_suibian_temple_bundle(
            self.suibian_temple_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        // before_step 必须由引擎在投递前持久保存，不能在适配层悄悄补齐。
        let ready = match progress.stage.as_str() {
            "Stop" => progress.stop_attempted,
            "ConfirmStop" => progress.stop_confirm_attempted,
            "Claim" => progress.claim_attempted,
            "ConfirmReward" => progress.reward_confirm_attempted,
            "Start" => progress.start_attempted,
            "ConfirmStart" => progress.start_confirm_attempted,
            _ => true,
        };
        ensure!(ready, "随便观托管输入前缺少已保存投递意图");
        ensure!(
            od_core::suibian_temple_auto::pipeline()
                .get(progress.entry())
                .is_some(),
            "随便观托管步骤未登记"
        );
        if game {
            self.prepare_input_target(
                od_core::mail::MailConfig::recovery_inputs()[1],
                &cancel,
                &mut None,
            )?;
        }
        self.run_pipeline_detailed(
            &progress.entry(),
            &crate::PipelineInputPlan {
                keys: BTreeSet::new(),
                contacts: [0].into(),
            },
            cancel,
            timeout,
            "{}",
            true,
        )
    }
}

/// 完整入口要求最新的固定导航；旧子流程包仍可独立加载。
pub fn validate_full_temple_bundle(root: &Path) -> Result<PathBuf> {
    let root = validate_suibian_temple_bundle(root)?;
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("suibian-temple-manifest.json"))?)?;
    ensure!(
        manifest["schema_version"] == 8,
        "完整随便观运行需要 schema 8 资源"
    );
    Ok(root)
}
impl crate::NativeSession {
    pub fn temple_navigation_step(
        &mut self,
        stage: &str,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
        game: bool,
    ) -> Result<crate::PipelineExecution> {
        ensure!(
            matches!(
                stage,
                "Inspect"
                    | "OpenMap"
                    | "SelectArea"
                    | "SelectPoint"
                    | "ConfirmTransport"
                    | "WaitArrival"
                    | "Return"
            ),
            "随便观导航步骤未登记"
        );
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "随便观控制器模式不符"
        );
        validate_full_temple_bundle(
            self.suibian_temple_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        if game {
            self.prepare_input_target(
                od_core::mail::MailConfig::recovery_inputs()[1],
                &cancel,
                &mut None,
            )?;
        }
        self.run_pipeline_detailed(
            &format!("Temple.{stage}"),
            &crate::PipelineInputPlan {
                keys: [27, 78].into(),
                contacts: [0].into(),
            },
            cancel,
            timeout,
            "{}",
            true,
        )
    }
}
