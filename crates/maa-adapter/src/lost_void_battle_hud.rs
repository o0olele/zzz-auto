//! 正常战斗 HUD 的完整主战身份；固定前台 / 后台区域，不接受紧凑探索头像。
use crate::{
    NativeSession,
    combat_vision::{CombatFrame, CombatRecognizer},
};
use anyhow::{Context, Result, ensure};
use od_core::lost_void_roster::{RosterMember, validate_members};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

struct Source {
    member: RosterMember,
    template: String,
    mask: Option<Vec<u8>>,
    width: u32,
    height: u32,
    front: bool,
}
struct Slot {
    source: usize,
    rect: [i32; 4],
    pixels: Vec<u8>,
}
#[derive(Debug, Clone, Serialize)]
pub struct BattleHudMatch {
    pub slot: u8,
    pub member: RosterMember,
    pub template: String,
    pub rect: [i32; 4],
    pub score: f64,
}
pub struct LostVoidBattleHudReader {
    root: PathBuf,
    members: Vec<RosterMember>,
    areas: Vec<[i32; 4]>,
    sources: Vec<Source>,
    slots: Vec<Slot>,
    diagnostics: Vec<BattleHudMatch>,
}
impl LostVoidBattleHudReader {
    /// 名单来自父任务完整实际队伍回执，不能从未命中的后台推断人数。
    pub fn new(recognizer: &CombatRecognizer, ids: &[String]) -> Result<Self> {
        let bundle = &recognizer.bundle;
        let members: Vec<_> = ids
            .iter()
            .map(|id| {
                let a = bundle
                    .spec
                    .agents
                    .iter()
                    .find(|a| &a.id == id)
                    .context("战斗 HUD 名单不在冻结目录")?;
                Ok(RosterMember {
                    id: a.id.clone(),
                    kind: a.kind.clone(),
                })
            })
            .collect::<Result<_>>()?;
        validate_members(&members, members.len() as u8).map_err(anyhow::Error::msg)?;
        let names: &[&str] = match members.len() {
            1 => &["头像-3-1"],
            2 => &["头像-3-1", "头像-2-2"],
            _ => &["头像-3-1", "头像-3-2", "头像-3-3"],
        };
        let areas = names
            .iter()
            .map(|name| {
                bundle
                    .spec
                    .areas
                    .get(*name)
                    .copied()
                    .context("战斗 HUD 缺少来源区域")
            })
            .collect::<Result<_>>()?;
        let mut sources = vec![];
        for member in &members {
            let a = bundle
                .spec
                .agents
                .iter()
                .find(|a| a.id == member.id)
                .unwrap();
            for front in [true, false] {
                for skin in &a.templates {
                    let key = format!("battle/avatar_{}_{skin}", if front { 1 } else { 2 });
                    let t = bundle
                        .spec
                        .templates
                        .get(&key)
                        .context("战斗 HUD 缺少头像变体")?;
                    let size = image::image_dimensions(bundle.root.join(&t.image))?;
                    let mask = t
                        .mask
                        .as_ref()
                        .map(|p| image::open(bundle.root.join(p)).map(|p| p.to_luma8().into_raw()))
                        .transpose()?;
                    sources.push(Source {
                        member: member.clone(),
                        template: t
                            .image
                            .strip_prefix("image/")
                            .context("头像不在 image 目录")?
                            .into(),
                        mask,
                        width: size.0,
                        height: size.1,
                        front,
                    });
                }
            }
        }
        Ok(Self {
            root: bundle.root.clone(),
            members,
            areas,
            sources,
            slots: vec![],
            diagnostics: vec![],
        })
    }
    pub fn reset(&mut self) {
        self.slots.clear();
        self.diagnostics.clear();
    }
    pub fn diagnostics(&self) -> &[BattleHudMatch] {
        &self.diagnostics
    }
    fn fingerprint(&self, frame: &CombatFrame, source: usize, rect: [i32; 4]) -> Result<Vec<u8>> {
        let s = &self.sources[source];
        ensure!(
            frame.image.buffer.width() == 1920
                && frame.image.buffer.height() == 1080
                && frame.image.buffer.image_type() == 16,
            "战斗 HUD 须为 1080p BGR"
        );
        ensure!(
            od_core::intel_board::valid_rect(rect)
                && rect[2] == s.width as i32
                && rect[3] == s.height as i32,
            "战斗 HUD 匹配范围无效"
        );
        let bgr = frame.image.buffer.raw_data().context("战斗 HUD 没有像素")?;
        ensure!(bgr.len() == 1920 * 1080 * 3, "战斗 HUD 像素长度无效");
        let mut h = Sha256::new();
        for y in 0..s.height as usize {
            for x in 0..s.width as usize {
                if s.mask
                    .as_ref()
                    .is_none_or(|m| m[y * s.width as usize + x] > 0)
                {
                    let i = ((rect[1] as usize + y) * 1920 + rect[0] as usize + x) * 3;
                    h.update(&bgr[i..i + 3]);
                }
            }
        }
        Ok(h.finalize().to_vec())
    }
    fn cached(&self, frame: &CombatFrame) -> Result<Option<Vec<RosterMember>>> {
        if self.slots.len() != self.members.len() {
            return Ok(None);
        }
        for slot in &self.slots {
            if self.fingerprint(frame, slot.source, slot.rect)? != slot.pixels {
                return Ok(None);
            }
        }
        Ok(Some(
            self.slots
                .iter()
                .map(|s| self.sources[s.source].member.clone())
                .collect(),
        ))
    }
    fn discover(
        &mut self,
        session: &NativeSession,
        frame: &CombatFrame,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Vec<RosterMember>>> {
        self.reset();
        let started = Instant::now();
        let mut chosen: Vec<(usize, [i32; 4], f64)> = vec![];
        for (slot, area) in self.areas.iter().enumerate() {
            let mut by_id = std::collections::BTreeMap::<String, (usize, [i32; 4], f64)>::new();
            for (index, s) in self
                .sources
                .iter()
                .enumerate()
                .filter(|(_, s)| s.front == (slot == 0))
            {
                if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
                    return Ok(None);
                }
                let roi = [area[0], area[1], area[2] - area[0], area[3] - area[1]];
                let found=session.recognize_prepared(&frame.image,"TemplateMatch",&json!({"roi":roi,"template":s.template,"threshold":0.8,"method":5,"green_mask":s.mask.is_some(),"order_by":"Score"}))?;
                if found.detail["hit"] != true {
                    continue;
                }
                let rect: [i32; 4] =
                    serde_json::from_value(found.detail["detail"]["best"]["box"].clone())?;
                let score = found.detail["detail"]["best"]["score"]
                    .as_f64()
                    .context("战斗 HUD 缺少得分")?;
                ensure!(score.is_finite() && score >= 0.8, "战斗 HUD 得分无效");
                self.diagnostics.push(BattleHudMatch {
                    slot: slot as u8,
                    member: s.member.clone(),
                    template: s.template.clone(),
                    rect,
                    score,
                });
                if by_id.get(&s.member.id).is_none_or(|h| score > h.2) {
                    by_id.insert(s.member.id.clone(), (index, rect, score));
                }
            }
            let mut hits: Vec<_> = by_id.into_values().collect();
            hits.sort_by(|a, b| b.2.total_cmp(&a.2));
            let Some(best) = hits.first().copied() else {
                return Ok(None);
            };
            if hits
                .get(1)
                .is_some_and(|other| (best.2 - other.2).abs() < 1e-6)
            {
                return Ok(None);
            }
            if chosen
                .iter()
                .any(|(s, _, _)| self.sources[*s].member.id == self.sources[best.0].member.id)
            {
                return Ok(None);
            }
            chosen.push(best);
        }
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let actual: Vec<_> = chosen
            .iter()
            .map(|(i, _, _)| self.sources[*i].member.clone())
            .collect();
        validate_members(&actual, self.members.len() as u8).map_err(anyhow::Error::msg)?;
        for (source, rect, _) in chosen {
            self.slots.push(Slot {
                source,
                rect,
                pixels: self.fingerprint(frame, source, rect)?,
            });
        }
        Ok(Some(actual))
    }
}
impl NativeSession {
    /// 初次发现只读；执行守卫应在新帧用有效像素复核，而不是放宽战斗帧龄。
    pub fn inspect_lost_void_battle_hud(
        &self,
        frame: &CombatFrame,
        reader: &mut LostVoidBattleHudReader,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Vec<RosterMember>>> {
        ensure!(
            self.combat_bundle.as_deref() == Some(reader.root.as_path()),
            "正常 HUD 与当前战斗资源不一致"
        );
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let started = Instant::now();
        let members = if let Some(cached) = reader.cached(frame)? {
            Some(cached)
        } else {
            reader.discover(
                self,
                frame,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?
        };
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        Ok(members)
    }
    /// 无发现循环的每帧复核。变化返回未知，禁止用旧匹配开放攻击。
    pub fn verify_lost_void_battle_hud(
        &self,
        frame: &CombatFrame,
        reader: &LostVoidBattleHudReader,
        cancel: &AtomicBool,
        max_frame_age_ms: u32,
    ) -> Result<Option<Vec<RosterMember>>> {
        ensure!(
            self.combat_bundle.as_deref() == Some(reader.root.as_path()),
            "正常 HUD 与当前战斗资源不一致"
        );
        ensure!(
            (1..=1000).contains(&max_frame_age_ms),
            "正常 HUD 帧龄限制无效"
        );
        if cancel.load(Ordering::Acquire) || frame.age_ms() > max_frame_age_ms {
            return Ok(None);
        }
        let members = reader.cached(frame)?;
        if cancel.load(Ordering::Acquire) || frame.age_ms() > max_frame_age_ms {
            return Ok(None);
        }
        Ok(members)
    }
}
