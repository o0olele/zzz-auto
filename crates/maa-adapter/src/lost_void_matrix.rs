//! 矩阵编队同帧识别和输入。坐标只存在于本次识别，不进入检查点。
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, Word, words},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    InputProbeAction, MouseButton,
    lost_void_matrix::{
        LostVoidMatrixFlow, LostVoidMatrixRequest, MatrixAction as A, MatrixCandidate,
        MatrixCandidateKind, MatrixPane, MatrixRole, MatrixSlot, MatrixView, matrix_property,
    },
    lost_void_scene::LostVoidScreen as S,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixInputEnd {
    Posted,
    NotPosted,
    Changed,
    PartialScroll,
    TimedOut,
}
struct Sight {
    view: MatrixView,
    points: BTreeMap<String, [i32; 2]>,
}
struct ReadMode<'a> {
    hint: Option<&'a MatrixView>,
    discover: bool,
    force: bool,
}
const MAIN: [[i32; 4]; 3] = [
    [1028, 162, 142, 137],
    [1170, 162, 142, 137],
    [1312, 162, 142, 137],
];
const SUPPORT: [i32; 4] = [1018, 350, 163, 159];
fn raw(frame: &RecognitionImage) -> Result<&[u8]> {
    ensure!(
        frame.buffer.width() == 1920
            && frame.buffer.height() == 1080
            && frame.buffer.image_type() == 16,
        "矩阵截图须为 1080p BGR"
    );
    let b = frame.buffer.raw_data().context("矩阵截图没有像素")?;
    ensure!(b.len() == 1920 * 1080 * 3, "矩阵像素长度无效");
    Ok(b)
}
fn rgb(b: &[u8], x: i32, y: i32) -> [u8; 3] {
    let i = (y as usize * 1920 + x as usize) * 3;
    [b[i + 2], b[i + 1], b[i]]
}
fn crop(frame: &RecognitionImage, [x, y, w, h]: [i32; 4]) -> Result<RecognitionImage> {
    ensure!(
        od_core::intel_board::valid_rect([x, y, w, h]),
        "矩阵裁图区域无效"
    );
    let b = raw(frame)?;
    let mut out = Vec::with_capacity((w * h * 3) as usize);
    for yy in y..y + h {
        let i = (yy as usize * 1920 + x as usize) * 3;
        out.extend_from_slice(&b[i..i + w as usize * 3]);
    }
    let mut buffer = maa_framework::buffer::MaaImageBuffer::new()?;
    buffer.set_raw_data(&out, w, h, 16)?;
    Ok(RecognitionImage {
        buffer,
        prepare_us: 0,
    })
}
fn signature(frame: &RecognitionImage, [x, y, w, h]: [i32; 4]) -> Result<String> {
    let b = raw(frame)?;
    let mut hash = Sha256::new();
    for yy in y + 8..y + h - 35 {
        let i = (yy as usize * 1920 + (x + 8) as usize) * 3;
        hash.update(&b[i..i + (w - 16) as usize * 3]);
    }
    Ok(hex::encode(hash.finalize()))
}
fn unique(ws: &[Word], s: &str) -> Option<[i32; 2]> {
    let a: Vec<_> = ws.iter().filter(|w| w.text == s).collect();
    (a.len() == 1).then(|| a[0].center())
}
fn colorful(frame: &RecognitionImage, [x, y, w, h]: [i32; 4]) -> Result<bool> {
    let b = raw(frame)?;
    let mut n = 0;
    for yy in y..y + h {
        for xx in x..x + w {
            let p = rgb(b, xx, yy);
            if *p.iter().max().unwrap() >= 100
                && i32::from(*p.iter().max().unwrap()) - i32::from(*p.iter().min().unwrap()) >= 60
            {
                n += 1;
            }
        }
    }
    Ok(n >= 120)
}
fn plus(frame: &RecognitionImage, [x, y, w, h]: [i32; 4]) -> Result<bool> {
    let b = raw(frame)?;
    let (cx, cy) = (x + w / 2, y + (h - 12) / 2);
    let bright = |x, y| {
        let p = rgb(b, x, y);
        *p.iter().min().unwrap() >= 90
            && i32::from(*p.iter().max().unwrap()) - i32::from(*p.iter().min().unwrap()) <= 40
    };
    let a = (-12..=12)
        .filter(|d| (-2..=2).any(|q| bright(cx + *d, cy + q)))
        .count();
    let z = (-12..=12)
        .filter(|d| (-2..=2).any(|q| bright(cx + q, cy + *d)))
        .count();
    let corners = [(-12, -12), (-12, 12), (12, -12), (12, 12)]
        .iter()
        .filter(|&&(dx, dy)| bright(cx + dx, cy + dy))
        .count();
    Ok(a >= 19 && z >= 19 && corners == 0)
}
fn cached_slot(s: Option<&MatrixSlot>, sig: &str) -> Option<String> {
    match s {
        Some(MatrixSlot::Occupied {
            agent_id,
            signature,
            ..
        }) if signature == sig => agent_id.clone(),
        _ => None,
    }
}
impl NativeSession {
    fn matrix_avatar(
        &self,
        frame: &RecognitionImage,
        roi: [i32; 4],
        ids: Option<&[String]>,
        r: &Reader<'_>,
    ) -> Result<Option<String>> {
        let card = crop(frame, roi)?;
        let mut found = Vec::new();
        for a in od_core::random_play::agents()
            .iter()
            .filter(|a| ids.is_none_or(|ids| ids.contains(&a.id)))
        {
            ensure!(
                !r.cancel.load(Ordering::Acquire) && r.started.elapsed() < r.timeout,
                "矩阵头像识别取消或超时"
            );
            let templates: Vec<_> = a
                .templates
                .iter()
                .map(|t| format!("random-play/avatar_{t}.png"))
                .collect();
            let m=self.recognize_prepared(&card,"FeatureMatch",&json!({"template":templates,"roi":[0,0,roi[2],roi[3]],"detector":"SIFT","ratio":0.5,"count":4,"green_mask":true,"order_by":"Score"}))?;
            if m.detail["hit"] == true
                && m.detail["detail"]["filtered"]
                    .as_array()
                    .context("矩阵头像结果缺失")?
                    .iter()
                    .any(|h| {
                        serde_json::from_value::<[i32; 4]>(h["box"].clone()).is_ok_and(
                            |[x, y, w, h]| {
                                x >= 0
                                    && y >= 0
                                    && w >= 20
                                    && h >= 25
                                    && x + w <= roi[2]
                                    && y + h <= roi[3]
                            },
                        )
                    })
            {
                found.push(a.id.clone());
            }
        }
        Ok((found.len() == 1).then(|| found.remove(0)))
    }
    #[allow(clippy::too_many_arguments)]
    fn matrix_slot(
        &self,
        r: &Reader<'_>,
        roi: [i32; 4],
        role: MatrixRole,
        ws: &[Word],
        hint: Option<&MatrixSlot>,
        discover: bool,
        ids: Option<&[String]>,
    ) -> Result<(MatrixSlot, Option<[i32; 2]>)> {
        let [x, y, w, h] = roi;
        let tag = match role {
            MatrixRole::Main => "主战",
            MatrixRole::Support => "协战",
        };
        let tags: Vec<_> = ws
            .iter()
            .filter(|a| {
                a.inside([x, y, x + w, y + h])
                    && (a.text == tag
                        || role == MatrixRole::Main
                            && ["主战1", "主战2", "主战3"].contains(&a.text.as_str()))
            })
            .collect();
        if tags.len() == 1 {
            let sig = signature(r.frame, roi)?;
            let mut id = cached_slot(hint, &sig);
            if id.is_none() && discover {
                id = self.matrix_avatar(r.frame, roi, ids, r)?;
            }
            return Ok((
                MatrixSlot::Occupied {
                    agent_id: id,
                    signature: sig,
                    role,
                },
                Some(tags[0].center()),
            ));
        }
        if tags.is_empty() && plus(r.frame, roi)? {
            return Ok((MatrixSlot::Empty, None));
        }
        Ok((MatrixSlot::Unknown, None))
    }
    fn matrix_sight(
        &self,
        frame: &RecognitionImage,
        flow: &LostVoidMatrixFlow,
        mode: ReadMode<'_>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        let ReadMode {
            hint,
            discover,
            force,
        } = mode;
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let r = Reader {
            session: self,
            frame,
            cancel,
            started,
            timeout,
            pipeline: od_core::lost_void_scene::pipeline(),
        };
        let read = |s: &str| words(&r.read(&format!("LostVoid.{s}"), None)?);
        let mut out = Sight {
            view: MatrixView::default(),
            points: BTreeMap::new(),
        };
        let mut modal = read("MatrixConfirm")?;
        modal.extend(read("ModalConfirm")?);
        if modal
            .iter()
            .any(|w| ["确认", "确定", "取消"].contains(&w.text.as_str()))
        {
            out.view.screen = S::Modal;
            return Ok(Some(out));
        }
        let tutorial = read("Tutorial")?;
        if tutorial
            .iter()
            .any(|w| w.text.contains("此界面可以进行代理人编队"))
            && tutorial.iter().any(|w| w.text.contains("继续下一步"))
        {
            out.view.screen = S::MatrixTeam;
            out.view.tutorial = true;
            out.points.insert("tutorial".into(), [1450, 120]);
            return Ok(Some(out));
        }
        let title = read("PrepareTitle")?;
        if unique(&title, "矩阵行动").is_some() && r.hit("LostVoid.Back", None)? {
            let header = read("PrepareHeader")?;
            if unique(&header, "街区").is_some() {
                out.view.screen = S::Matrix;
                let next = read("PrepareNextEnabled")?;
                if let Some(p) = unique(&next, "下一步") {
                    out.view.next_enabled = true;
                    out.points.insert("next".into(), p);
                }
                return Ok(Some(out));
            }
        }
        let tabs = read("MatrixTabs")?;
        let labels = read("MatrixLabels")?;
        let start = read("MatrixStart")?;
        if unique(&tabs, "预备编队").is_none()
            || unique(&labels, "主战编队").is_none()
            || unique(&labels, "协战代理人").is_none()
            || unique(&start, "开始挑战").is_none()
        {
            return Ok(Some(out));
        }
        out.view.screen = S::MatrixTeam;
        let enabled = read("MatrixStartEnabled")?;
        out.view.start_enabled = unique(&enabled, "开始挑战").is_some();
        let free = colorful(frame, [175, 15, 420, 65])?;
        let preset = colorful(frame, [620, 15, 330, 65])?;
        out.view.pane = match (free, preset) {
            (true, false) => MatrixPane::Agents,
            (false, true) => MatrixPane::Presets,
            _ => MatrixPane::Unknown,
        };
        if let Some(p) = unique(&tabs, "自由编队") {
            out.points.insert("agents".into(), p);
        }
        if let Some(p) = unique(&tabs, "预备编队") {
            out.points.insert("presets".into(), p);
        }
        if let Some(p) = unique(&labels, "协战代理人") {
            out.points.insert("support-focus".into(), p);
        }
        let main = read("MatrixMain")?;
        let support = read("MatrixSupport")?;
        let ids: Vec<_> = flow
            .plan()
            .expected_agents
            .iter()
            .filter(|s| s.as_str() != od_core::predefined_teams::UNKNOWN_AGENT)
            .cloned()
            .collect();
        let main_discover = discover && (force || flow.needs_main_identity() || !ids.is_empty());
        let identity_filter = (!ids.is_empty() && ids.len() == 3).then_some(ids.as_slice());
        for (i, roi) in MAIN.into_iter().enumerate() {
            let (s, p) = self.matrix_slot(
                &r,
                roi,
                MatrixRole::Main,
                &main,
                hint.map(|v| &v.main[i]),
                main_discover,
                identity_filter,
            )?;
            out.view.main[i] = s;
            if let Some(p) = p {
                out.points.insert(format!("remove:{i}"), p);
            }
        }
        if let Some(w) = main.iter().find(|w| w.inside([1028, 162, 1633, 299])) {
            out.points.insert("main-focus".into(), w.center());
        } else if let Some(p) = unique(&labels, "主战编队") {
            out.points.insert("main-focus".into(), p);
        }
        let (support_slot, _) = self.matrix_slot(
            &r,
            SUPPORT,
            MatrixRole::Support,
            &support,
            hint.map(|v| &v.support),
            discover,
            None,
        )?;
        out.view.support = support_slot;
        for w in read("MatrixProperty")? {
            if let Some(p) = matrix_property(&w.text)
                && !out.view.support_rules.contains(&p)
            {
                out.view.support_rules.push(p);
            }
        }
        let list = read("MatrixList")?;
        if out.view.pane == MatrixPane::Presets {
            let mut names: Vec<_> = list
                .iter()
                .filter(|w| {
                    flow.request()
                        .teams
                        .teams
                        .iter()
                        .any(|t| od_core::predefined_teams::normalize_name(&t.name) == w.text)
                })
                .collect();
            names.sort_by_key(|w| (w.center()[1], w.center()[0]));
            for w in names {
                let matches: Vec<_> = flow
                    .request()
                    .teams
                    .teams
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| od_core::predefined_teams::normalize_name(&t.name) == w.text)
                    .collect();
                let team_index = (matches.len() == 1).then(|| matches[0].0 as u8);
                let roi = [
                    w.rect[0].max(185),
                    w.rect[1].max(156),
                    w.rect[2].min(966 - w.rect[0].max(185)).max(17),
                    w.rect[3].max(44).min(1059 - w.rect[1].max(156)),
                ];
                if roi[2] <= 16 || roi[3] <= 35 {
                    continue;
                }
                let slot = out.view.candidates.len() as u8;
                out.view.candidates.push(MatrixCandidate {
                    slot,
                    signature: signature(frame, roi)?,
                    item: MatrixCandidateKind::Preset {
                        team_index,
                        name: w.text.clone(),
                    },
                });
                out.points.insert(format!("preset:{slot}"), w.center());
            }
        } else if out.view.pane == MatrixPane::Agents {
            let ah = read("MatrixAgentsHeader")?;
            if unique(&ah, "代理人").is_none() {
                out.view.pane = MatrixPane::Unknown;
                return Ok(Some(out));
            }
            let mut cells = Vec::new();
            for row in 0..4 {
                for col in 0..5 {
                    let roi = [190 + 156 * col, 192 + 200 * row, 150, 156];
                    let [x, y, w, h] = roi;
                    let levels: Vec<_> = list
                        .iter()
                        .filter(|a| a.inside([x, y + 112, x + w, y + h]))
                        .filter_map(|a| a.text.parse::<u8>().ok())
                        .filter(|v| *v <= 90)
                        .collect();
                    let up = list.iter().any(|a| {
                        a.inside([x, y, x + w, y + h]) && a.text.eq_ignore_ascii_case("UP")
                    }) || r.hit("LostVoid.MatrixUP", Some([x + 4, y + 3, 60, 32]))?;
                    if levels.is_empty() && !up {
                        continue;
                    }
                    let level = (levels.len() == 1).then(|| levels[0]);
                    cells.push((roi, level, up));
                }
            }
            let support_mode = force || flow.selecting_support();
            let candidate_index = if cells.iter().any(|c| c.2) {
                cells.iter().position(|c| c.2)
            } else {
                cells.iter().position(|c| c.1 == Some(60))
            };
            for (i, (roi, level, up)) in cells.into_iter().enumerate() {
                let slot = out.view.candidates.len() as u8;
                let sig = signature(frame, roi)?;
                let old = hint
                    .and_then(|v| v.candidates.get(slot as usize))
                    .filter(|c| c.signature == sig);
                let mut id = old.and_then(|c| match &c.item {
                    MatrixCandidateKind::Agent { agent_id, .. } => agent_id.clone(),
                    _ => None,
                });
                if id.is_none() && discover {
                    if support_mode && candidate_index == Some(i) {
                        id = self.matrix_avatar(frame, roi, None, &r)?;
                    } else if !support_mode && !ids.is_empty() {
                        id = self.matrix_avatar(frame, roi, Some(&ids), &r)?;
                    }
                }
                out.view.candidates.push(MatrixCandidate {
                    slot,
                    signature: sig,
                    item: MatrixCandidateKind::Agent {
                        agent_id: id,
                        level,
                        up,
                    },
                });
                out.points.insert(
                    format!("agent:{slot}"),
                    [roi[0] + roi[2] / 2, roi[1] + roi[3] / 2],
                );
            }
        }
        out.view.validate().map_err(anyhow::Error::msg)?;
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        Ok(Some(out))
    }
    pub fn inspect_lost_void_matrix(
        &self,
        frame: &RecognitionImage,
        request: LostVoidMatrixRequest,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<MatrixView>> {
        self.require_lost_void_scene()?;
        let flow = LostVoidMatrixFlow::new(request).map_err(anyhow::Error::msg)?;
        Ok(self
            .matrix_sight(
                frame,
                &flow,
                ReadMode {
                    hint: None,
                    discover: true,
                    force: true,
                },
                cancel,
                timeout,
            )?
            .map(|s| s.view))
    }
    /// 出战前只复验完成帧的实际槽位像素摘要，变化后保持未知，不在输入守卫里全目录发现。
    pub(crate) fn verify_lost_void_matrix_prepared(
        &self,
        frame: &RecognitionImage,
        flow: &LostVoidMatrixFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<MatrixView>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        ensure!(flow.completed(), "矩阵出战缺少完成的编队检查点");
        Ok(self
            .matrix_sight(
                frame,
                flow,
                ReadMode {
                    hint: flow.current(),
                    discover: false,
                    force: false,
                },
                cancel,
                timeout,
            )?
            .map(|s| s.view))
    }
    pub fn read_lost_void_matrix(
        &self,
        flow: &LostVoidMatrixFlow,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<MatrixView>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_lost_void_scene()?;
        let at = Instant::now();
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let Some(mut s) = self.matrix_sight(
            &frame.image,
            flow,
            ReadMode {
                hint: flow.current(),
                discover: true,
                force: false,
            },
            cancel,
            timeout.saturating_sub(at.elapsed()),
        )?
        else {
            return Ok(None);
        };
        if frame.age() > Duration::from_millis(1500) {
            let Some(fresh) = self.navigation_frame(cancel)? else {
                return Ok(None);
            };
            let Some(verified) = self.matrix_sight(
                &fresh.image,
                flow,
                ReadMode {
                    hint: Some(&s.view),
                    discover: false,
                    force: false,
                },
                cancel,
                timeout.saturating_sub(at.elapsed()),
            )?
            else {
                return Ok(None);
            };
            if fresh.age() > Duration::from_millis(1500) {
                return Ok(None);
            }
            s = verified;
        }
        if cancel.load(Ordering::Acquire) || at.elapsed() >= timeout {
            return Ok(None);
        }
        Ok(Some(s.view))
    }
    pub fn act_lost_void_matrix(
        &mut self,
        flow: &LostVoidMatrixFlow,
        action: A,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<MatrixInputEnd> {
        let at = Instant::now();
        let result = (|| -> Result<MatrixInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.require_lost_void_scene()?;
            self.check_ether_synthesis_target(game)?;
            if cancel.load(Ordering::Acquire) {
                return Ok(MatrixInputEnd::NotPosted);
            }
            if timeout.is_zero() {
                return Ok(MatrixInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(MatrixInputEnd::NotPosted);
            };
            let Some(s) = self.matrix_sight(
                &frame.image,
                flow,
                ReadMode {
                    hint: flow.before(),
                    discover: false,
                    force: false,
                },
                cancel,
                timeout.saturating_sub(at.elapsed()),
            )?
            else {
                return Ok(MatrixInputEnd::NotPosted);
            };
            if cancel.load(Ordering::Acquire) {
                return Ok(MatrixInputEnd::NotPosted);
            }
            if at.elapsed() >= timeout {
                return Ok(MatrixInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) || !flow.allows(action, &s.view) {
                return Ok(MatrixInputEnd::Changed);
            }
            let key = match action {
                A::Next => "next".into(),
                A::DismissTutorial => "tutorial".into(),
                A::AgentsTab => "agents".into(),
                A::PresetsTab => "presets".into(),
                A::FocusMain => "main-focus".into(),
                A::RemoveMain(i) => format!("remove:{i}"),
                A::PickAgent(i) | A::PickSupport(i) => format!("agent:{i}"),
                A::PickPreset(i) => format!("preset:{i}"),
                A::FocusSupport => "support-focus".into(),
                A::Up | A::Down => "scroll".into(),
            };
            let scroll = matches!(action, A::Up | A::Down);
            let point = if scroll {
                Some([575, if action == A::Up { 337 } else { 818 }])
            } else {
                s.points.get(&key).copied()
            };
            let Some([x, y]) = point else {
                return Ok(MatrixInputEnd::Changed);
            };
            let input = |y| InputProbeAction::MouseButton {
                button: MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input(y), true, cancel)? {
                return Ok(MatrixInputEnd::NotPosted);
            }
            let held = Instant::now();
            let duration = Duration::from_millis(if scroll { 1000 } else { 100 });
            while held.elapsed() < duration
                && !cancel.load(Ordering::Acquire)
                && at.elapsed() < timeout
            {
                if scroll {
                    let d = (held.elapsed().as_millis().min(1000) as i32) * 481 / 1000;
                    self.combat_input(
                        input(y + if action == A::Up { d } else { -d }),
                        true,
                        cancel,
                    )?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let complete = held.elapsed() >= duration
                && !cancel.load(Ordering::Acquire)
                && at.elapsed() < timeout;
            if scroll && complete {
                self.combat_input(input(if action == A::Up { 818 } else { 337 }), true, cancel)?;
            }
            self.combat_input(input(y), false, &AtomicBool::new(false))?;
            if scroll && !complete {
                return Ok(MatrixInputEnd::PartialScroll);
            }
            let wait = Instant::now();
            while wait.elapsed() < Duration::from_millis(500)
                && !cancel.load(Ordering::Acquire)
                && at.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(MatrixInputEnd::Posted)
        })();
        let released = self.release_inputs();
        result.and_then(|e| released.map(|_| e))
    }
}
