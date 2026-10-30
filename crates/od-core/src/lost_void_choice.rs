//! 局内通用选择：只使用同帧目录身份与选择后态，不保存点击坐标。
use crate::{
    lost_void::{ArtifactCandidate, ArtifactSelection, LostVoidChallenge, choose_artifacts},
    lost_void_scene::{ChoiceKind, LostVoidObservation, LostVoidScreen as S},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceCard {
    pub artifact: ArtifactCandidate,
    pub selected: bool,
    pub blocked: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoicePage {
    pub kind: ChoiceKind,
    pub cards: Vec<ChoiceCard>,
    pub count: Option<(u8, u8)>,
    pub confirm_enabled: bool,
}
impl ChoicePage {
    pub fn required(&self) -> u8 {
        match self.kind {
            ChoiceKind::Artifact(n) => n,
            ChoiceKind::Gear | ChoiceKind::GearBranch => 1,
            _ => 0,
        }
    }
    fn same_content(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.cards.len() == other.cards.len()
            && self.cards.iter().zip(&other.cards).all(|(a, b)| {
                a.artifact.slot == b.artifact.slot
                    && a.artifact.category == b.artifact.category
                    && a.artifact.name == b.artifact.name
                    && a.artifact.level == b.artifact.level
                    && a.artifact.is_primary == b.artifact.is_primary
                    && a.blocked == b.blocked
            })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let required = self.required();
        if self.cards.is_empty()
            || self.cards.len() > 8
            || matches!(self.kind, ChoiceKind::Artifact(n) if !(1..=4).contains(&n))
            || self
                .cards
                .iter()
                .enumerate()
                .any(|(i, c)| c.artifact.slot != i as u8 || (c.selected && c.blocked))
            || self.count.is_some_and(|(a, b)| b != required || a > b)
            || self.cards.iter().filter(|c| c.selected).count() > required as usize
            || self
                .count
                .is_some_and(|(a, _)| self.cards.iter().filter(|c| c.selected).count() > a as usize)
        {
            return Err("迷失之地选择数量或槽位证据冲突");
        }
        if self.kind == ChoiceKind::GearBranch {
            let branches: Vec<_> = self
                .cards
                .iter()
                .filter_map(|c| {
                    c.artifact
                        .name
                        .rsplit_once('-')
                        .map(|(base, branch)| (&c.artifact.category, base, branch))
                })
                .collect();
            if self.cards.len() != 2
                || branches.len() != 2
                || branches[0].0 != branches[1].0
                || branches[0].1 != branches[1].1
                || !["a", "b"]
                    .iter()
                    .all(|b| branches.iter().filter(|(_, _, branch)| branch == b).count() == 1)
            {
                return Err("迷失之地战术棱镜缺少明确的两种分支身份");
            }
        }
        choose_artifacts(
            &LostVoidChallenge::default(),
            &self
                .cards
                .iter()
                .map(|c| c.artifact.clone())
                .collect::<Vec<_>>(),
            0,
            &ArtifactSelection::default(),
        )?;
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidChoiceView {
    pub scene: LostVoidObservation,
    pub page: Option<ChoicePage>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidChoiceRequest {
    pub run_id: Uuid,
    pub interaction_id: Uuid,
    pub challenge: LostVoidChallenge,
    pub options: ArtifactSelection,
    pub return_to: Vec<S>,
    pub timeout_ms: u32,
}
impl LostVoidChoiceRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.challenge.validate()?;
        choose_artifacts(&self.challenge, &[], 0, &self.options)?;
        if self.run_id.is_nil()
            || self.interaction_id.is_nil()
            || !(1000..=300_000).contains(&self.timeout_ms)
            || self.return_to.is_empty()
            || self.return_to.len() > 6
            || self.return_to.iter().any(|s| {
                ![
                    S::World,
                    S::Event,
                    S::Gear,
                    S::Store,
                    S::Lottery,
                    S::RouteChange,
                ]
                .contains(s)
            })
            || self
                .return_to
                .iter()
                .enumerate()
                .any(|(i, s)| self.return_to[..i].contains(s))
        {
            return Err("迷失之地选择身份、返回页面或时间预算无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceAction {
    Pick(u8),
    Confirm,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    action: ChoiceAction,
    before: ChoicePage,
    posted: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidChoiceReceipt {
    pub run_id: Uuid,
    pub interaction_id: Uuid,
    pub returned_to: S,
    pub confirmed_pages: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidChoiceFlow {
    request: LostVoidChoiceRequest,
    current: Option<ChoicePage>,
    pending: Option<Pending>,
    selected: Vec<u8>,
    confirmed: Vec<ChoicePage>,
    receipt: Option<LostVoidChoiceReceipt>,
}
impl LostVoidChoiceFlow {
    pub fn new(request: LostVoidChoiceRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            current: None,
            pending: None,
            selected: vec![],
            confirmed: vec![],
            receipt: None,
        })
    }
    pub fn request(&self) -> &LostVoidChoiceRequest {
        &self.request
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn receipt(&self) -> Option<&LostVoidChoiceReceipt> {
        self.receipt.as_ref()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        for p in self
            .current
            .iter()
            .chain(self.confirmed.iter())
            .chain(self.pending.iter().map(|p| &p.before))
        {
            p.validate()?;
        }
        let invalid_selection = self.selected.iter().enumerate().any(|(i, s)| {
            self.selected[..i].contains(s)
                || self
                    .current
                    .as_ref()
                    .is_none_or(|p| *s as usize >= p.cards.len())
        });
        let invalid_pending=self.pending.as_ref().is_some_and(|p| {
            self.current.as_ref()!=Some(&p.before)
                || matches!(p.action,ChoiceAction::Pick(s) if p.before.cards.get(s as usize).is_none_or(|c| c.selected || c.blocked))
                || self.action(&p.before).ok().flatten()!=Some(p.action)
        });
        let invalid_receipt = self.receipt.as_ref().is_some_and(|r| {
            r.run_id != self.request.run_id
                || r.interaction_id != self.request.interaction_id
                || !self.request.return_to.contains(&r.returned_to)
                || r.confirmed_pages as usize != self.confirmed.len() + 1
                || !self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.action == ChoiceAction::Confirm)
        });
        if self.confirmed.len() > 15
            || self.selected.len() > 4
            || invalid_selection
            || invalid_pending
            || invalid_receipt
        {
            return Err("迷失之地选择检查点不一致");
        }
        Ok(())
    }
    fn filled(&self, p: &ChoicePage) -> bool {
        let n = p.required();
        n == 0
            || p.count.is_some_and(|(a, b)| a == b && b == n)
            || (p.count.is_none()
                && n == 1
                && (p.cards.iter().any(|c| c.selected)
                    || (!self.selected.is_empty() && p.confirm_enabled)))
    }
    fn action(&self, p: &ChoicePage) -> Result<Option<ChoiceAction>, &'static str> {
        if self.filled(p) {
            return Ok(p.confirm_enabled.then_some(ChoiceAction::Confirm));
        }
        if p.required() > 1 && p.count.is_none() {
            return Ok(None);
        }
        if p.count.is_some_and(|(a, _)| {
            a as usize
                > self
                    .selected
                    .len()
                    .max(p.cards.iter().filter(|c| c.selected).count())
        }) {
            return Ok(None);
        }
        let mut options = self.request.options.clone();
        options.ignored_slots.extend(self.selected.iter().copied());
        options.ignored_slots.extend(
            p.cards
                .iter()
                .filter(|c| c.selected || c.blocked)
                .map(|c| c.artifact.slot),
        );
        options.ignored_slots.sort_unstable();
        options.ignored_slots.dedup();
        let result = choose_artifacts(
            &self.request.challenge,
            &p.cards
                .iter()
                .map(|c| c.artifact.clone())
                .collect::<Vec<_>>(),
            1,
            &options,
        )?;
        Ok(result.first().map(|c| ChoiceAction::Pick(c.slot)))
    }
    pub fn next(&mut self, v: &LostVoidChoiceView) -> Result<Option<ChoiceAction>, &'static str> {
        self.validate()?;
        if self.completed() {
            return Ok(None);
        }
        if self.request.return_to.contains(&v.scene.screen) {
            if self
                .pending
                .as_ref()
                .is_some_and(|p| p.action == ChoiceAction::Confirm)
            {
                self.receipt = Some(LostVoidChoiceReceipt {
                    run_id: self.request.run_id,
                    interaction_id: self.request.interaction_id,
                    returned_to: v.scene.screen,
                    confirmed_pages: self.confirmed.len() as u8 + 1,
                });
            }
            return Ok(None);
        }
        if v.scene.screen == S::Unknown {
            return Ok(None);
        }
        if v.scene.screen != S::CommonChoice {
            return Err("迷失之地选择遇到非预期页面");
        }
        let Some(p) = &v.page else { return Ok(None) };
        p.validate()?;
        if v.scene.choice != Some(p.kind) || v.scene.selection_count != p.count || !v.scene.confirm
        {
            return Err("迷失之地选择标题与页面观察不一致");
        }
        if self.confirmed.iter().any(|old| old.same_content(p)) {
            return Err("迷失之地选择重新出现已确认内容");
        }
        if let Some(pending) = &self.pending {
            match pending.action {
                ChoiceAction::Confirm => {
                    if pending.before.same_content(p) {
                        return Ok(None);
                    }
                    if self.confirmed.len() >= 15 {
                        return Err("迷失之地连续获得物确认达到上限");
                    }
                    self.confirmed.push(pending.before.clone());
                    self.selected.clear();
                    self.pending = None;
                    self.current = None;
                }
                ChoiceAction::Pick(slot) => {
                    if !pending.before.same_content(p) {
                        return Err("迷失之地选择后候选身份发生变化");
                    }
                    let before = &pending.before;
                    let marker = p.cards[slot as usize].selected;
                    let delta = match (before.count, p.count) {
                        (Some((a, b)), Some((x, y))) if b == y && x == a + 1 => true,
                        (Some(a), Some(b)) if a != b => return Err("迷失之地选择后计数异常"),
                        (Some(_), None) | (None, Some(_)) => return Ok(None),
                        _ => false,
                    };
                    let changed_elsewhere = p
                        .cards
                        .iter()
                        .zip(&before.cards)
                        .any(|(a, b)| a.artifact.slot != slot && a.selected != b.selected);
                    if changed_elsewhere {
                        return Err("迷失之地非目标藏品选择状态变化");
                    }
                    let single_enabled = p.required() == 1
                        && p.count.is_none()
                        && p.confirm_enabled
                        && !before.confirm_enabled;
                    if !(marker || delta || single_enabled) {
                        return Ok(None);
                    }
                    self.selected.push(slot);
                    self.pending = None;
                }
            }
        } else if self
            .current
            .as_ref()
            .is_some_and(|old| !old.same_content(p))
        {
            return Err("迷失之地待操作候选身份发生变化");
        }
        self.current = Some(p.clone());
        self.action(p)
    }
    pub fn prepare(&mut self, a: ChoiceAction) -> Result<(), &'static str> {
        self.validate()?;
        let p = self.current.as_ref().ok_or("迷失之地没有选择页面")?;
        if self.pending.is_some() || self.completed() || self.action(p)? != Some(a) {
            return Err("迷失之地没有待保存的选择动作");
        }
        self.pending = Some(Pending {
            action: a,
            before: p.clone(),
            posted: false,
        });
        Ok(())
    }
    pub fn allows(&self, a: ChoiceAction, v: &LostVoidChoiceView) -> bool {
        self.validate().is_ok()
            && v.scene.screen == S::CommonChoice
            && v.scene.confirm
            && v.page.as_ref().is_some_and(|p| {
                v.scene.choice == Some(p.kind) && v.scene.selection_count == p.count
            })
            && self
                .pending
                .as_ref()
                .is_some_and(|p| p.action == a && !p.posted && v.page.as_ref() == Some(&p.before))
    }
    pub fn posted(&mut self, a: ChoiceAction) -> Result<(), &'static str> {
        let p = self.pending.as_mut().ok_or("迷失之地选择投递没有意图")?;
        if p.action != a || p.posted {
            return Err("迷失之地选择投递回执不匹配");
        }
        p.posted = true;
        self.validate()
    }
    pub fn not_posted(&mut self, a: ChoiceAction) -> Result<(), &'static str> {
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| p.action == a && !p.posted)
        {
            return Err("迷失之地已投递选择不能撤销");
        }
        self.pending = None;
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.receipt = None;
        Ok(())
    }
}

/// 只接受目录中唯一且完整的名称，允许明确的中英文方括号及分类别名。
pub fn identify_artifact(text: &str) -> Result<Option<ArtifactCandidate>, &'static str> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let normalized = text
        .replace('【', "[")
        .replace('】', "]")
        .replace('『', "「")
        .replace('』', "」");
    let (category, name, primary) = if let Some(t) = normalized.strip_prefix('[') {
        let Some((c, n)) = t.split_once(']') else {
            return Ok(None);
        };
        (Some(c.split([':', '：']).next().unwrap_or(c)), n, true)
    } else {
        (None, normalized.as_str(), text.starts_with('「'))
    };
    let norm = |s: &str| {
        s.chars()
            .filter(|c| !c.is_whitespace() && *c != '·')
            .collect::<String>()
    };
    let list = crate::lost_void::artifacts()?;
    let hits: Vec<_> = list
        .iter()
        .filter(|a| {
            norm(&a.name) == norm(name)
                && category.is_none_or(|c| {
                    norm(&a.category) == norm(c) || (c == "击破" && a.category == "异常·击破")
                })
        })
        .collect();
    if hits.len() != 1 {
        return Ok(None);
    }
    let a = hits[0];
    Ok(Some(ArtifactCandidate {
        slot: 0,
        category: a.category.clone(),
        name: a.name.clone(),
        level: a.level.clone(),
        is_new: false,
        is_primary: primary || (category.is_none() && a.category != "无详情"),
    }))
}
