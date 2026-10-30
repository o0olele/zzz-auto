use crate::{NativeSession, PipelineEnd, RecognitionImage, predefined_teams::TeamPageMode};
use anyhow::{Context, Result, ensure};
use od_core::{
    predefined_teams::{self, TeamConfig},
    team_selection::{self, SelectionAction, SelectionCard, SelectionPhase, TeamSelection},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct SelectionPage {
    pub mode: TeamPageMode,
    pub cards: Vec<SelectionCard>,
    pub unreadable: Vec<u8>,
    pub confirm_enabled: bool,
    pub deployment: bool,
    pub slot_cards: Vec<od_core::team_selection_slots::SlotSelectionCard>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionEnd {
    Completed,
    Cancelled,
    TimedOut,
}

fn rect(reading: &Value) -> Option<[i32; 4]> {
    let value = serde_json::from_value(reading["box"].clone()).ok()?;
    od_core::intel_board::valid_rect(value).then_some(value)
}
fn in_area(reading: &Value, [left, top, right, bottom]: [i32; 4]) -> bool {
    rect(reading).is_some_and(|[x, y, w, h]| {
        (left..right).contains(&(x + w / 2)) && (top..bottom).contains(&(y + h / 2))
    })
}

impl NativeSession {
    fn selection_by_slot(&self) -> Result<bool> {
        let root = self.daily_bundle.as_deref().unwrap_or(&self.resource_root);
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(root.join("basic-daily-manifest.json"))?)?;
        Ok(manifest["schema_version"]
            .as_u64()
            .is_some_and(|schema| schema >= 10))
    }
    pub fn inspect_team_selection(
        &self,
        frame: &RecognitionImage,
        config: &TeamConfig,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<SelectionPage>> {
        self.inspect_team_selection_at(frame, config, 0, cancel, timeout)
    }
    pub fn inspect_team_selection_at(
        &self,
        frame: &RecognitionImage,
        config: &TeamConfig,
        page: u8,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<SelectionPage>> {
        ensure!(page < 5, "选队页号无效");
        let slot_page = self.selection_by_slot()?.then_some(page);
        self.inspect_team_selection_mode(frame, config, slot_page, cancel, timeout)
    }
    fn inspect_team_selection_mode(
        &self,
        frame: &RecognitionImage,
        config: &TeamConfig,
        slot_page: Option<u8>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<SelectionPage>> {
        let started = Instant::now();
        let ready = || -> Result<bool> {
            if cancel.load(Ordering::Acquire) {
                return Ok(false);
            }
            ensure!(started.elapsed() < timeout, "预备编队选择识别超时");
            Ok(true)
        };
        config.validate().map_err(anyhow::Error::msg)?;
        ensure!(!self.tasker.running(), "选队识别前须等待 Pipeline 收尾");
        if !ready()? {
            return Ok(None);
        }
        let mut page = SelectionPage {
            mode: TeamPageMode::Unknown,
            cards: vec![],
            unreadable: vec![],
            confirm_enabled: false,
            deployment: false,
            slot_cards: vec![],
        };
        let ocr =
            self.recognize_prepared(frame, "OCR", &json!({"model":"ppocrv5","threshold":0.6}))?;
        if !ready()? {
            return Ok(None);
        }
        let texts = ocr.detail["detail"]["filtered"]
            .as_array()
            .context("选队 OCR 缺少结果")?;
        let has = |word: &str, area| texts.iter().any(|t| t["text"] == word && in_area(t, area));
        if [
            "恢复电量",
            "快捷使用",
            "获得",
            "确认",
            "取消",
            "确定并出战",
            "确定並出战",
        ]
        .iter()
        .any(|word| has(word, [570, 290, 1310, 810]))
        {
            return Ok(Some(page));
        }
        let back = self.recognize_prepared(frame, "TemplateMatch", &json!({"template":"daily/back.png","roi":[54,12,115,80],"green_mask":true,"threshold":0.7}))?;
        if !ready()? {
            return Ok(None);
        }
        if back.detail["hit"] != true {
            return Ok(Some(page));
        }
        if has("预备编队", [368, 1002, 563, 1057]) && has("出战", [1638, 1002, 1832, 1052]) {
            page.deployment = true;
            return Ok(Some(page));
        }
        let counters = texts
            .iter()
            .filter(|t| in_area(t, [765, 100, 890, 800]) || in_area(t, [1580, 100, 1710, 800]))
            .filter(|t| {
                t["text"]
                    .as_str()
                    .and_then(predefined_teams::member_count)
                    .is_some()
            })
            .count();
        if counters < 2 {
            return Ok(Some(page));
        }
        let selection_mark = texts.iter().any(|t| {
            (in_area(t, [795, 215, 940, 960]) || in_area(t, [1610, 215, 1755, 960]))
                && t["text"].as_str().is_some_and(|text| {
                    text.to_uppercase().contains("SELECT")
                        || text.to_uppercase().starts_with("TEAM")
                })
        });
        if !has("预备出战", [1570, 985, 1880, 1070]) && !selection_mark {
            page.mode = TeamPageMode::Editing;
            return Ok(Some(page));
        }
        page.mode = TeamPageMode::Selection;
        let white = self.recognize_prepared(frame, "OcrText", &json!({"model":"ppocrv5","roi":[1634,1000,192,54],
            "threshold":0.6,"words":["预备出战"],"match_mode":"exact","color_range":[[240,240,240],[255,255,255]]}))?;
        if !ready()? {
            return Ok(None);
        }
        page.confirm_enabled = white.detail["hit"] == true;
        if let Some(index) = slot_page {
            let mut readings = texts.clone();
            let (_, unreadable) = crate::team_selection_slots::read_cards(index, &readings)?;
            for slot in unreadable {
                if !ready()? {
                    return Ok(None);
                }
                let cell = usize::from(slot - index * 4);
                let [_, top, _, height] = od_core::team_slots::CARDS[cell];
                let left = if cell % 2 == 0 { 774 } else { 1598 };
                let result=self.recognize_prepared(frame,"OCR",&json!({"model":"ppocrv5","roi":[left,top+height/2-85,166,170],"threshold":0.6}))?;
                readings.extend(
                    result.detail["detail"]["filtered"]
                        .as_array()
                        .context("槽位按钮 OCR 缺少结果")?
                        .iter()
                        .cloned(),
                );
            }
            let (cards, unreadable) = crate::team_selection_slots::read_cards(index, &readings)?;
            page.slot_cards = cards;
            page.unreadable = unreadable;
            return Ok(ready()?.then_some(page));
        }
        let mut names = BTreeMap::new();
        for reading in texts {
            let Some(text) = reading["text"].as_str() else {
                continue;
            };
            let Some(index) = predefined_teams::match_name(config, text) else {
                continue;
            };
            let Some(bounds @ [x, y, _, _]) = rect(reading) else {
                continue;
            };
            if !((140..=400).contains(&x) || (960..=1220).contains(&x)) || !(100..=740).contains(&y)
            {
                continue;
            }
            if names.insert(index, (bounds, text.to_owned())).is_some() {
                page.unreadable.push(index);
            }
        }
        for (index, (name_rect @ [x, y, _, _], recognized_name)) in names {
            if page.unreadable.contains(&index) {
                continue;
            }
            let counts: BTreeSet<_> = texts
                .iter()
                .filter(|t| in_area(t, [x + 580, y - 20, x + 740, y + 50]))
                .filter_map(|t| t["text"].as_str())
                .filter_map(predefined_teams::member_count)
                .collect();
            let Some(&members) = counts.iter().next().filter(|_| counts.len() == 1) else {
                page.unreadable.push(index);
                continue;
            };
            let marker_area = [x + 610, y + 70, x + 770, y + 195];
            let mut words: Vec<_> = texts
                .iter()
                .filter(|t| in_area(t, marker_area))
                .filter_map(|t| t["text"].as_str().map(String::from))
                .collect();
            let mut order = team_selection::marker(&words);
            if order.is_err() {
                let result = self.recognize_prepared(frame,"OCR",&json!({"model":"ppocrv5","roi":[marker_area[0],marker_area[1],160,125],"threshold":0.6}))?;
                if !ready()? {
                    return Ok(None);
                }
                words = result.detail["detail"]["filtered"]
                    .as_array()
                    .context("选队标记 OCR 缺少结果")?
                    .iter()
                    .filter_map(|t| t["text"].as_str().map(String::from))
                    .collect();
                order = team_selection::marker(&words);
            }
            match order {
                Ok(order) => page.cards.push(SelectionCard {
                    index,
                    members,
                    order,
                    name_rect,
                    recognized_name,
                }),
                Err(_) => page.unreadable.push(index),
            }
        }
        if !ready()? {
            return Ok(None);
        }
        page.cards.sort_by_key(|card| card.index);
        page.unreadable.sort_unstable();
        page.unreadable.dedup();
        Ok(Some(page))
    }

    /// 调用方持有运行槽；每次投递前发布检查点，返回前等待 Tasker 与输入收尾。
    pub fn select_predefined_teams(
        &mut self,
        config: &TeamConfig,
        selection: &mut TeamSelection,
        game: bool,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
        mut publish: impl FnMut(TeamSelection) -> Result<()>,
    ) -> Result<SelectionEnd> {
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "选队控制器须为前台界面控制器"
        );
        crate::validate_basic_daily_bundle(
            self.daily_bundle.as_deref().unwrap_or(&self.resource_root),
        )?;
        config.validate().map_err(anyhow::Error::msg)?;
        selection.validate().map_err(anyhow::Error::msg)?;
        let mut resumed_confirmation = selection.pending == Some(SelectionAction::Confirm);
        if !resumed_confirmation && selection.phase != SelectionPhase::Confirmed {
            selection.restart().map_err(anyhow::Error::msg)?;
        }
        let by_slot = self.selection_by_slot()?;
        let definition = if by_slot {
            slot_selection_pipeline()
        } else {
            selection_pipeline()
        };
        self._resource.override_pipeline_json(&definition)?;
        let started = Instant::now();
        let result = (|| -> Result<SelectionEnd> {
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Ok(SelectionEnd::Cancelled);
                }
                if started.elapsed() >= timeout {
                    return Ok(SelectionEnd::TimedOut);
                }
                let Some(frame) = self.navigation_frame(&cancel)? else {
                    return Ok(SelectionEnd::Cancelled);
                };
                let Some(page) = self.inspect_team_selection_at(
                    &frame.image,
                    config,
                    selection.page,
                    &cancel,
                    timeout.saturating_sub(started.elapsed()),
                )?
                else {
                    return Ok(SelectionEnd::Cancelled);
                };
                if cancel.load(Ordering::Acquire) {
                    return Ok(SelectionEnd::Cancelled);
                }
                if page.deployment {
                    if selection.phase != SelectionPhase::Confirmed {
                        selection.observe_deployment().map_err(anyhow::Error::msg)?;
                    }
                    publish(selection.clone())?;
                    return Ok(if cancel.load(Ordering::Acquire) {
                        SelectionEnd::Cancelled
                    } else {
                        SelectionEnd::Completed
                    });
                }
                ensure!(
                    page.mode != TeamPageMode::Editing,
                    "编队管理页不能执行出战选择"
                );
                if by_slot && resumed_confirmation && page.mode == TeamPageMode::Selection {
                    selection.restart().map_err(anyhow::Error::msg)?;
                    resumed_confirmation = false;
                }
                let resetting = by_slot
                    && selection.top_scrolls < 4
                    && matches!(
                        selection.phase,
                        SelectionPhase::ResetClear
                            | SelectionPhase::ResetChoose
                            | SelectionPhase::ResetVerify
                    );
                let action = if page.mode != TeamPageMode::Selection
                    || (!resetting
                        && (!page.unreadable.is_empty()
                            || (if by_slot {
                                page.slot_cards.is_empty()
                            } else {
                                page.cards.is_empty()
                            }))) {
                    None
                } else {
                    if resumed_confirmation {
                        selection.restart().map_err(anyhow::Error::msg)?;
                        resumed_confirmation = false;
                    }
                    (if by_slot {
                        selection.next_slots(&page.slot_cards, page.confirm_enabled)
                    } else {
                        selection.next(&page.cards, page.confirm_enabled)
                    })
                    .map_err(anyhow::Error::msg)?
                };
                if let Some(action) = action {
                    let mut overrides = json!({});
                    let entry = match action {
                        SelectionAction::Up => "__ZzzSelect.Up",
                        SelectionAction::Down => "__ZzzSelect.Down",
                        SelectionAction::Confirm => "__ZzzSelect.Confirm",
                        SelectionAction::Toggle { index, order } => {
                            if by_slot {
                                slot_toggle_overrides(&page, index, order, &mut overrides)?;
                                "__ZzzSelect.Toggle"
                            } else {
                                let card = page
                                    .cards
                                    .iter()
                                    .find(|card| card.index == index)
                                    .context("选队点击缺少当前帧卡片")?;
                                let [x, y, w, h] = card.name_rect;
                                overrides["__ZzzSelect.Name"] = json!({"roi":[x.saturating_sub(15),y.saturating_sub(10),(w+30).min(1920-x),h+20],
                                "expected":[format!("^{}$",regex::escape(&card.recognized_name))]});
                                overrides["__ZzzSelect.Marker"] = json!({"roi":[x+610,y+70,160,125],"expected":if order.is_some() { json!(["(?i).*SELECT.*"]) } else { json!(["(?i)^TEAM.*"]) }});
                                overrides["__ZzzSelect.Toggle"] = json!({"target":[x+300,y+h/2]});
                                "__ZzzSelect.Toggle"
                            }
                        }
                    };
                    publish(selection.clone())?;
                    if cancel.load(Ordering::Acquire) {
                        if action == SelectionAction::Confirm {
                            selection.pending = None;
                            publish(selection.clone())?;
                        }
                        return Ok(SelectionEnd::Cancelled);
                    }
                    let remaining = timeout.saturating_sub(started.elapsed());
                    if remaining.is_zero() {
                        if action == SelectionAction::Confirm {
                            selection.pending = None;
                            publish(selection.clone())?;
                        }
                        return Ok(SelectionEnd::TimedOut);
                    }
                    let execution = self.run_pipeline_detailed(
                        &format!("{entry}.Guard"),
                        &crate::PipelineInputPlan {
                            keys: BTreeSet::new(),
                            contacts: [0].into(),
                        },
                        cancel.clone(),
                        remaining.min(Duration::from_secs(10)),
                        &overrides.to_string(),
                        true,
                    )?;
                    ensure!(execution.report.released, "选队 Pipeline 尚未确认释放输入");
                    ensure!(
                        !execution
                            .nodes
                            .iter()
                            .any(|node| node.name == "__ZzzSelect.Blocked" && node.completed),
                        "选队输入前出现弹窗，不能点击背景"
                    );
                    match execution.report.end {
                        PipelineEnd::Cancelled => return Ok(SelectionEnd::Cancelled),
                        PipelineEnd::TimedOut => return Ok(SelectionEnd::TimedOut),
                        PipelineEnd::Failed => anyhow::bail!(
                            "选队输入或对应画面检查失败（{entry}）：{:?}",
                            execution.report
                        ),
                        PipelineEnd::Succeeded => {
                            ensure!(
                                execution.nodes.iter().any(|node| node.name
                                    == (if by_slot
                                        && matches!(
                                            action,
                                            SelectionAction::Up | SelectionAction::Down
                                        ) {
                                        format!("{entry}Done")
                                    } else {
                                        entry.into()
                                    })
                                    && node.completed),
                                "选队 Pipeline 缺少实际输入节点证明"
                            );
                            if matches!(action, SelectionAction::Up | SelectionAction::Down) {
                                selection.acknowledge_scroll().map_err(anyhow::Error::msg)?;
                            }
                        }
                    }
                    publish(selection.clone())?;
                } else {
                    let idle = Instant::now();
                    while idle.elapsed() < Duration::from_millis(100)
                        && !cancel.load(Ordering::Acquire)
                        && started.elapsed() < timeout
                    {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}

fn selection_pipeline() -> Value {
    let ocr = |roi, text| json!({"recognition":"OCR","model":"ppocrv5","roi":roi,"expected":[text],"threshold":0.6,"action":"DoNothing","next":[]});
    let mut p = json!({
        "__ZzzSelect.Back":{"recognition":"TemplateMatch","template":"daily/back.png","roi":[54,12,115,80],"green_mask":true,"threshold":0.7},
        "__ZzzSelect.Page":{"recognition":"And","all_of":["__ZzzSelect.Back","__ZzzSelect.Presence"]},
        "__ZzzSelect.White":{"recognition":"ColorMatch","method":4,"lower":[240,240,240],"upper":[255,255,255]},
        "__ZzzSelect.Toggle":{"recognition":"And","all_of":["__ZzzSelect.Page","__ZzzSelect.Name","__ZzzSelect.Marker"],"action":"LongPress","target":[450,160],"duration":100},
        "__ZzzSelect.Confirm":{"recognition":"And","all_of":["__ZzzSelect.Page","__ZzzSelect.Enabled"],"box_index":1,"action":"LongPress","duration":100},
        "__ZzzSelect.Up":{"recognition":"And","all_of":["__ZzzSelect.Page"],"action":"Swipe","begin":[960,300],"end":[960,900],"duration":500},
        "__ZzzSelect.Down":{"recognition":"And","all_of":["__ZzzSelect.Page"],"action":"Swipe","begin":[960,850],"end":[960,250],"duration":500}
    });
    p["__ZzzSelect.Presence"] = ocr([1570, 985, 310, 85], "^预备出战$");
    p["__ZzzSelect.Enabled"] = ocr([1634, 1000, 192, 54], "^预备出战$");
    p["__ZzzSelect.Enabled"]["color_filter"] = json!("__ZzzSelect.White");
    p["__ZzzSelect.Name"] = ocr([140, 100, 700, 70], ".+");
    p["__ZzzSelect.Blocked"] = ocr(
        [570, 290, 740, 520],
        "^(恢复电量|快捷使用|获得|确认|取消|确定[并並]出战)$",
    );
    for entry in ["Up", "Down", "Toggle", "Confirm"] {
        p[format!("__ZzzSelect.{entry}.Guard")] = json!({"recognition":"DirectHit","action":"DoNothing","next":["__ZzzSelect.Blocked",format!("__ZzzSelect.{entry}")],"pre_delay":0,"post_delay":0});
    }
    p["__ZzzSelect.Marker"] = ocr([795, 215, 160, 125], "(?i).*SELECT.*|^TEAM.*");
    for node in p.as_object_mut().unwrap().values_mut() {
        for (key, value) in [
            ("action", json!("DoNothing")),
            ("pre_delay", json!(0)),
            ("post_delay", json!(500)),
            ("timeout", json!(5000)),
            ("rate_limit", json!(50)),
            ("next", json!([])),
            ("on_error", json!([])),
        ] {
            node.as_object_mut().unwrap().entry(key).or_insert(value);
        }
    }
    p
}

fn slot_toggle_overrides(
    page: &SelectionPage,
    index: u8,
    order: Option<u8>,
    overrides: &mut Value,
) -> Result<()> {
    let card = page
        .slot_cards
        .iter()
        .find(|c| c.identity.index == index)
        .context("选队槽位缺少当前按钮")?;
    let bounds = od_core::team_slots::CARDS
        .iter()
        .copied()
        .find(|[x, y, w, h]| {
            card.button[0] >= *x
                && card.button[1] >= *y
                && card.button[0] + card.button[2] <= x + w
                && card.button[1] + card.button[3] <= y + h
        })
        .context("按钮不在固定卡片内")?;
    let [left, top, width, _] = bounds;
    overrides["__ZzzSelect.CardCount"] = json!({"roi":[left+width-180,top,135,70],"expected":[format!("^\\s*[0-{}]\\s*[/／1]\\s*{}\\s*$",card.identity.members,card.identity.members)]});
    overrides["__ZzzSelect.Name"] = if let Some(name) = &card.identity.name {
        json!({"recognition":"OCR","model":"ppocrv5","roi":[left,top,width,70],"expected":[format!("^\\s*{}\\s*$",name.chars().map(|c|regex::escape(&c.to_string())).collect::<Vec<_>>().join("\\s*"))]})
    } else {
        json!({"recognition":"DirectHit"})
    };
    let [x, y, w, h] = card.button;
    overrides["__ZzzSelect.Marker"] = json!({"roi":[x.saturating_sub(10),y.saturating_sub(10),w+20,h+20],"expected":[if order.is_some() {"(?i)^\\s*\\+?\\s*SELECT\\s*$"} else {"(?i)^\\s*(TEAM(?:\\s*0?[12])?|SELECTED)\\s*$"}]});
    overrides["__ZzzSelect.Toggle"] = json!({"target":[x+w/2,y+h/2]});
    Ok(())
}

fn slot_selection_pipeline() -> Value {
    let mut p = selection_pipeline();
    p["__ZzzSelect.CardCount"] = json!({"recognition":"OCR","model":"ppocrv5","roi":[765,130,135,70],"expected":["^[0-3][/／1][0-3]$"],"threshold":0.6,"action":"DoNothing","next":[]});
    p["__ZzzSelect.Toggle"]["all_of"] = json!([
        "__ZzzSelect.Page",
        "__ZzzSelect.Name",
        "__ZzzSelect.CardCount",
        "__ZzzSelect.Marker"
    ]);
    let source = od_core::basic_daily::pipeline_for_schema(9);
    for direction in ["Up", "Down"] {
        for name in std::iter::once("Teams.Drag".to_string())
            .chain((0..=10).map(|i| format!("Teams.DragMove{i}")))
            .chain(std::iter::once("Teams.DragUp".into()))
        {
            let target = name.replace("Teams.Drag", &format!("__ZzzSelect.{direction}"));
            let mut node: Value = serde_json::from_str(
                &source[&name]
                    .to_string()
                    .replace("Teams.Drag", &format!("__ZzzSelect.{direction}"))
                    .replace(
                        "Teams.WaitScrolled",
                        &format!("__ZzzSelect.{direction}Done"),
                    ),
            )
            .expect("固定选队拖动节点");
            if name == "Teams.Drag" {
                node["all_of"] = json!(["__ZzzSelect.Page"]);
            }
            if direction == "Up"
                && let Some(point) = node["target"].as_array_mut()
            {
                point[1] = json!(865 - point[1].as_i64().expect("固定拖动坐标"));
            }
            p[target] = node;
        }
        p[format!("__ZzzSelect.{direction}Done")] = json!({"recognition":"DirectHit","action":"DoNothing","pre_delay":0,"post_delay":0,"next":[]});
    }
    p
}
