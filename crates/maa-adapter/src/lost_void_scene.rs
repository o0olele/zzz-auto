//! 迷失之地同帧场景观察。动态点击位置只在本帧有效。
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, Word, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::lost_void_scene::{
    LostVoidObservation as O, LostVoidScreen as S, parse_bounty, parse_choice_title,
    parse_selection_count,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(crate) struct LostVoidSight {
    pub observation: O,
    pub points: BTreeMap<String, [i32; 2]>,
    pub event: Option<od_core::lost_void_event::EventPage>,
}
fn named(words: &[Word], text: &str) -> bool {
    words.iter().any(|w| w.text == text)
}
fn contains(words: &[Word], text: &str) -> bool {
    words.iter().any(|w| w.text.contains(text))
}
fn progress(words: &[Word]) -> Option<od_core::lost_void_scene::BountyProgress> {
    let mut words = words.to_vec();
    words.sort_by_key(|w| (w.center()[1] / 20, w.center()[0]));
    parse_bounty(&words.iter().map(|w| w.text.clone()).collect::<Vec<_>>())
}
impl NativeSession {
    pub fn load_lost_void_scene_bundle(&mut self, root: &Path) -> Result<()> {
        ensure!(!self.tasker.running(), "执行中不能替换迷失之地资源");
        self.release_inputs()?;
        let root = validate_lost_void_scene_bundle(root)?;
        ensure!(
            self._resource
                .post_bundle(root.to_str().context("迷失之地资源路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "迷失之地资源载入失败"
        );
        self.lost_void_scene_bundle = Some(root);
        Ok(())
    }
    pub fn require_lost_void_scene(&self) -> Result<()> {
        validate_lost_void_scene_bundle(
            self.lost_void_scene_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    pub fn inspect_lost_void_scene(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<O>> {
        self.require_lost_void_scene()?;
        Ok(self
            .lost_void_sight(frame, cancel, timeout)?
            .map(|s| s.observation))
    }
    pub(crate) fn lost_void_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<LostVoidSight>> {
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let started = Instant::now();
        let result = (|| -> Result<LostVoidSight> {
            let r = Reader {
                session: self,
                frame,
                cancel,
                started,
                timeout,
                pipeline: od_core::lost_void_scene::pipeline(),
            };
            let read = |n: &str| words(&r.read(&format!("LostVoid.{n}"), None)?);
            let hit = |n: &str| r.hit(&format!("LostVoid.{n}"), None);
            let mut s = LostVoidSight {
                observation: O::default(),
                points: BTreeMap::new(),
                event: None,
            };
            let modal = if named(&read("ModalConfirm")?, "确认") {
                read("Modal")?
            } else {
                vec![]
            };
            let confirm = point(&modal, "确认", [1000, 580, 1260, 720]);
            if let Some(position) = confirm
                && modal
                    .iter()
                    .any(|w| w.text.trim_end_matches(['?', '？', '。']) == "是否立即传送")
            {
                s.observation.screen = S::TransportConfirm;
                s.points.insert("confirm".into(), position);
                return Ok(s);
            }
            // 弹窗优先。底层菜单和标题不能为未知确认框提供操作依据。
            if let Some(position) = confirm
                && named(&modal, "不再提示")
                && modal
                    .iter()
                    .any(|w| w.text.starts_with("选择初始战术棱镜方案"))
            {
                s.observation.screen = S::GearConfirm;
                s.points.insert("confirm".into(), position);
                return Ok(s);
            }
            if confirm.is_some() && named(&modal, "取消") {
                s.observation.screen = S::Modal;
                return Ok(s);
            }
            let world_hud = hit("WorldTab")? && hit("BattleMenu")?;
            if world_hud {
                if let Some(event) = crate::lost_void_event::event_regions(&r)? {
                    s.observation.screen = if event.page.is_some() {
                        S::Event
                    } else {
                        S::Unknown
                    };
                    s.event = event.page;
                    s.points.extend(event.points);
                    return Ok(s);
                }
                s.observation.screen = S::World;
                return Ok(s);
            }
            let common = read("CommonTitle")?;
            let kinds: Vec<_> = common
                .iter()
                .filter_map(|w| parse_choice_title(&w.text).map(|kind| (kind, w.text.clone())))
                .collect();
            if (kinds.len() == 1 || common.is_empty()) && hit("WorldTab")? {
                let confirms = read("CommonConfirm")?;
                let buttons: Vec<_> = confirms
                    .iter()
                    .filter(|w| w.text == "确定" || parse_selection_count(&w.text).is_some())
                    .collect();
                let titleless = common.is_empty()
                    && buttons.len() == 1
                    && buttons[0].text == "确定"
                    && named(&read("CommonEnabled")?, "确定");
                if buttons.len() == 1 && (kinds.len() == 1 || titleless) {
                    s.observation.screen = S::CommonChoice;
                    s.observation.choice = kinds
                        .first()
                        .map(|(kind, _)| *kind)
                        .or(Some(od_core::lost_void_scene::ChoiceKind::Gain));
                    s.observation.title = kinds.first().map(|(_, title)| title.clone());
                    s.observation.confirm = true;
                    s.observation.selection_count = parse_selection_count(&buttons[0].text);
                    s.points.insert("confirm".into(), buttons[0].center());
                    return Ok(s);
                }
            }
            let gear_carry = read("GearCarry")?;
            if named(&gear_carry, "携带") && named(&read("GearHeader")?, "详情") {
                s.observation.screen = S::Gear;
                if let Some(p) = point(&gear_carry, "携带", [1732, 846, 1912, 896]) {
                    s.points.insert("carry".into(), p);
                }
                return Ok(s);
            }
            if hit("WorldFunction")? && hit("WorldPreset")? {
                s.observation.screen = S::NormalWorld;
                let icon = r.read("LostVoid.HandbookIcon", None)?;
                if icon["hit"] == true {
                    let rect: [i32; 4] =
                        serde_json::from_value(icon["detail"]["best"]["box"].clone())?;
                    if od_core::intel_board::valid_rect(rect) {
                        let [x, y, w, h] = rect;
                        s.points.insert("open".into(), [x + w / 2, y + h / 2]);
                    }
                }
                return Ok(s);
            }
            if named(&read("StoreRefresh")?, "刷新")
                && named(&read("GearHeader")?, "详情")
                && hit("Back")?
            {
                s.observation.screen = S::Store;
                return Ok(s);
            }
            if let Some(event) = crate::lost_void_event::event_regions(&r)?
                && event.page.is_some()
            {
                s.observation.screen = S::Event;
                s.event = event.page;
                s.points.extend(event.points);
                return Ok(s);
            }
            let tabs = read("Tabs")?;
            if ["目标", "日常", "训练", "作战", "战术"]
                .iter()
                .filter(|t| named(&tabs, t))
                .count()
                >= 4
            {
                s.observation.screen = S::Handbook;
                if let Some(p) = point(&tabs, "作战", [708, 140, 1636, 220]) {
                    s.points.insert("combat".into(), p);
                }
                let selected = read("SelectedTab")?;
                let combat = selected.len() == 1 && named(&selected, "作战");
                let categories = if combat { read("Categories")? } else { vec![] };
                let mut selected_category = None;
                let mut visible = Vec::new();
                for name in ["周期征讨", "零号空洞"] {
                    let choices: Vec<_> = categories.iter().filter(|w| w.text == name).collect();
                    if choices.len() == 1 {
                        let w = choices[0];
                        if !w.inside([164, 287, 429, 870]) {
                            continue;
                        }
                        visible.push(name.to_string());
                        s.points.insert(format!("category:{name}"), w.center());
                        let [x, y, w, h] = w.rect;
                        let left = (x - 12).max(164);
                        let top = (y - 10).max(287);
                        let width = (x + w + 12).min(429) - left;
                        let height = (y + h + 10).min(870) - top;
                        let mut params = r.pipeline["LostVoid.CategoryColor"].clone();
                        params.as_object_mut().unwrap().remove("recognition");
                        params["roi"] = json!([left, top, width, height]);
                        params["count"] = json!((width * height / 5).max(1));
                        if self
                            .recognize_prepared(frame, "ColorMatch", &params)?
                            .detail["hit"]
                            == true
                        {
                            ensure!(selected_category.is_none(), "迷失之地手册分类选中态冲突");
                            selected_category = Some(name.to_string());
                        }
                    }
                }
                let mut types = if selected_category.is_some() {
                    read("Types")?
                } else {
                    vec![]
                };
                types.retain(|w| {
                    od_core::lost_void_scene::HANDBOOK_MISSIONS.contains(&w.text.as_str())
                });
                let go = if selected_category.is_some() {
                    read("Go")?
                } else {
                    vec![]
                };
                let targets: Vec<_> = types.iter().filter(|w| w.text == "迷失之地").collect();
                let paired = if targets.len() == 1 {
                    let y = targets[0].center()[1];
                    let bottom = types
                        .iter()
                        .map(|w| w.center()[1])
                        .filter(|other| *other > y)
                        .min()
                        .unwrap_or(890)
                        .min(y + 180);
                    let buttons: Vec<_> = go
                        .iter()
                        .filter(|w| w.text == "前往" && w.center()[1] > y && w.center()[1] < bottom)
                        .collect();
                    (buttons.len() == 1).then(|| buttons[0].center())
                } else {
                    None
                };
                if let Some(p) = paired {
                    s.points.insert("go".into(), p);
                }
                s.observation.handbook = Some(od_core::lost_void_scene::LostVoidHandbook {
                    combat,
                    categories: visible,
                    selected_category,
                    types: types.into_iter().map(|w| w.text).collect(),
                    target_go: paired.is_some(),
                });
                return Ok(s);
            }
            let header = read("Header")?;
            let footer = read("Footer")?;
            if named(&header, "奖励预览") && hit("Back")? {
                s.observation.screen = S::Lottery;
                let mut start = read("LotteryStart")?;
                start.sort_by_key(|w| w.center()[0]);
                s.observation.lottery = od_core::lost_void_scene::parse_lottery_tickets(
                    &start.iter().map(|w| w.text.as_str()).collect::<String>(),
                );
                if s.observation.lottery.is_some() {
                    let position = start
                        .iter()
                        .find(|w| w.text.starts_with("开始"))
                        .map(Word::center);
                    if let Some(p) = position {
                        s.points.insert("start".into(), p);
                    }
                }
                return Ok(s);
            }
            if named(&footer, "图例详情") && hit("Back")? {
                s.observation.screen = S::RouteChange;
                return Ok(s);
            }
            if contains(&header, "此界面可以进行代理人编队")
                && contains(&header, "点击[屏幕任意处]继续下一步")
            {
                s.observation.screen = S::MatrixTeam;
                s.observation.tutorial = true;
                return Ok(s);
            }
            for (name, list, text, area) in [
                ("standard", &header, "常规", [1580, 10, 1790, 100]),
                ("periodic", &header, "周期", [1400, 10, 1600, 100]),
                ("next", &footer, "下一步", [1580, 980, 1850, 1065]),
                ("deploy", &footer, "出战", [1550, 970, 1850, 1065]),
                ("finish", &footer, "完成", [1500, 980, 1870, 1070]),
                (
                    "matrix_deploy",
                    &footer,
                    "开始挑战",
                    [1580, 980, 1850, 1065],
                ),
            ] {
                if let Some(p) = point(list, text, area) {
                    s.points.insert(name.into(), p);
                }
            }
            s.observation.next = s.points.contains_key("next");
            if named(&header, "周期") && named(&header, "常规") && named(&footer, "探索收集")
            {
                let cards = read("EntryCards")?;
                let standard = named(&cards, "战线肃清") && named(&cards, "特遣调查");
                let periodic = named(&cards, "矩阵行动") && named(&cards, "前往挑战");
                s.observation.screen = match (standard, periodic) {
                    (true, false) => S::EntryStandard,
                    (false, true) => S::EntryPeriodic,
                    _ => S::Unknown,
                };
                if s.observation.screen != S::Unknown {
                    s.observation.bounty = progress(&header);
                    for (name, text, area) in [
                        ("purge", "战线肃清", [878, 318, 1069, 365]),
                        ("task_force", "特遣调查", [1425, 311, 1613, 362]),
                        ("matrix", "前往挑战", [1142, 746, 1384, 809]),
                    ] {
                        if let Some(p) = point(&cards, text, area) {
                            s.points.insert(name.into(), p);
                            s.observation.missions.push(match name {
                                "purge" => od_core::lost_void::LostVoidMission::Purge,
                                "task_force" => od_core::lost_void::LostVoidMission::TaskForce,
                                _ => od_core::lost_void::LostVoidMission::MatrixAction,
                            });
                        }
                    }
                    return Ok(s);
                }
            }
            let titles = read("Title")?;
            let failure = read("FailureButtons")?;
            if named(&failure, "撤退") && named(&failure, "重播") {
                s.observation.screen = S::BattleFail;
                return Ok(s);
            }
            let result_title = read("ResultTitle")?;
            if named(&result_title, "挑战结果") || named(&read("ResultTitleLine")?, "挑战结果")
            {
                s.observation.screen = S::BattleResult;
                let confirm = read("ResultConfirm")?;
                s.observation.confirm = named(&confirm, "确定");
                s.observation.finish = s.points.contains_key("finish");
                if let Some(p) = point(&confirm, "确定", [855, 765, 1338, 997]) {
                    s.points.insert("confirm".into(), p);
                }
                return Ok(s);
            }
            if named(&header, "自由编队")
                && named(&header, "预备编队")
                && s.points.contains_key("matrix_deploy")
            {
                s.observation.screen = S::MatrixTeam;
                let tutorial = read("Tutorial")?;
                s.observation.tutorial = contains(&tutorial, "此界面可以进行代理人编队")
                    && contains(&tutorial, "继续下一步");
                return Ok(s);
            }
            if named(&header, "街区") && hit("Back")? {
                s.observation.bounty = progress(&header);
                let purge = titles.iter().find(|w| w.text.starts_with("战线肃清"));
                let matrix = titles.iter().find(|w| w.text == "矩阵行动");
                s.observation.screen = if purge.is_some() && matrix.is_none() {
                    S::Purge
                } else if matrix.is_some() && purge.is_none() {
                    S::Matrix
                } else if named(&footer, "特遣任务") {
                    S::TaskForce
                } else {
                    S::Unknown
                };
                if s.observation.screen != S::Unknown {
                    return Ok(s);
                }
            }
            if contains(&header, "详情") {
                let carry = read("GearCarry")?;
                if named(&carry, "携带") {
                    s.observation.screen = S::Gear;
                    s.observation.confirm = confirm.is_some();
                    return Ok(s);
                }
                if named(&footer, "刷新") && hit("Back")? {
                    s.observation.screen = S::Store;
                    return Ok(s);
                }
            }
            // 列表必须同时含多个目录战略和独立确认，不能把副本详情当作列表。
            let strategic = read("StrategyNames")?;
            let strategies = od_core::lost_void::strategies().map_err(anyhow::Error::msg)?;
            if strategic
                .iter()
                .filter(|w| strategies.contains(&w.text))
                .count()
                >= 2
                && (named(&footer, "确定") || named(&modal, "确定"))
            {
                s.observation.screen = S::Strategy;
                return Ok(s);
            }
            if hit("WorldTab")? && hit("BattleMenu")? {
                s.observation.screen = S::World;
                return Ok(s);
            }
            if hit("WorldFunction")? && hit("WorldPreset")? {
                s.observation.screen = S::NormalWorld;
            } else if s.points.contains_key("deploy") {
                s.observation.screen = S::Deploy;
            }
            Ok(s)
        })();
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        result.map(Some)
    }
}

pub fn validate_lost_void_scene_bundle(root: &Path) -> Result<PathBuf> {
    od_core::config::reject_links(root)?;
    let root = root.canonicalize().context("迷失之地画面资源缺失")?;
    let path = root.join("lost_void_scene-manifest.json");
    od_core::config::reject_links(&path)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "迷失之地画面清单过大"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sources = match manifest["schema_version"].as_u64() {
        Some(1) => include_str!("../../../resources/lost-void/screen-source-lock.json"),
        _ => anyhow::bail!("迷失之地画面资源版本不兼容"),
    };
    ensure!(
        manifest["sdk_version"] == crate::SDK_VERSION
            && manifest["base_size"] == json!([1920, 1080])
            && manifest["accepted"] == false
            && manifest["capabilities"] == json!(["lost_void_scene"])
            && manifest["sources"] == serde_json::from_str::<Value>(sources)?,
        "迷失之地画面资源版本或来源不兼容"
    );
    ensure!(
        manifest["local_sources"]
            == json!([
                od_core::lost_void_loading::lore_source(),
                od_core::lost_void_settlement::settlement_source(),
                od_core::lost_void_deploy::count_source()
            ])
            && od_core::lost_void_loading::lore_source()["asset_sha256"]
                == hex::encode(Sha256::digest(od_core::lost_void_loading::lore_logo())),
        "迷失之地加载标识来源不兼容"
    );
    let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
    let ocr = lock["models"]
        .as_array()
        .context("模型清单无效")?
        .iter()
        .find(|m| m["id"] == "ppocrv5")
        .context("缺少 OCR")?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    for (name, bytes) in od_core::lost_void_scene::templates() {
        expected.insert(
            format!("image/lost-void-scene/{name}.png"),
            (bytes.len() as u64, hex::encode(Sha256::digest(bytes))),
        );
    }
    for (name, bytes) in od_core::lost_void_scene::avatar_templates() {
        expected.insert(
            name.into(),
            (bytes.len() as u64, hex::encode(Sha256::digest(bytes))),
        );
    }
    let pipeline = serde_json::to_vec_pretty(&od_core::lost_void_scene::pipeline())?;
    expected.insert(
        "pipeline/lost_void_scene.json".into(),
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
        .context("迷失之地画面文件清单无效")?;
    ensure!(files.len() == expected.len(), "迷失之地画面文件数量不匹配");
    let mut found = BTreeSet::new();
    for file in files {
        let name = file["path"].as_str().context("迷失之地画面文件路径无效")?;
        let (size, hash) = expected.get(name).context("迷失之地画面资源含未知文件")?;
        ensure!(
            found.insert(name.to_owned()) && file["bytes"] == *size && file["sha256"] == *hash,
            "迷失之地画面文件声明不匹配"
        );
        let path = root.join(name);
        od_core::config::reject_links(&path)?;
        ensure!(
            std::fs::metadata(&path)?.len() == *size,
            "迷失之地画面文件大小不匹配"
        );
        ensure!(
            hex::encode(Sha256::digest(std::fs::read(path)?)) == *hash,
            "迷失之地画面文件内容不匹配"
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
                name == "lost_void_scene-manifest.json" || expected.contains_key(&name)
            } else {
                expected.keys().any(|p| p.starts_with(&format!("{name}/")))
            },
            "迷失之地画面资源含额外文件或目录"
        );
    }
    Ok(root)
}
