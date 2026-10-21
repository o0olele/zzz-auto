//! 售卖动态坐标仅使用本次输入前的同一帧；不缓存旧商品或邦布选择。
pub use crate::suibian_adventure::AdventureInputEnd as SalesInputEnd;
use crate::{
    NativeSession,
    suibian_adventure::{Reader, Word, has, point, words},
    suibian_craft::BANGBOOS,
};
use anyhow::{Result, ensure};
use od_core::{
    suibian_craft::goods_key,
    suibian_sales::{
        SalesAction as A, SalesBoo, SalesPicker, SalesSelection, SalesShelf, SalesShelfState,
        SalesView as V, TempleSales,
    },
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
fn unique(
    texts: &[Word],
    area: [i32; 4],
    parse: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let mut v: Vec<_> = texts
        .iter()
        .filter(|w| w.inside(area))
        .filter_map(|w| parse(&w.text))
        .collect();
    v.sort();
    v.dedup();
    (v.len() == 1).then(|| v[0].clone())
}
fn energy(t: &str) -> Option<(u32, u32)> {
    let (a, b) = t.split_once('/')?;
    let a = a.replace(',', "").parse().ok()?;
    let b = b.replace(',', "").parse().ok()?;
    (a <= b && b > 0 && b <= 1_000_000).then_some((a, b))
}
fn stock(t: &str) -> Option<u32> {
    let t = t.strip_prefix("库存")?;
    let n = t.replace(',', "").parse().ok()?;
    (n <= 1_000_000_000).then_some(n)
}
fn product(t: &str) -> Option<String> {
    t.starts_with(['[', '［', '【'])
        .then(|| goods_key(t))
        .flatten()
}
impl NativeSession {
    pub fn require_suibian_sales(&self) -> Result<()> {
        let root = crate::suibian_temple::validate_suibian_temple_bundle(
            self.suibian_temple_bundle
                .as_deref()
                .unwrap_or(&self.resource_root),
        )?;
        let m: Value =
            serde_json::from_slice(&std::fs::read(root.join("suibian-temple-manifest.json"))?)?;
        ensure!(
            matches!(m["schema_version"].as_u64(), Some(5..=7)),
            "售卖需要 schema 5 随便观资源"
        );
        Ok(())
    }
    fn sales_sight(
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
            pipeline: od_core::suibian_sales_pipeline::pipeline(),
        };
        let result = (|| -> Result<Sight> {
            let t = words(&r.read("Sales.Area.Text", None)?)?;
            let mut points = BTreeMap::new();
            if has(&t, "确认", [550, 400, 1400, 900])
                || has(&t, "功能暂时无法进入", [450, 300, 1450, 750])
            {
                return Ok(Sight {
                    view: if has(&t, "正在自动托管", [450, 300, 1450, 750]) {
                        V::Managed
                    } else {
                        V::Unknown
                    },
                    points,
                });
            }
            if has(&t, "邻里街坊", [1300, 900, 1650, 1000])
                && has(&t, "派驻邦布", [80, 720, 280, 820])
            {
                return Ok(Sight {
                    view: if has(&t, "托管中", [80, 940, 280, 1040])
                        || has(&t, "自动托管中", [1200, 630, 1740, 790])
                    {
                        V::Managed
                    } else if has(&t, "经营", [1240, 570, 1430, 650]) {
                        V::Entry
                    } else {
                        V::Unknown
                    },
                    points,
                });
            }
            if !r.hit("Hosting.Area.Back", None)? {
                return Ok(Sight {
                    view: V::Unknown,
                    points,
                });
            }
            if has(&t, "邦布售卖增益", [1260, 470, 1790, 550])
                && has(&t, "派驻邦布", [110, 850, 500, 965])
            {
                let rect = BANGBOOS[1];
                let area = [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]];
                let energies: Vec<_> = t
                    .iter()
                    .filter(|w| w.inside(area))
                    .filter_map(|w| energy(&w.text))
                    .collect();
                let en = (energies.len() == 1).then(|| energies[0]);
                let second = if ["制造中", "游历中", "售卖中"]
                    .into_iter()
                    .any(|n| has(&t, n, area))
                {
                    SalesBoo::Busy
                } else if has(&t, "经营", area) {
                    match en {
                        Some((0, _)) => SalesBoo::Exhausted,
                        Some(_) => SalesBoo::Available,
                        None => SalesBoo::Unknown,
                    }
                } else {
                    SalesBoo::Unknown
                };
                let mut selected = vec![];
                for (i, rect) in BANGBOOS.into_iter().take(2).enumerate() {
                    if r.hit("Sales.Area.Selected", Some(rect))? {
                        selected.push(i as u8 + 1);
                    }
                }
                let name = unique(&t, [1470, 180, 1820, 265], |s| {
                    s.starts_with(['[', '［', '【']).then(|| s.to_owned())
                });
                let assign = point(&t, "确认派驻", [1300, 860, 1780, 980]);
                if let Some(p) = assign {
                    points.insert("assign".into(), p);
                }
                let assigned = has(&t, "取消派驻", [1300, 860, 1780, 980]);
                return Ok(Sight {
                    view: V::Picker(SalesPicker {
                        second,
                        selected: if selected.len() == 1 {
                            Some(selected[0])
                        } else {
                            None
                        },
                        name,
                        energy: en,
                        assigned,
                        can_assign: assign.is_some() && !assigned,
                    }),
                    points,
                });
            }
            let slot = unique(&t, [150, 10, 400, 100], |s| {
                let n = s.strip_suffix("号货架")?.parse::<u8>().ok()?;
                (1..=4).contains(&n).then(|| n.to_string())
            })
            .and_then(|n| n.parse().ok());
            if let Some(slot) = slot {
                let name = unique(&t, [1410, 225, 1810, 290], product);
                let inventory = unique(&t, [1320, 750, 1750, 840], |s| {
                    stock(s).map(|n| n.to_string())
                })
                .and_then(|n| n.parse().ok());
                let shortage = has(&t, "库存不足", [1240, 750, 1780, 840]);
                let start = point(&t, "开始售卖", [1300, 920, 1720, 1035]);
                let cancel_point = point(&t, "取消售卖", [1300, 920, 1720, 1035]);
                if let Some(p) = start {
                    points.insert("start".into(), p);
                }
                if let Some(p) = cancel_point {
                    points.insert("cancel".into(), p);
                }
                if let Some(goods) = name {
                    return Ok(Sight {
                        view: V::Goods(SalesSelection {
                            slot,
                            goods,
                            stock: if shortage && inventory.is_none() {
                                Some(0)
                            } else {
                                inventory
                            },
                            shortage,
                            can_start: start.is_some() && cancel_point.is_none(),
                            can_cancel: cancel_point.is_some() && start.is_none(),
                        }),
                        points,
                    });
                }
                return Ok(Sight {
                    view: V::Unknown,
                    points,
                });
            }
            if has(&t, "售卖铺", [1380, 470, 1630, 550])
                && has(&t, "邦掌柜", [1680, 840, 1890, 940])
            {
                let mut shelves = vec![];
                for (i, x) in [440, 785, 1128, 1470].into_iter().enumerate() {
                    let area = [x - 155, 650, x + 155, 1045];
                    let empty = point(&t, "开始售卖", area);
                    let shortage = point(&t, "库存不足", area);
                    let selling = point(&t, "售卖中", area);
                    let locked = has(&t, "未解锁", area);
                    let states = usize::from(empty.is_some())
                        + usize::from(shortage.is_some())
                        + usize::from(selling.is_some())
                        + usize::from(locked);
                    let state = if states != 1 {
                        SalesShelfState::Unknown
                    } else if empty.is_some() {
                        SalesShelfState::Empty
                    } else if shortage.is_some() {
                        SalesShelfState::Shortage
                    } else if selling.is_some() {
                        SalesShelfState::Selling
                    } else {
                        SalesShelfState::Locked
                    };
                    let goods = unique(&t, [area[0], area[1], area[2], 940], product);
                    if let Some(p) = shortage.or(empty).or(selling) {
                        points.insert(format!("shelf:{}", i + 1), p);
                    } else if state != SalesShelfState::Unknown {
                        points.insert(format!("shelf:{}", i + 1), [x, 850]);
                    }
                    shelves.push(SalesShelf {
                        slot: i as u8 + 1,
                        state,
                        goods,
                    });
                }
                return Ok(Sight {
                    view: V::Main(shelves),
                    points,
                });
            }
            if has(&t, "开物清单", [280, 675, 650, 775])
                && has(&t, "制造台", [800, 675, 1130, 775])
                && has(&t, "货架", [1360, 675, 1640, 775])
            {
                if let Some(p) = point(&t, "售卖", [1250, 180, 1700, 350]) {
                    points.insert("sales".into(), p);
                }
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
    pub fn read_suibian_sales(&self, cancel: &AtomicBool, timeout: Duration) -> Result<Option<V>> {
        let started = Instant::now();
        self.require_suibian_sales()?;
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        Ok(self
            .sales_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            .filter(|_| frame.age() <= Duration::from_millis(1500))
            .map(|s| s.view))
    }
    pub fn act_suibian_sales(
        &mut self,
        flow: &TempleSales,
        a: &A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<SalesInputEnd> {
        let started = Instant::now();
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_suibian_sales()?;
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "售卖控制器模式不匹配"
        );
        let result = (|| -> Result<SalesInputEnd> {
            if cancel.load(Ordering::Acquire) {
                return Ok(SalesInputEnd::NotPosted);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(SalesInputEnd::NotPosted);
            };
            let Some(s) = self.sales_sight(
                &frame.image,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
            else {
                return Ok(SalesInputEnd::NotPosted);
            };
            if started.elapsed() >= timeout {
                return Ok(SalesInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) || !flow.allows(a, &s.view) {
                return Ok(SalesInputEnd::Changed);
            }
            let p = match a {
                A::OpenManagement => Some([1320, 605]),
                A::OpenSales => s.points.get("sales").copied(),
                A::OpenPicker => Some([1789, 945]),
                A::SelectSecond => Some([519, 345]),
                A::OpenShelf(n) => s.points.get(&format!("shelf:{n}")).copied(),
                A::Assign(_) => s.points.get("assign").copied(),
                A::Cancel(_) => s.points.get("cancel").copied(),
                A::Start(_) => s.points.get("start").copied(),
                A::Back => Some([116, 52]),
            };
            let Some([x, y]) = p.filter(|[x, y]| (1..1920).contains(x) && (1..1080).contains(y))
            else {
                return Ok(SalesInputEnd::Changed);
            };
            if started.elapsed() >= timeout {
                return Ok(SalesInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) {
                return Ok(SalesInputEnd::Changed);
            }
            let input = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(SalesInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(100)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input, false, &AtomicBool::new(false))?;
            Ok(SalesInputEnd::Posted)
        })();
        let release = self.release_inputs();
        result.and_then(|v| release.map(|_| v))
    }
}
