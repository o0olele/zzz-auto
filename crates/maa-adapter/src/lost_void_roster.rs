//! 紧凑及普通 HUD 的完整头像发现，新帧有效像素复核和冻结切人输入。
use crate::{
    NativeSession,
    combat_vision::{CombatBundle, CombatFrame, CombatRecognizer},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    lost_void_roster::{RosterAction, RosterFlow, RosterMember, RosterView},
    lost_void_scene::LostVoidScreen as S,
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize)]
pub struct HudCandidate {
    pub member: RosterMember,
    pub rect: [i32; 4],
    pub score: f64,
    pub template: String,
}
struct AvatarSource {
    member: RosterMember,
    template: String,
    mask: Option<Vec<u8>>,
    width: u32,
    height: u32,
}
struct HudSlot {
    source: usize,
    rect: [i32; 4],
    pixels: Vec<u8>,
}
pub struct LostVoidHudReader {
    root: PathBuf,
    roi: [i32; 4],
    sources: Vec<AvatarSource>,
    cache: Vec<HudSlot>,
    diagnostics: Vec<HudCandidate>,
}
impl LostVoidHudReader {
    pub fn new(recognizer: &CombatRecognizer) -> Result<Self> {
        let bundle = &recognizer.bundle;
        let areas = ["头像-3-1", "头像-3-2", "头像-3-3", "头像-2-2"]
            .into_iter()
            .map(|n| {
                bundle
                    .spec
                    .areas
                    .get(n)
                    .copied()
                    .context("HUD 缺少固定头像区域")
            })
            .collect::<Result<Vec<_>>>()?;
        let left = areas.iter().map(|r| r[0]).min().unwrap();
        let top = areas.iter().map(|r| r[1]).min().unwrap();
        let right = areas.iter().map(|r| r[2]).max().unwrap();
        let bottom = areas.iter().map(|r| r[3]).max().unwrap();
        let roi = [left, top, right - left, bottom - top];
        ensure!(od_core::intel_board::valid_rect(roi), "HUD 并集区域无效");
        let mut sources = vec![];
        for agent in &bundle.spec.agents {
            let member = RosterMember {
                id: agent.id.clone(),
                kind: agent.kind.clone(),
            };
            member.validate().map_err(anyhow::Error::msg)?;
            for name in &agent.templates {
                for prefix in ["avatar_1_", "avatar_2_"] {
                    let t = bundle
                        .spec
                        .templates
                        .get(&format!("battle/{prefix}{name}"))
                        .context("完整 HUD 头像资源缺失")?;
                    let image = image::open(bundle.root.join(&t.image))?;
                    let mask = t
                        .mask
                        .as_ref()
                        .map(|p| image::open(bundle.root.join(p)).map(|i| i.to_luma8().into_raw()))
                        .transpose()?;
                    sources.push(AvatarSource {
                        member: member.clone(),
                        template: t
                            .image
                            .strip_prefix("image/")
                            .context("HUD 模板不在 image 目录")?
                            .into(),
                        mask,
                        width: image.width(),
                        height: image.height(),
                    });
                }
            }
        }
        ensure!(
            !sources.is_empty() && sources.len() <= 512,
            "HUD 头像目录无效"
        );
        Ok(Self {
            root: bundle.root.clone(),
            roi,
            sources,
            cache: vec![],
            diagnostics: vec![],
        })
    }
    pub fn reset(&mut self) {
        self.cache.clear();
        self.diagnostics.clear();
    }
    pub fn diagnostics(&self) -> &[HudCandidate] {
        &self.diagnostics
    }
    fn fingerprint(&self, frame: &CombatFrame, source: usize, rect: [i32; 4]) -> Result<Vec<u8>> {
        let s = &self.sources[source];
        ensure!(
            frame.image.buffer.image_type() == 16
                && frame.image.buffer.width() == 1920
                && frame.image.buffer.height() == 1080
                && od_core::intel_board::valid_rect(rect)
                && rect[2] == s.width as i32
                && rect[3] == s.height as i32,
            "HUD 像素区域无效"
        );
        let bgr = frame.image.buffer.raw_data().context("HUD 没有像素")?;
        ensure!(bgr.len() == 1920 * 1080 * 3, "HUD 像素格式无效");
        let mut digest = Sha256::new();
        for y in 0..s.height as usize {
            for x in 0..s.width as usize {
                let index = y * s.width as usize + x;
                if s.mask.as_ref().is_none_or(|m| m[index] > 0) {
                    let i = ((rect[1] as usize + y) * 1920 + rect[0] as usize + x) * 3;
                    digest.update(&bgr[i..i + 3]);
                }
            }
        }
        Ok(digest.finalize().to_vec())
    }
    fn cached(&self, frame: &CombatFrame) -> Result<Option<Vec<RosterMember>>> {
        if self.cache.is_empty() {
            return Ok(None);
        }
        for slot in &self.cache {
            if self.fingerprint(frame, slot.source, slot.rect)? != slot.pixels {
                return Ok(None);
            }
        }
        Ok(Some(
            self.cache
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
        self.cache.clear();
        self.diagnostics.clear();
        let started = Instant::now();
        let mut hits = vec![];
        for (source, s) in self.sources.iter().enumerate() {
            if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
                return Ok(None);
            }
            let read=session.recognize_prepared(&frame.image,"TemplateMatch",&json!({"roi":self.roi,"template":s.template,"threshold":0.8,"method":5,"green_mask":s.mask.is_some(),"order_by":"Score"}))?;
            if read.detail["hit"] != true {
                continue;
            }
            for item in read.detail["detail"]["filtered"]
                .as_array()
                .context("HUD 模板没有完整结果")?
            {
                let rect: [i32; 4] = serde_json::from_value(item["box"].clone())?;
                let score = item["score"].as_f64().context("HUD 没有匹配得分")?;
                ensure!(
                    score.is_finite() && score >= 0.8 && od_core::intel_board::valid_rect(rect),
                    "HUD 匹配结果无效"
                );
                self.diagnostics.push(HudCandidate {
                    member: s.member.clone(),
                    rect,
                    score,
                    template: s.template.clone(),
                });
                hits.push((source, rect, score));
                ensure!(hits.len() <= 1024, "HUD 匹配数量过多");
            }
        }
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let overlap = |a: [i32; 4], b: [i32; 4]| {
            let w = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
            let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
            w > 0
                && h > 0
                && i64::from(w) * i64::from(h) * 2
                    >= i64::from(a[2] * a[3]).min(i64::from(b[2] * b[3]))
        };
        let mut groups: Vec<Vec<(usize, [i32; 4], f64)>> = vec![];
        for hit in hits {
            let mut merged = vec![hit];
            let mut i = 0;
            while i < groups.len() {
                if groups[i]
                    .iter()
                    .any(|a| merged.iter().any(|b| overlap(a.1, b.1)))
                {
                    merged.extend(groups.remove(i));
                    i = 0;
                } else {
                    i += 1;
                }
            }
            groups.push(merged);
        }
        if groups.is_empty() || groups.len() > 3 {
            return Ok(None);
        }
        let mut selected = vec![];
        for group in groups {
            let mut by_id: BTreeMap<&str, (usize, [i32; 4], f64)> = BTreeMap::new();
            for hit in group {
                let key = self.sources[hit.0].member.id.as_str();
                if by_id.get(key).is_none_or(|old| hit.2 > old.2) {
                    by_id.insert(key, hit);
                }
            }
            let mut best: Vec<_> = by_id.into_values().collect();
            best.sort_by(|a, b| b.2.total_cmp(&a.2));
            if best.get(1).is_some_and(|b| (best[0].2 - b.2).abs() < 1e-6) {
                return Ok(None);
            }
            selected.push(best[0]);
        }
        selected.sort_by_key(|h| h.1[0]);
        let members: Vec<_> = selected
            .iter()
            .map(|h| self.sources[h.0].member.clone())
            .collect();
        if od_core::lost_void_roster::validate_members(&members, members.len() as u8).is_err() {
            return Ok(None);
        }
        for (source, rect, _) in selected {
            self.cache.push(HudSlot {
                source,
                rect,
                pixels: self.fingerprint(frame, source, rect)?,
            });
        }
        Ok(Some(members))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RosterInputEnd {
    Posted,
    NotPosted,
    Changed,
    TimedOut,
}
impl NativeSession {
    pub fn require_lost_void_roster(&self, reader: &LostVoidHudReader) -> Result<()> {
        self.require_lost_void_scene()?;
        ensure!(
            self.combat_bundle.as_deref() == Some(reader.root.as_path()),
            "HUD 读取与当前战斗资源不符"
        );
        CombatBundle::load(&reader.root)?;
        Ok(())
    }
    fn roster_view(
        &self,
        frame: &CombatFrame,
        reader: &mut LostVoidHudReader,
        cancel: &AtomicBool,
        timeout: Duration,
        discover: bool,
    ) -> Result<Option<RosterView>> {
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        ensure!(
            self.combat_bundle.as_deref() == Some(reader.root.as_path()),
            "HUD 资源尚未载入"
        );
        let Some(scene) = self.inspect_lost_void_npc(&frame.image, false, cancel, timeout)? else {
            return Ok(None);
        };
        let mut members = None;
        if scene.screen == S::World {
            members = reader.cached(frame)?;
            if members.is_none() {
                reader.cache.clear();
                if discover {
                    members = reader.discover(
                        self,
                        frame,
                        cancel,
                        timeout.saturating_sub(started.elapsed()),
                    )?;
                }
            }
        } else {
            reader.reset();
        }
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        Ok(Some(RosterView {
            screen: scene.screen,
            members,
        }))
    }
    pub fn inspect_lost_void_roster(
        &self,
        frame: &CombatFrame,
        reader: &mut LostVoidHudReader,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<RosterView>> {
        self.roster_view(frame, reader, cancel, timeout, true)
    }
    pub fn read_lost_void_roster(
        &self,
        flow: &RosterFlow,
        reader: &mut LostVoidHudReader,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<RosterView>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        let started = Instant::now();
        let Some(frame) = self.combat_frame(cancel)? else {
            return Ok(None);
        };
        let mut view = self.roster_view(
            &frame,
            reader,
            cancel,
            timeout.saturating_sub(started.elapsed()),
            true,
        )?;
        if frame.age_ms() > 1500 || cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        if let Some(v) = view.as_mut()
            && v.members
                .as_ref()
                .is_some_and(|m| m.len() != usize::from(flow.request().expected_count))
        {
            v.members = None;
        }
        Ok(view)
    }
    pub fn act_lost_void_roster(
        &mut self,
        flow: &RosterFlow,
        action: RosterAction,
        reader: &mut LostVoidHudReader,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<RosterInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<RosterInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(RosterInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(RosterInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.combat_frame(cancel)? else {
                return Ok(RosterInputEnd::NotPosted);
            };
            let view = self.roster_view(
                &frame,
                reader,
                cancel,
                timeout.saturating_sub(started.elapsed()),
                false,
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(RosterInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(RosterInputEnd::TimedOut);
            }
            if frame.age_ms() > 1500 || view.as_ref().is_none_or(|v| !flow.allows(action, v)) {
                return Ok(RosterInputEnd::Changed);
            }
            let mut sent = vec![];
            for input in flow.request().inputs(action) {
                if frame.age_ms() > 1500
                    || cancel.load(Ordering::Acquire)
                    || started.elapsed() >= timeout
                {
                    break;
                }
                if !self.combat_input(*input, true, cancel)? {
                    break;
                }
                sent.push(*input);
            }
            if sent.is_empty() {
                return Ok(RosterInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(100)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(2));
            }
            for input in sent.into_iter().rev() {
                self.combat_input(input, false, &AtomicBool::new(false))?;
            }
            Ok(RosterInputEnd::Posted)
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}
