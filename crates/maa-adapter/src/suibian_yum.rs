//! 饮茶仙同帧读取；亮度沿用旧版 HSV V>229、比例>0.8 的按钮规则。
pub use crate::suibian_adventure::AdventureInputEnd as YumInputEnd;
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, Word, has, point, words},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    suibian_adventure::{AdventureView, parse_mission_label},
    suibian_craft::goods_key,
    suibian_yum::{
        TempleYum, YumAction as A, YumCount, YumMenu, YumNeed, YumPage, YumTransactionKind as K,
        YumView as V, order_key,
    },
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
struct Sight {
    view: V,
    points: BTreeMap<String, [i32; 2]>,
}
fn count(text: &str) -> Option<YumCount> {
    let (v, m) = text.split_once('/')?;
    Some(YumCount {
        value: v.replace(',', "").parse().ok()?,
        max: m.replace(',', "").parse().ok()?,
    })
}
fn unique(t: &[Word], area: [i32; 4], parse: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut values: Vec<_> = t
        .iter()
        .filter(|w| w.inside(area))
        .filter_map(|w| parse(&w.text))
        .collect();
    values.sort();
    values.dedup();
    (values.len() == 1).then(|| values[0].clone())
}
fn one_count(t: &[Word], area: [i32; 4], prefix: &str) -> Option<YumCount> {
    let mut v: Vec<_> = t
        .iter()
        .filter(|w| w.inside(area))
        .filter_map(|w| count(w.text.strip_prefix(prefix)?))
        .collect();
    v.sort_by_key(|n| (n.max, n.value));
    v.dedup();
    (v.len() == 1).then(|| v[0])
}
fn enabled(frame: &RecognitionImage, [x, y, w, h]: [usize; 4]) -> Result<bool> {
    ensure!(
        frame.buffer.width() == 1920
            && frame.buffer.height() == 1080
            && frame.buffer.image_type() == 16,
        "饮茶仙截图格式不匹配"
    );
    let bgr = frame.buffer.raw_data().context("饮茶仙截图没有像素")?;
    ensure!(
        bgr.len() == 1920 * 1080 * 3 && x + w <= 1920 && y + h <= 1080,
        "饮茶仙亮度区域无效"
    );
    let mut n = 0usize;
    for row in y..y + h {
        for col in x..x + w {
            let i = (row * 1920 + col) * 3;
            n += usize::from(bgr[i].max(bgr[i + 1]).max(bgr[i + 2]) > 229);
        }
    }
    Ok(n * 5 > w * h * 4)
}
impl NativeSession {
    pub fn require_suibian_yum(&self) -> Result<()> {
        let root = super::suibian_temple::validate_suibian_temple_bundle(
            self.suibian_temple_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        let m: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("suibian-temple-manifest.json"))?)?;
        ensure!(
            matches!(m["schema_version"].as_u64(), Some(4..=7)),
            "饮茶仙需要 schema 4 随便观资源"
        );
        Ok(())
    }
    fn yum_sight(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let started = Instant::now();
        let r = Reader {
            session: self,
            frame,
            cancel,
            started,
            timeout,
            pipeline: od_core::suibian_yum_pipeline::pipeline(),
        };
        let result = (|| -> Result<Sight> {
            let t = words(&r.read("Yum.Area.Text", None)?)?;
            let mut points = BTreeMap::new();
            if has(&t, "正在自动托管", [400, 400, 1500, 650])
                && has(&t, "功能暂时无法", [600, 430, 1350, 580])
            {
                return Ok(Sight {
                    view: V::Managed,
                    points,
                });
            }
            if (has(&t, "获得奖励", [500, 200, 1400, 600])
                || has(&t, "委托完成", [500, 200, 1400, 600]))
                && has(&t, "百通宝", [500, 250, 1450, 750])
                && let Some(p) = point(&t, "确认", [600, 600, 1400, 900])
            {
                points.insert("reward".into(), p);
                return Ok(Sight {
                    view: V::Reward,
                    points,
                });
            }
            for (title, kind) in [
                ("刷新委托", K::Refresh),
                ("提交委托", K::Submit),
                ("提交订单", K::Submit),
            ] {
                if has(&t, title, [500, 250, 1450, 650])
                    && let Some(p) = point(&t, "确认", [600, 550, 1400, 900])
                {
                    points.insert("prompt".into(), p);
                    return Ok(Sight {
                        view: V::Prompt(kind),
                        points,
                    });
                }
            }
            if has(&t, "当前拥有", [600, 320, 1080, 385])
                && let Some(name) = unique(&t, [620, 275, 1080, 340], goods_key)
            {
                let craft = t
                    .iter()
                    .find(|w| w.inside([620, 520, 1300, 605]) && w.text.contains("经营-制造"));
                let route = t
                    .iter()
                    .filter(|w| w.inside([620, 665, 1320, 755]))
                    .find_map(|w| {
                        let (_, tail) = w.text.split_once("游历-")?;
                        parse_mission_label(tail).map(|m| (m, w.center()))
                    });
                if let Some(w) = craft {
                    points.insert("craft".into(), w.center());
                }
                if let Some((_, p)) = &route {
                    points.insert("adventure".into(), *p);
                }
                if craft.is_some() || route.is_some() || has(&t, "经营-售卖", [620, 600, 1320, 680])
                {
                    return Ok(Sight {
                        view: V::Material(YumMenu {
                            name,
                            craft: craft.is_some(),
                            mission: route.map(|(m, _)| m),
                        }),
                        points,
                    });
                }
            }
            if point(&t, "确认", [550, 400, 1450, 900]).is_some() {
                return Ok(Sight {
                    view: V::Unknown,
                    points,
                });
            }
            if has(&t, "邻里街坊", [1300, 900, 1650, 1000])
                && has(&t, "派驻邦布", [80, 720, 280, 820])
            {
                if let Some(p) = point(&t, "邻里街坊", [1300, 900, 1650, 1000]) {
                    points.insert("shops".into(), p);
                }
                let view = if has(&t, "托管中", [80, 940, 280, 1040])
                    || has(&t, "自动托管中", [1200, 630, 1740, 790])
                {
                    V::Managed
                } else {
                    V::Entry
                };
                return Ok(Sight { view, points });
            }
            if has(&t, "饮茶仙", [270, 645, 580, 735])
                && has(&t, "好物铺", [630, 645, 950, 735])
                && let Some(p) = point(&t, "关闭", [750, 740, 1200, 850])
            {
                points.insert("close".into(), p);
                if let Some(p) = point(&t, "饮茶仙", [270, 645, 580, 735]) {
                    points.insert("yum".into(), p);
                }
                return Ok(Sight {
                    view: V::Shops,
                    points,
                });
            }
            if !r.hit("Hosting.Area.Back", None)? {
                return Ok(Sight {
                    view: V::Unknown,
                    points,
                });
            }
            if has(&t, "定期采办", [1550, 100, 1840, 185])
                && has(&t, "邻里心愿", [1260, 100, 1550, 185])
            {
                let regular = r.hit("Yum.Area.Regular", None)?;
                let cap = one_count(&t, [1390, 185, 1670, 255], "");
                let counter = one_count(&t, [820, 935, 1110, 1005], "刷新");
                let selected = unique(&t, [780, 270, 1150, 330], order_key);
                let mut orders = vec![];
                for w in t.iter().filter(|w| w.inside([1365, 264, 1662, 750])) {
                    if let Some(name) = order_key(&w.text) {
                        points.insert(format!("order:{name}"), w.center());
                        orders.push(name);
                    }
                }
                orders.sort_by_key(|n| points[&format!("order:{n}")][1]);
                let list_readable = !orders.is_empty()
                    && orders.len() <= 8
                    && orders
                        .iter()
                        .enumerate()
                        .all(|(i, n)| !orders[..i].contains(n));
                let mut needs = vec![];
                let mut readable = true;
                for (i, (top, bottom)) in
                    [(570, 615), (625, 665), (675, 715)].into_iter().enumerate()
                {
                    let area = [852, top, 1130, bottom];
                    if let Some(n) = one_count(&t, area, "") {
                        // 完整数量 OCR 决定数值，原红色范围只证明同一数量区域的颜色。
                        let quantity = t
                            .iter()
                            .find(|w| w.inside(area) && count(&w.text) == Some(n))
                            .context("材料数量缺少同帧文字框")?;
                        let [x, y, w, h] = quantity.rect;
                        let roi = [x - 2, y - 2, w + 4, h + 4];
                        let missing = od_core::intel_board::valid_rect(roi)
                            && r.hit("Yum.Area.Red", Some(roi))?;
                        if n.max == 0 || missing != (n.value < n.max) {
                            readable = false;
                        }
                        needs.push(YumNeed {
                            row: i as u8 + 1,
                            have: n.value,
                            need: n.max,
                            red: missing,
                        });
                    } else if !has(&t, "EMPTY", [800, top, 1130, bottom]) {
                        readable = false;
                    }
                }
                let submit_point = point(&t, "提交", [1440, 935, 1710, 1005]);
                let refresh_word = t
                    .iter()
                    .find(|w| w.inside([820, 935, 1110, 1005]) && w.text.starts_with("刷新"));
                let submit = submit_point
                    .map(|_| enabled(frame, [1446, 944, 252, 46]))
                    .transpose()?;
                let refresh = refresh_word
                    .map(|_| enabled(frame, [834, 942, 262, 48]))
                    .transpose()?;
                if let Some(p) = submit_point {
                    points.insert("submit".into(), p);
                }
                if let Some(w) = refresh_word {
                    points.insert("refresh".into(), w.center());
                }
                let reward = unique(&t, [1210, 825, 1350, 885], |v| {
                    v.parse::<u32>().ok().map(|n| n.to_string())
                })
                .and_then(|n| n.parse().ok());
                if let Some(p) = point(&t, "定期采办", [1550, 100, 1840, 185]) {
                    points.insert("regular".into(), p);
                }
                return Ok(Sight {
                    view: V::Page(YumPage {
                        regular,
                        cap,
                        counter,
                        submit,
                        refresh,
                        reward,
                        orders,
                        list_readable,
                        selected,
                        needs,
                        materials_readable: readable,
                    }),
                    points,
                });
            }
            if has(&t, "制造坊", [160, 15, 340, 90])
                && let Some(name) = unique(&t, [1410, 380, 1800, 452], goods_key)
            {
                return Ok(Sight {
                    view: V::Craft(name),
                    points,
                });
            }
            if has(&t, "制造坊", [1380, 470, 1630, 550])
                && [440, 785, 1128, 1470].iter().all(|x| {
                    has(&t, "制造中", [x - 150, 650, x + 150, 1040])
                        || has(&t, "未解锁", [x - 150, 650, x + 150, 1040])
                })
            {
                return Ok(Sight {
                    view: V::CraftUnavailable,
                    points,
                });
            }
            if has(&t, "游历小队", [1500, 10, 1760, 90])
                && let Some(AdventureView::Page(p)) = self.inspect_suibian_adventure_texts(
                    frame,
                    cancel,
                    timeout.saturating_sub(started.elapsed()),
                    t,
                )?
                && let Some(mission) = p.selected
            {
                return Ok(Sight {
                    view: V::Adventure(mission),
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
    pub fn read_suibian_yum(&self, cancel: &AtomicBool, timeout: Duration) -> Result<Option<V>> {
        let started = Instant::now();
        self.require_suibian_yum()?;
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        Ok(self
            .yum_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            .filter(|_| frame.age() <= Duration::from_millis(1500))
            .map(|s| s.view))
    }
    pub fn act_suibian_yum(
        &mut self,
        flow: &TempleYum,
        action: &A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<YumInputEnd> {
        let started = Instant::now();
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_suibian_yum()?;
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "饮茶仙控制器模式不匹配"
        );
        let result = (|| -> Result<YumInputEnd> {
            if cancel.load(Ordering::Acquire) {
                return Ok(YumInputEnd::NotPosted);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(YumInputEnd::NotPosted);
            };
            let Some(s) = self.yum_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            else {
                return Ok(YumInputEnd::NotPosted);
            };
            if started.elapsed() >= timeout {
                return Ok(YumInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &s.view) {
                return Ok(YumInputEnd::Changed);
            }
            let key = match action {
                A::OpenShops => "shops",
                A::OpenYum => "yum",
                A::Regular => "regular",
                A::Submit(_) => "submit",
                A::Refresh(_) => "refresh",
                A::ConfirmPrompt => "prompt",
                A::ConfirmReward => "reward",
                A::OpenCraft => "craft",
                A::OpenAdventure => "adventure",
                _ => "",
            };
            let p = match action {
                A::SelectOrder(n) => s.points.get(&format!("order:{n}")).copied(),
                A::OpenMaterial(n) => [[822, 592], [822, 645], [822, 695]]
                    .get(usize::from(n.saturating_sub(1)))
                    .copied(),
                A::Scroll => Some([1500, 650]),
                A::Back => Some(if matches!(s.view, V::Shops) {
                    *s.points.get("close").unwrap_or(&[0, 0])
                } else {
                    [116, 52]
                }),
                _ => s.points.get(key).copied(),
            };
            let Some([x, y]) = p.filter(|[x, y]| (1..1920).contains(x) && (1..1080).contains(y))
            else {
                return Ok(YumInputEnd::Changed);
            };
            let input = |x, y| od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if started.elapsed() >= timeout {
                return Ok(YumInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) {
                return Ok(YumInputEnd::Changed);
            }
            if !self.combat_input(input(x, y), true, cancel)? {
                return Ok(YumInputEnd::NotPosted);
            }
            let held = Instant::now();
            let hold = if *action == A::Scroll { 300 } else { 100 };
            while held.elapsed() < Duration::from_millis(hold)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                if *action == A::Scroll {
                    self.combat_input(
                        input(
                            x,
                            y - (held.elapsed().as_millis().min(300) * 400 / 300) as i32,
                        ),
                        true,
                        cancel,
                    )?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input(x, y), false, &AtomicBool::new(false))?;
            Ok(YumInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|v| release.map(|_| v))
    }
}
