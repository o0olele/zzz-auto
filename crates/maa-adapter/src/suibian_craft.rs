//! 制造的选中、白字商品、可制造标记、邦布和同台位运行状态均来自同一新帧。
pub use crate::suibian_adventure::AdventureInputEnd as CraftInputEnd;
use crate::{
    NativeSession,
    suibian_adventure::{AdventureInputEnd, Reader, Word, has, point, words},
};
use anyhow::{Result, ensure};
use od_core::suibian_craft::{
    CraftAction as A, CraftBangboo, CraftCard, CraftGood, CraftSelection, CraftSlot,
    CraftView as V, SlotState, TempleCraft, goods_key,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
pub const BANGBOOS: [[i32; 4]; 8] = [
    [133, 182, 241, 324],
    [402, 186, 233, 307],
    [669, 187, 221, 305],
    [935, 187, 219, 303],
    [135, 526, 239, 292],
    [400, 526, 238, 292],
    [666, 526, 236, 292],
    [929, 526, 236, 291],
];
struct Sight {
    view: V,
    points: BTreeMap<String, [i32; 2]>,
}
fn pair(text: &str, separator: char) -> Option<(u32, u32)> {
    let (a, b) = text.split_once(separator)?;
    let parse = |t: &str| t.replace(',', "").parse().ok();
    Some((parse(a)?, parse(b)?))
}
fn row_name(text: &str) -> Option<String> {
    if !text.starts_with(['[', '［', '【']) {
        return None;
    }
    let t = text
        .split("可制造")
        .next()?
        .trim_end_matches(['(', '（', ')', '）']);
    goods_key(t.trim_end_matches(|c: char| c.is_ascii_digit()))
}
fn exact_one(
    texts: &[Word],
    area: [i32; 4],
    parse: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let mut values: Vec<_> = texts
        .iter()
        .filter(|w| w.inside(area))
        .filter_map(|w| parse(&w.text))
        .collect();
    values.sort();
    values.dedup();
    (values.len() == 1).then(|| values[0].clone())
}
fn craft_words(r: &Reader<'_>) -> Result<Vec<Word>> {
    let mut texts = words(&r.read("Craft.Area.Text", Some([110, 850, 1670, 130]))?)?;
    let picker = has(&texts, "确认派驻", [1300, 860, 1780, 980])
        && has(&texts, "派驻邦布", [110, 850, 500, 965]);
    if !picker {
        return words(&r.read("Craft.Area.Text", None)?);
    }
    let modal_roi = [450, 250, 1000, 650];
    let modal = words(&r.read("Craft.Area.Text", Some(modal_roi))?)?;
    texts.retain(|word| !word.inside([450, 250, 1450, 900]));
    texts.extend(modal);
    // 先交给原有弹窗判定，避免后续卡片 / 标题边缘裁切掉覆盖其上的确认文字。
    if point(&texts, "确认", [550, 400, 1400, 950]).is_some()
        || has(&texts, "当前拥有", [600, 320, 1080, 385])
    {
        return Ok(texts);
    }
    // 完整覆盖八张卡片和原有弹窗判断区，排除右侧无关的天赋、货品和说明文字。
    // 重叠区域采用本次区域的完整识别结果，避免同一“确认”或电量被重复计数。
    for roi @ [x, y, w, h] in [
        [133, 182, 1035, 68],
        [133, 250, 502, 585],
        [133, 526, 1035, 65],
        [1260, 470, 530, 80],
    ] {
        let read = words(&r.read("Craft.Area.Text", Some(roi))?)?;
        texts.retain(|word| !word.inside([x, y, x + w, y + h]));
        texts.extend(read);
    }
    Ok(texts)
}
fn card_energy(
    r: &Reader<'_>,
    inside: &[&Word],
    [x, y, w, h]: [i32; 4],
) -> Result<Option<(u32, u32)>> {
    let found: Vec<_> = inside.iter().filter_map(|v| pair(&v.text, '/')).collect();
    if !found.is_empty() {
        return Ok((found.len() == 1).then(|| found[0]));
    }
    // 局部检测可能将“1040/1040”拆成两框。回到同一帧的数字框做 Maa 单行识别，
    // 不拼接文字或推测缺失数字；多个完整读数及没有斜杠的未知内容保持不可读。
    let pieces: Vec<_> = inside
        .iter()
        .filter(|v| {
            v.center()[1] < y + 65
                && !v.text.is_empty()
                && v.text
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b == b'/' || b == b',')
        })
        .collect();
    if pieces.is_empty() || pieces.len() > 4 || !pieces.iter().any(|v| v.text.contains('/')) {
        return Ok(None);
    }
    let left = (pieces.iter().map(|v| v.rect[0]).min().unwrap() - 3).max(x);
    let top = (pieces.iter().map(|v| v.rect[1]).min().unwrap() - 3).max(y);
    let right = (pieces.iter().map(|v| v.rect[0] + v.rect[2]).max().unwrap() + 3).min(x + w);
    let bottom = (pieces.iter().map(|v| v.rect[1] + v.rect[3]).max().unwrap() + 3).min(y + h);
    let mut pipeline = r.pipeline.clone();
    pipeline["Craft.Area.Text"]["only_rec"] = json!(true);
    let line = Reader {
        pipeline,
        session: r.session,
        frame: r.frame,
        cancel: r.cancel,
        started: r.started,
        timeout: r.timeout,
    };
    let values = words(&line.read(
        "Craft.Area.Text",
        Some([left, top, right - left, bottom - top]),
    )?)?;
    Ok((values.len() == 1)
        .then(|| pair(&values[0].text, '/'))
        .flatten())
}
impl NativeSession {
    pub fn require_suibian_craft(&self) -> Result<()> {
        let root = super::suibian_temple::validate_suibian_temple_bundle(
            self.suibian_temple_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        let m: Value =
            serde_json::from_slice(&std::fs::read(root.join("suibian-temple-manifest.json"))?)?;
        ensure!(
            matches!(m["schema_version"].as_u64(), Some(3..=8)),
            "制造需要 schema 3 随便观资源"
        );
        Ok(())
    }
    fn craft_sight(
        &self,
        frame: &crate::RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let r = Reader {
            session: self,
            frame,
            cancel,
            started: Instant::now(),
            timeout,
            pipeline: od_core::suibian_craft_pipeline::pipeline(),
        };
        let result = (|| -> Result<Sight> {
            let t = craft_words(&r)?;
            let mut points = BTreeMap::new();
            if has(&t, "当前拥有", [600, 320, 1080, 385])
                && has(&t, "经营-制造", [600, 510, 1300, 610])
                && let Some(name) = exact_one(&t, [620, 275, 1080, 340], goods_key)
            {
                return Ok(Sight {
                    view: V::MaterialMenu(name),
                    points,
                });
            }
            if (has(&t, "调整计划", [500, 250, 1400, 650])
                || has(&t, "调整制造计划", [500, 250, 1400, 650]))
                && (has(&t, "返回材料", [500, 300, 1400, 730])
                    || (has(&t, "返还", [500, 300, 1400, 730])
                        && has(&t, "材料", [500, 300, 1400, 730])))
                && let Some(p) = point(&t, "确认", [800, 600, 1350, 850])
            {
                points.insert("plan".into(), p);
                return Ok(Sight {
                    view: V::PlanConfirm,
                    points,
                });
            }
            if has(&t, "邦布电量不足", [450, 250, 1450, 750])
                && let Some(p) = point(&t, "确认", [550, 450, 1400, 950])
            {
                points.insert("reject".into(), p);
                return Ok(Sight {
                    view: V::AssignRejected,
                    points,
                });
            }
            if point(&t, "确认", [550, 400, 1400, 900]).is_some() {
                return Ok(Sight {
                    view: V::Unknown,
                    points,
                });
            }
            if has(&t, "邻里街坊", [1300, 900, 1650, 1000])
                && has(&t, "派驻邦布", [80, 720, 280, 820])
            {
                let view = if has(&t, "托管中", [80, 940, 280, 1040])
                    || has(&t, "自动托管中", [1200, 630, 1740, 790])
                {
                    V::Managed
                } else if has(&t, "经营", [1240, 570, 1430, 650]) {
                    V::Entry
                } else {
                    V::Unknown
                };
                return Ok(Sight { view, points });
            }
            if !r.hit("Hosting.Area.Back", None)? {
                return Ok(Sight {
                    view: V::Unknown,
                    points,
                });
            }
            if has(&t, "确认派驻", [1300, 860, 1780, 980])
                && has(&t, "邦布制造增益", [1260, 470, 1790, 550])
                && has(&t, "派驻邦布", [110, 850, 500, 965])
            {
                let mut cards = [CraftCard::Unknown; 8];
                let mut selected = vec![];
                for (i, rect @ [x, y, w, h]) in BANGBOOS.into_iter().enumerate() {
                    let area = [x, y, x + w, y + h];
                    let inside: Vec<_> = t.iter().filter(|v| v.inside(area)).collect();
                    let energy = card_energy(&r, &inside, rect)?;
                    cards[i] = if inside
                        .iter()
                        .any(|v| ["制造中", "游历中", "售卖中"].iter().any(|s| v.text == *s))
                    {
                        CraftCard::Busy
                    } else if let Some((current, total)) = energy
                        && inside.iter().any(|v| v.text.contains("经营"))
                    {
                        if current > 0 && current <= total {
                            CraftCard::Available
                        } else {
                            CraftCard::Exhausted
                        }
                    } else {
                        CraftCard::Unknown
                    };
                    if r.hit("Craft.Area.Selected", Some(rect))? {
                        selected.push(i as u8 + 1);
                    }
                }
                if let Some(p) = point(&t, "确认派驻", [1300, 860, 1780, 980]) {
                    points.insert("assign".into(), p);
                }
                return Ok(Sight {
                    view: V::Picker {
                        cards,
                        selected: if selected.len() == 1 {
                            Some(selected[0])
                        } else {
                            None
                        },
                    },
                    points,
                });
            }
            if has(&t, "制造坊", [160, 15, 340, 90]) {
                let slot = exact_one(&t, [300, 930, 1140, 995], |s| {
                    let rest = s.strip_prefix("请选择")?;
                    let digit = rest.chars().next()?;
                    ('1'..='4').contains(&digit).then(|| digit.to_string())
                })
                .and_then(|s| s.parse::<u8>().ok());
                let name = exact_one(&t, [1410, 380, 1800, 452], goods_key);
                if let (Some(slot), Some(goods)) = (slot, name) {
                    let white = words(&r.read("Craft.Area.White", None)?)?;
                    let mut rows = vec![];
                    for text in t.iter().filter(|v| v.inside([306, 326, 1136, 930])) {
                        let Some(name) = row_name(&text.text) else {
                            continue;
                        };
                        let [cx, cy] = text.center();
                        let is_white = white.iter().any(|v| {
                            (v.center()[1] - cy).abs() < 20
                                && row_name(&v.text).as_ref() == Some(&name)
                        });
                        let makeable = t.iter().any(|v| {
                            v.inside([306, 326, 1136, 930])
                                && v.text.contains("可制造")
                                && (v.center()[1] - cy).abs() < 20
                                && (v.center()[0] > cx || v.text == text.text)
                        });
                        points.insert(
                            format!("goods:{name}"),
                            [text.rect[0] + text.rect[2] + 50, cy],
                        );
                        rows.push(CraftGood {
                            name,
                            white: is_white,
                            makeable,
                        });
                    }
                    rows.sort_by_key(|g| points[&format!("goods:{}", g.name)][1]);
                    let amount = exact_one(&t, [1360, 625, 1710, 705], |s| {
                        s.strip_prefix("待制造")
                            .and_then(|n| n.parse::<u32>().ok())
                            .map(|n| n.to_string())
                    })
                    .and_then(|n| n.parse().ok());
                    let energy: Vec<_> = t
                        .iter()
                        .filter(|v| v.inside([1200, 315, 1410, 370]))
                        .filter_map(|v| pair(&v.text, '-'))
                        .collect();
                    let bangboo = if has(&t, "未选择邦布", [1180, 200, 1820, 1000])
                        || has(&t, "请先选择邦布", [1180, 200, 1820, 1000])
                    {
                        CraftBangboo::Missing
                    } else if has(&t, "邦布电量不足", [1180, 200, 1820, 1000]) {
                        CraftBangboo::Exhausted
                    } else if energy.len() == 1 {
                        if energy[0].0 >= energy[0].1 && energy[0].0 > 0 {
                            CraftBangboo::Ready
                        } else {
                            CraftBangboo::Exhausted
                        }
                    } else {
                        CraftBangboo::Unknown
                    };
                    let materials: Vec<_> = t
                        .iter()
                        .filter(|v| v.inside([1410, 312, 1790, 372]))
                        .filter_map(|v| pair(&v.text, '/'))
                        .collect();
                    let enough = if has(&t, "所需材料不足", [1180, 700, 1830, 1020]) {
                        Some(false)
                    } else if !materials.is_empty() {
                        Some(materials.iter().all(|(n, need)| need > &0 && n >= need))
                    } else {
                        None
                    };
                    let start = point(&t, "开始制造", [1260, 920, 1760, 1020])
                        .or_else(|| point(&t, "调整计划", [1260, 920, 1760, 1020]));
                    if let Some(p) = start {
                        points.insert("start".into(), p);
                    }
                    if has(&t, "制造中", [1200, 600, 1800, 1000])
                        && has(&t, "剩余", [1200, 600, 1800, 1000])
                    {
                        return Ok(Sight {
                            view: V::Running { slot, goods },
                            points,
                        });
                    }
                    let list_readable = !rows.is_empty();
                    return Ok(Sight {
                        view: V::Selection(CraftSelection {
                            slot,
                            goods,
                            bangboo,
                            materials: enough,
                            amount,
                            can_start: start.is_some(),
                            list_readable,
                            rows,
                        }),
                        points,
                    });
                }
            }
            if has(&t, "制造坊", [1380, 470, 1630, 550]) && has(&t, "街区", [230, 15, 410, 85])
            {
                let mut slots = vec![];
                for (i, x) in [440, 785, 1128, 1470].into_iter().enumerate() {
                    let area = [x - 150, 650, x + 150, 1040];
                    let status = if point(&t, "开工", area).is_some() {
                        SlotState::Open
                    } else if has(&t, "制造暂停", area) {
                        SlotState::Paused
                    } else if has(&t, "制造中", area) {
                        SlotState::Running
                    } else if has(&t, "未解锁", area) {
                        SlotState::Unavailable
                    } else {
                        SlotState::Unknown
                    };
                    let goods = exact_one(&t, area, row_name);
                    if let Some(p) = point(&t, "开工", area).or_else(|| point(&t, "制造暂停", area))
                    {
                        points.insert(format!("slot:{}", i + 1), p);
                    }
                    slots.push(CraftSlot {
                        slot: i as u8 + 1,
                        state: status,
                        goods,
                    });
                }
                return Ok(Sight {
                    view: V::Overview(slots),
                    points,
                });
            }
            if has(&t, "开物清单", [280, 675, 650, 775])
                && has(&t, "制造台", [800, 675, 1130, 775])
                && has(&t, "货架", [1360, 675, 1640, 775])
            {
                return Ok(Sight {
                    view: V::Management,
                    points,
                });
            }
            Ok(Sight {
                view: V::Unknown,
                points,
            })
        })();
        if cancel.load(Ordering::Acquire) {
            Ok(None)
        } else {
            result.map(Some)
        }
    }
    pub fn read_suibian_craft(&self, cancel: &AtomicBool, timeout: Duration) -> Result<Option<V>> {
        let started = Instant::now();
        self.require_suibian_craft()?;
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        Ok(self
            .craft_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            .filter(|_| frame.age() <= Duration::from_millis(1500))
            .map(|s| s.view))
    }
    pub fn act_suibian_craft(
        &mut self,
        flow: &TempleCraft,
        action: &A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<CraftInputEnd> {
        let started = Instant::now();
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_suibian_craft()?;
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "制造控制器模式不匹配"
        );
        let result = (|| -> Result<CraftInputEnd> {
            if cancel.load(Ordering::Acquire) {
                return Ok(CraftInputEnd::NotPosted);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(CraftInputEnd::NotPosted);
            };
            let Some(s) = self.craft_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            else {
                return Ok(CraftInputEnd::NotPosted);
            };
            if started.elapsed() >= timeout {
                return Ok(CraftInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &s.view) {
                return Ok(CraftInputEnd::Changed);
            }
            let p = match action {
                A::OpenManagement => Some([1320, 605]),
                A::OpenWorkshop => Some([960, 500]),
                A::OpenSlot(n) => s.points.get(&format!("slot:{n}")).copied(),
                A::OpenPicker => Some([1275, 290]),
                A::SelectBangboo(n) => BANGBOOS
                    .get(usize::from(n.saturating_sub(1)))
                    .map(|r| [r[0] + r[2] / 2, r[1] + r[3] / 2]),
                A::Assign(_) => s.points.get("assign").copied(),
                A::SelectGood(g) => s.points.get(&format!("goods:{g}")).copied(),
                A::Scroll => Some([900, 800]),
                A::Start(_) => s.points.get("start").copied(),
                A::ConfirmPlan => s.points.get("plan").copied(),
                A::Back => Some(if matches!(s.view, V::AssignRejected) {
                    *s.points.get("reject").unwrap_or(&[0, 0])
                } else {
                    [116, 52]
                }),
            };
            let Some([x, y]) = p.filter(|[x, y]| (1..1920).contains(x) && (1..1080).contains(y))
            else {
                return Ok(CraftInputEnd::Changed);
            };
            let input = |x, y| od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input(x, y), true, cancel)? {
                return Ok(CraftInputEnd::NotPosted);
            }
            let held = Instant::now();
            let hold = if *action == A::Scroll { 300 } else { 100 };
            while held.elapsed() < Duration::from_millis(hold)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                if *action == A::Scroll {
                    self.combat_input(
                        input(x, y - (held.elapsed().as_millis().min(300) as i32)),
                        true,
                        cancel,
                    )?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input(x, y), false, &AtomicBool::new(false))?;
            Ok(AdventureInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|r| release.map(|_| r))
    }
}
