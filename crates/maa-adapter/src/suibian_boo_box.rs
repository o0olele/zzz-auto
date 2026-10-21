//! 邦巢只在新帧核对当前卡片、详情、金额和本次弹窗后发送输入。
pub use crate::suibian_adventure::AdventureInputEnd as BooInputEnd;
use crate::{
    NativeSession,
    suibian_adventure::{Reader, Word, has, point, words},
};
use anyhow::{Result, ensure};
use od_core::suibian_boo_box::{
    BooAction as A, BooOffer, BooOutcome, BooPage, BooRefresh, BooSelection, BooSpend,
    BooView as V, TempleBooBox,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
struct Sight {
    view: V,
    points: BTreeMap<String, [i32; 2]>,
}
fn number(s: &str) -> Option<u32> {
    let s = s.replace(',', "");
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}
fn unique<T: Eq + Clone>(
    t: &[Word],
    area: [i32; 4],
    parse: impl Fn(&str) -> Option<T>,
) -> Option<T> {
    let mut v = vec![];
    for w in t.iter().filter(|w| w.inside(area)) {
        if let Some(n) = parse(&w.text)
            && !v.contains(&n)
        {
            v.push(n)
        }
    }
    (v.len() == 1).then(|| v[0].clone())
}
fn name(s: &str) -> Option<String> {
    let mut s = s.replace(['【', '［'], "[").replace(['】', '］'], "]");
    if ["游历]", "制造]", "售卖]"].iter().any(|p| s.starts_with(p)) {
        s.insert(0, '[');
    }
    let n = &s[s.find('[')?..];
    (n.contains(']') && n.chars().count() <= 64).then(|| n.to_owned())
}
fn counter(s: &str) -> Option<[u8; 2]> {
    let s = s.strip_prefix("刷新")?;
    let (a, b) = s.split_once('/')?;
    let [a, b] = [number(a)?, number(b)?];
    (b > 0 && b <= 50 && a <= b).then_some([a as u8, b as u8])
}
impl NativeSession {
    pub fn require_suibian_boo_box(&self) -> Result<()> {
        let root = crate::suibian_temple::validate_suibian_temple_bundle(
            self.suibian_temple_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        let m: Value =
            serde_json::from_slice(&std::fs::read(root.join("suibian-temple-manifest.json"))?)?;
        ensure!(m["schema_version"] == 7, "邦巢需要 schema 7 随便观资源");
        Ok(())
    }
    fn boo_box_sight(
        &self,
        frame: &crate::RecognitionImage,
        wanted: Option<u8>,
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
            timeout,
            started: Instant::now(),
            pipeline: od_core::suibian_boo_box_pipeline::pipeline(),
        };
        let result = (|| -> Result<Sight> {
            let mut bottom = words(&r.read("BooBox.Area.Text", Some([1170, 900, 600, 150]))?)?;
            let middle = words(&r.read("BooBox.Area.Text", Some([650, 410, 650, 370]))?)?;
            let modal = has(&middle, "确认", [650, 410, 1300, 780])
                || has(&middle, "取消", [650, 410, 1300, 780])
                || has(&middle, "获得", [650, 410, 1300, 780]);
            let page = !modal
                && point(&bottom, "聘用", [1500, 960, 1720, 1050]).is_some()
                && (unique(&bottom, [1200, 970, 1460, 1050], counter).is_some()
                    || has(&bottom, "次数用尽", [1200, 900, 1450, 1050]));
            let t = if page {
                bottom.extend(middle);
                for roi in [
                    [1560, 15, 185, 80],
                    [1150, 437, 642, 43],
                    [1150, 707, 642, 43],
                ] {
                    bottom.extend(words(&r.read("BooBox.Area.Text", Some(roi))?)?);
                }
                bottom
            } else {
                words(&r.read("BooBox.Area.Text", None)?)?
            };
            let mut points = BTreeMap::new();
            let capacity = has(&t, "无法聘用", [650, 420, 1300, 580])
                && has(&t, "上限", [650, 420, 1300, 580]);
            if capacity && let Some(p) = point(&t, "取消", [650, 580, 960, 710]) {
                points.insert("dismiss".into(), p);
                return Ok(Sight {
                    view: V::Rejection(BooOutcome::CapacityLimit),
                    points,
                });
            }
            if (has(&t, "云纹徽", [500, 350, 1450, 740]) && has(&t, "不足", [500, 350, 1450, 740])
                || has(&t, "货币不足", [500, 350, 1450, 740]))
                && let Some(p) = point(&t, "确认", [650, 580, 1300, 800])
            {
                points.insert("dismiss".into(), p);
                return Ok(Sight {
                    view: V::Rejection(BooOutcome::InsufficientFunds),
                    points,
                });
            }
            if has(&t, "获得", [800, 370, 1150, 470])
                && let Some(p) = point(&t, "确认", [760, 680, 1170, 800])
            {
                points.insert("reward".into(), p);
                let n = unique(&t, [650, 610, 1300, 670], name);
                let mut q = unique(&t, [800, 560, 1120, 615], number);
                if q.is_none() {
                    q = unique(
                        &words(&r.read("BooBox.Area.Quantity", None)?)?,
                        [936, 580, 984, 616],
                        number,
                    );
                }
                return Ok(Sight {
                    view: if let (Some(name), Some(quantity)) = (n, q) {
                        V::Reward { name, quantity }
                    } else {
                        V::Unknown
                    },
                    points,
                });
            }
            if point(&t, "确认", [550, 400, 1400, 900]).is_some() {
                return Ok(Sight {
                    view: V::Unknown,
                    points,
                });
            }
            if let Some(p) = point(&t, "跳过", [1680, 105, 1841, 162]) {
                points.insert("skip".into(), p);
                return Ok(Sight {
                    view: V::Animation { skip: true },
                    points,
                });
            }
            if has(&t, "邻里街坊", [1300, 900, 1650, 1000])
                && has(&t, "派驻邦布", [80, 720, 280, 820])
            {
                if let Some(p) = point(&t, "邻里街坊", [1300, 900, 1650, 1000]) {
                    points.insert("shops".into(), p);
                }
                return Ok(Sight {
                    view: V::Entry,
                    points,
                });
            }
            if has(&t, "好物铺", [500, 580, 1000, 760])
                && has(&t, "饮茶仙", [120, 580, 600, 760])
                && let Some(p) = point(&t, "邦巢", [1200, 580, 1800, 760])
            {
                points.insert("shop".into(), p);
                if let Some(p) = point(&t, "关闭", [700, 740, 1250, 850]) {
                    points.insert("close".into(), p);
                }
                return Ok(Sight {
                    view: V::Shops,
                    points,
                });
            }
            let hire = point(&t, "聘用", [1500, 960, 1720, 1050]);
            let count = unique(&t, [1200, 970, 1460, 1050], counter);
            let exhausted = has(&t, "次数用尽", [1200, 900, 1450, 1050]);
            if let Some(hire) = hire
                && (count.is_some() || exhausted)
            {
                points.insert("hire".into(), hire);
                let balance = unique(&t, [1560, 15, 1745, 95], number);
                let mut offers = vec![];
                for slot in 1..=6u8 {
                    let col = i32::from((slot - 1) % 3);
                    let row = i32::from((slot - 1) / 3);
                    let x = 1145 + col * 218;
                    let y = 440 + row * 269;
                    let price = unique(&t, [x, y, x + 210, y + 42], number)
                        .filter(|v| *v > 0 && *v <= 40000);
                    let mut sold_out = has(&t, "已聘用", [x, y - 210, x + 210, y + 42])
                        || has(&t, "售罄", [x, y - 210, x + 210, y + 42]);
                    if price.is_none() && !sold_out {
                        let card =
                            words(&r.read("BooBox.Area.Text", Some([x, y - 210, 210, 252]))?)?;
                        sold_out = ["已聘用", "售罄", "EMPTY"]
                            .iter()
                            .any(|s| has(&card, s, [x, y - 210, x + 210, y + 42]));
                    }
                    points.insert(format!("card:{slot}"), [x + 105, y - 130]);
                    offers.push(BooOffer {
                        slot,
                        price,
                        sold_out,
                    });
                }
                let list_readable = offers.iter().all(|o| o.price.is_some() || o.sold_out);
                let mut selected = None;
                if let Some(slot) = wanted.filter(|s| (1..=6).contains(s)) {
                    let col = i32::from((slot - 1) % 3);
                    let row = i32::from((slot - 1) / 3);
                    if r.hit(
                        "BooBox.Area.Selected",
                        Some([1148 + col * 218, 250 + row * 269, 8, 180]),
                    )? {
                        let title = words(&r.read("BooBox.Area.Text", Some([580, 125, 468, 74]))?)?;
                        let n = unique(&title, [580, 125, 1048, 199], name);
                        let p = unique(&t, [1510, 915, 1710, 965], number);
                        if let (Some(name), Some(price)) = (n, p) {
                            selected = Some(BooSelection {
                                slot,
                                name,
                                price,
                                hire: true,
                            });
                        }
                    }
                }
                let cost = unique(&t, [1240, 915, 1450, 965], number);
                let refresh = if let Some(counter) = count {
                    points.insert("refresh".into(), [1310, 1000]);
                    Some(BooRefresh {
                        counter,
                        cost,
                        active: !exhausted && cost == Some(200),
                    })
                } else {
                    None
                };
                return Ok(Sight {
                    view: V::Page(BooPage {
                        balance,
                        offers,
                        list_readable,
                        selected,
                        refresh,
                        exhausted,
                    }),
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
    pub fn read_suibian_boo_box(
        &self,
        wanted: Option<u8>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        let started = Instant::now();
        self.require_suibian_boo_box()?;
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        Ok(self
            .boo_box_sight(
                &frame.image,
                wanted,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            .filter(|_| frame.age() <= Duration::from_millis(1500))
            .map(|s| s.view))
    }
    pub fn act_suibian_boo_box(
        &mut self,
        flow: &TempleBooBox,
        a: &A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<BooInputEnd> {
        let started = Instant::now();
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_suibian_boo_box()?;
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "邦巢控制器模式不匹配"
        );
        let result = (|| -> Result<BooInputEnd> {
            if cancel.load(Ordering::Acquire) {
                return Ok(BooInputEnd::NotPosted);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(BooInputEnd::NotPosted);
            };
            let Some(s) = self.boo_box_sight(
                &frame.image,
                flow.target.as_ref().map(|t| t.slot),
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            else {
                return Ok(BooInputEnd::NotPosted);
            };
            if started.elapsed() >= timeout {
                return Ok(BooInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) || !flow.allows(a, &s.view) {
                return Ok(BooInputEnd::Changed);
            }
            let p = match a {
                A::OpenShops => s.points.get("shops").copied(),
                A::OpenShop => s.points.get("shop").copied(),
                A::Select(t) => s.points.get(&format!("card:{}", t.slot)).copied(),
                A::Spend(BooSpend::Hire { .. }) => s.points.get("hire").copied(),
                A::Spend(BooSpend::Refresh { .. }) => s.points.get("refresh").copied(),
                A::RevealSkip => Some([1680, 105]),
                A::Skip => s.points.get("skip").copied(),
                A::ConfirmReward => s.points.get("reward").copied(),
                A::DismissRejection => s.points.get("dismiss").copied(),
                A::Back => {
                    if matches!(s.view, V::Shops) {
                        s.points.get("close").copied()
                    } else {
                        Some([116, 52])
                    }
                }
            };
            let Some([x, y]) = p.filter(|[x, y]| (1..1920).contains(x) && (1..1080).contains(y))
            else {
                return Ok(BooInputEnd::Changed);
            };
            if started.elapsed() >= timeout {
                return Ok(BooInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) {
                return Ok(BooInputEnd::Changed);
            }
            let input = |x, y| od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input(x, y), true, cancel)? {
                return Ok(BooInputEnd::NotPosted);
            }
            let moving = matches!(a, A::RevealSkip);
            let hold = if moving { 200 } else { 100 };
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(hold)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                if moving {
                    self.combat_input(
                        input(
                            x + (held.elapsed().as_millis().min(200) * 80 / 200) as i32,
                            y + (held.elapsed().as_millis().min(200) * 28 / 200) as i32,
                        ),
                        true,
                        cancel,
                    )?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input(x, y), false, &AtomicBool::new(false))?;
            Ok(BooInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|v| release.map(|_| v))
    }
}
