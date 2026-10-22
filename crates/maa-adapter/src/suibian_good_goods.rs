//! 好物铺菜单、数量、余额和确认均在本次输入前重新核对。
pub use crate::suibian_adventure::AdventureInputEnd as GoodInputEnd;
use crate::{
    NativeSession,
    suibian_adventure::{Reader, Word, has, point, words},
};
use anyhow::{Result, ensure};
use od_core::suibian_good_goods::{
    ENERGY_PLUGIN, GoodAction as A, GoodExchange, GoodOffer, GoodOutcome, GoodPage, GoodView as V,
    TempleGoodGoods,
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
fn number(t: &str) -> Option<u32> {
    let t = t.replace(',', "");
    (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
        .then(|| t.parse().ok())
        .flatten()
}
fn pair(t: &str) -> Option<(u32, u32)> {
    let (a, b) = t.split_once('/')?;
    Some((number(a)?, number(b)?))
}
fn quantity(t: &str, prefix: &str) -> Option<u32> {
    number(
        t.strip_prefix(prefix)?
            .trim_start_matches(['×', 'x', 'X', ':', '：']),
    )
}
fn unique<T: Eq + Clone>(
    t: &[Word],
    area: [i32; 4],
    parse: impl Fn(&str) -> Option<T>,
) -> Option<T> {
    let mut values = vec![];
    for w in t.iter().filter(|w| w.inside(area)) {
        if let Some(v) = parse(&w.text)
            && !values.contains(&v)
        {
            values.push(v);
        }
    }
    (values.len() == 1).then(|| values[0].clone())
}
fn price(t: &str) -> Option<u32> {
    let normalized = if ["5oo", "50o", "s00", "soo"].contains(&t.to_ascii_lowercase().as_str()) {
        "500"
    } else {
        t
    };
    number(normalized).filter(|n| (100..=1_000_000_000).contains(n))
}
impl NativeSession {
    pub fn require_suibian_good_goods(&self) -> Result<()> {
        let root = crate::suibian_temple::validate_suibian_temple_bundle(
            self.suibian_temple_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        let m: Value =
            serde_json::from_slice(&std::fs::read(root.join("suibian-temple-manifest.json"))?)?;
        ensure!(
            matches!(m["schema_version"].as_u64(), Some(6..=8)),
            "好物铺需要 schema 6 随便观资源"
        );
        Ok(())
    }
    fn good_goods_sight(
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
            timeout,
            started: Instant::now(),
            pipeline: od_core::suibian_good_goods_pipeline::pipeline(),
        };
        let result = (|| -> Result<Sight> {
            let t = words(&r.read("GoodGoods.Area.Text", None)?)?;
            let mut points = BTreeMap::new();
            let rejected = if has(&t, "背包", [400, 300, 1500, 750])
                && has(&t, "上限", [400, 300, 1500, 750])
            {
                Some(GoodOutcome::CapacityLimit)
            } else if (has(&t, "云纹徽", [400, 300, 1500, 750])
                && has(&t, "不足", [400, 300, 1500, 750]))
                || has(&t, "货币不足", [400, 300, 1500, 750])
            {
                Some(GoodOutcome::InsufficientFunds)
            } else {
                None
            };
            if let Some(reason) = rejected
                && let Some(p) = point(&t, "确认", [550, 520, 1400, 850])
            {
                points.insert("dismiss".into(), p);
                return Ok(Sight {
                    view: V::Rejection(reason),
                    points,
                });
            }
            if has(&t, "兑换确认", [550, 270, 950, 355]) {
                let name = unique(&t, [770, 420, 1210, 475], |s| {
                    s.starts_with(ENERGY_PLUGIN)
                        .then(|| ENERGY_PLUGIN.to_owned())
                });
                let qty = unique(&t, [850, 580, 1140, 640], |s| quantity(s, "兑换数量"));
                let limit = unique(&t, [860, 680, 1140, 730], |s| {
                    quantity(s, "日剩余").or_else(|| quantity(s, "剩余"))
                });
                let mut payment = unique(&t, [730, 735, 1010, 800], pair);
                if payment.is_none() {
                    let cropped = words(&r.read("GoodGoods.Area.Payment", None)?)?;
                    payment = unique(&cropped, [750, 735, 1010, 800], pair);
                }
                if let Some(p) = point(&t, "确认", [980, 730, 1280, 810]) {
                    points.insert("purchase".into(), p);
                }
                let view = if let (Some(name), Some(quantity), Some(limit), Some((balance, cost))) =
                    (name, qty, limit, payment)
                {
                    V::Exchange(GoodExchange {
                        name,
                        quantity,
                        limit,
                        balance,
                        cost,
                        confirm: points.contains_key("purchase"),
                    })
                } else {
                    V::Unknown
                };
                return Ok(Sight { view, points });
            }
            if has(&t, "获得", [800, 370, 1150, 470])
                && let Some(p) = point(&t, "确认", [760, 680, 1170, 800])
            {
                points.insert("reward".into(), p);
                let name = unique(&t, [650, 600, 1300, 670], |s| {
                    s.contains(ENERGY_PLUGIN).then(|| ENERGY_PLUGIN.to_owned())
                });
                let qty = unique(&t, [800, 560, 1120, 620], |s| {
                    number(s.trim_start_matches(['×', 'x', 'X']))
                });
                return Ok(Sight {
                    view: if let (Some(name), Some(quantity)) = (name, qty) {
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
            if has(&t, "好物铺", [650, 640, 900, 730])
                && has(&t, "邦巢", [1400, 640, 1620, 730])
                && has(&t, "饮茶仙", [300, 640, 560, 730])
            {
                if let Some(p) = point(&t, "好物铺", [650, 640, 900, 730]) {
                    points.insert("shop".into(), p);
                }
                if let Some(p) = point(&t, "关闭", [800, 740, 1130, 830]) {
                    points.insert("close".into(), p);
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
            if has(&t, "经营购置", [90, 680, 320, 765])
                && has(&t, "采集兑换", [350, 680, 1050, 765])
            {
                let business = r.hit("GoodGoods.Area.Business", None)?;
                if let Some(p) = point(&t, "经营购置", [90, 680, 320, 765]) {
                    points.insert("business".into(), p);
                }
                let balance = unique(&t, [1560, 15, 1745, 95], number);
                let mut offers = vec![];
                let mut readable = [false; 9];
                for (slot, known) in readable.iter_mut().enumerate() {
                    let left = 100 + 193 * slot as i32;
                    let area = [left, 795, left + 180, 1055];
                    let header = [left, 795, left + 180, 850];
                    let target = t
                        .iter()
                        .find(|w| w.inside(header) && w.text.contains(ENERGY_PLUGIN));
                    *known =
                        target.is_some() || has(&t, "EMPTY", header) || has(&t, "货架", header);
                    if let Some(word) = target {
                        points.insert(format!("good:{}", slot + 1), word.center());
                        offers.push(GoodOffer {
                            slot: slot as u8 + 1,
                            unit_price: unique(&t, [left, 1005, left + 180, 1055], price),
                            sold_out: has(&t, "已售罄", area) || has(&t, "售罄", area),
                        });
                    }
                }
                let required = offers
                    .iter()
                    .map(|o| usize::from(o.slot))
                    .min()
                    .unwrap_or(9);
                let list_readable = readable[..required].iter().all(|v| *v);
                return Ok(Sight {
                    view: V::Page(GoodPage {
                        business,
                        balance,
                        offers,
                        list_readable,
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
    pub fn read_suibian_good_goods(
        &self,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        let started = Instant::now();
        self.require_suibian_good_goods()?;
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        Ok(self
            .good_goods_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            .filter(|_| frame.age() <= Duration::from_millis(1500))
            .map(|s| s.view))
    }
    pub fn act_suibian_good_goods(
        &mut self,
        flow: &TempleGoodGoods,
        a: &A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<GoodInputEnd> {
        let started = Instant::now();
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_suibian_good_goods()?;
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "好物铺控制器模式不匹配"
        );
        let result = (|| -> Result<GoodInputEnd> {
            if cancel.load(Ordering::Acquire) {
                return Ok(GoodInputEnd::NotPosted);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(GoodInputEnd::NotPosted);
            };
            let Some(s) = self.good_goods_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            else {
                return Ok(GoodInputEnd::NotPosted);
            };
            if started.elapsed() >= timeout {
                return Ok(GoodInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) || !flow.allows(a, &s.view) {
                return Ok(GoodInputEnd::Changed);
            }
            let p = match a {
                A::OpenShops => s.points.get("shops").copied(),
                A::OpenShop => s.points.get("shop").copied(),
                A::BusinessTab => s.points.get("business").copied(),
                A::OpenGood(t) => s.points.get(&format!("good:{}", t.slot)).copied(),
                A::MaxQuantity(_) => Some([755, 672]),
                A::Purchase(_) => s.points.get("purchase").copied(),
                A::ConfirmReward => s.points.get("reward").copied(),
                A::DismissRejection => s.points.get("dismiss").copied(),
                A::Back => Some(if matches!(s.view, V::Shops) {
                    *s.points.get("close").unwrap_or(&[0, 0])
                } else {
                    [116, 52]
                }),
            };
            let Some([x, y]) = p.filter(|[x, y]| (1..1920).contains(x) && (1..1080).contains(y))
            else {
                return Ok(GoodInputEnd::Changed);
            };
            if started.elapsed() >= timeout {
                return Ok(GoodInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) {
                return Ok(GoodInputEnd::Changed);
            }
            let input = |x, y| od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input(x, y), true, cancel)? {
                return Ok(GoodInputEnd::NotPosted);
            }
            let moving = matches!(a, A::MaxQuantity(_));
            let hold = if moving { 2000 } else { 100 };
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(hold)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                if moving {
                    self.combat_input(
                        input(
                            x + (held.elapsed().as_millis().min(2000) * 545 / 2000) as i32,
                            y,
                        ),
                        true,
                        cancel,
                    )?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input(x, y), false, &AtomicBool::new(false))?;
            Ok(GoodInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|v| release.map(|_| v))
    }
}
