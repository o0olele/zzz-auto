//! 制造坊与饮茶仙共用派驻；只有同一台位和商品的运行后态能证明制造成功。
use crate::{suibian_adventure::CommitState, suibian_temple::SuibianTempleConfig};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CraftMode {
    Workshop,
    Material,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CraftPhase {
    Inspect,
    Open,
    Choose,
    Picker,
    Confirm,
    ReturnJob,
    ReturnEntry,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CraftOutcome {
    Started,
    NoBangboo,
    NoMaterials,
    NoGoods,
    SearchLimit,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CraftCommitKind {
    Assign,
    Start,
    ConfirmPlan,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CraftCommit {
    pub kind: CraftCommitKind,
    pub slot: u8,
    pub goods: String,
    pub bangboo: Option<u8>,
    pub amount: Option<u32>,
    pub state: CommitState,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CraftResult {
    pub slot: u8,
    pub goods: String,
    pub outcome: CraftOutcome,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CraftJob {
    pub slot: u8,
    pub goods: Option<String>,
    pub wanted: Option<String>,
    pub owned: bool,
    pub candidate: u8,
    pub scrolls: u8,
    pub last_names: Vec<String>,
    pub after_scroll: bool,
    pub unchanged_reads: u8,
}
impl CraftJob {
    fn new(slot: u8) -> Self {
        Self {
            slot,
            goods: None,
            wanted: None,
            owned: false,
            candidate: 2,
            scrolls: 0,
            last_names: vec![],
            after_scroll: false,
            unchanged_reads: 0,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    Open,
    Paused,
    Running,
    Unavailable,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CraftSlot {
    pub slot: u8,
    pub state: SlotState,
    pub goods: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CraftBangboo {
    Ready,
    Missing,
    Exhausted,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CraftGood {
    pub name: String,
    pub white: bool,
    pub makeable: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CraftSelection {
    pub slot: u8,
    pub goods: String,
    pub bangboo: CraftBangboo,
    pub materials: Option<bool>,
    pub amount: Option<u32>,
    pub can_start: bool,
    pub list_readable: bool,
    pub rows: Vec<CraftGood>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CraftCard {
    Available,
    Busy,
    Exhausted,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CraftView {
    Unknown,
    Entry,
    Managed,
    Management,
    Overview(Vec<CraftSlot>),
    Selection(CraftSelection),
    Picker {
        cards: [CraftCard; 8],
        selected: Option<u8>,
    },
    AssignRejected,
    PlanConfirm,
    Running {
        slot: u8,
        goods: String,
    },
    MaterialMenu(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CraftAction {
    OpenManagement,
    OpenWorkshop,
    OpenSlot(u8),
    OpenPicker,
    SelectBangboo(u8),
    Assign(u8),
    SelectGood(String),
    Scroll,
    Start(u32),
    ConfirmPlan,
    Back,
}
impl CraftAction {
    fn kind(&self) -> Option<CraftCommitKind> {
        match self {
            Self::Assign(_) => Some(CraftCommitKind::Assign),
            Self::Start(_) => Some(CraftCommitKind::Start),
            Self::ConfirmPlan => Some(CraftCommitKind::ConfirmPlan),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TempleCraft {
    pub config: SuibianTempleConfig,
    pub mode: CraftMode,
    pub material: Option<String>,
    pub phase: CraftPhase,
    pub job: Option<CraftJob>,
    pub results: Vec<CraftResult>,
    pub chosen: Vec<String>,
    pub commits: Vec<CraftCommit>,
    pub pending: Option<u8>,
    pub overview_checked: bool,
    pub returned_entry: bool,
    pub returned_material: bool,
}
pub fn goods_key(text: &str) -> Option<String> {
    let value: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '【' | '［' => '[',
            '】' | '］' => ']',
            _ => c,
        })
        .collect();
    (!value.is_empty()
        && value.chars().count() <= 64
        && value.chars().any(|c| ('一'..='鿿').contains(&c))
        && !value.chars().any(char::is_control))
    .then_some(value)
}
fn valid_goods(text: &str) -> bool {
    goods_key(text).as_deref() == Some(text)
}
impl TempleCraft {
    pub fn new(config: SuibianTempleConfig) -> Result<Self, &'static str> {
        Self::create(config, None)
    }
    pub fn material(config: SuibianTempleConfig, goods: String) -> Result<Self, &'static str> {
        Self::create(config, Some(goods))
    }
    fn create(config: SuibianTempleConfig, material: Option<String>) -> Result<Self, &'static str> {
        let flow = Self {
            config,
            mode: if material.is_some() {
                CraftMode::Material
            } else {
                CraftMode::Workshop
            },
            material,
            phase: CraftPhase::Inspect,
            job: None,
            results: vec![],
            chosen: vec![],
            commits: vec![],
            pending: None,
            overview_checked: false,
            returned_entry: false,
            returned_material: false,
        };
        flow.validate()?;
        Ok(flow)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate()?;
        if (self.mode == CraftMode::Material) != self.material.is_some()
            || self.material.as_ref().is_some_and(|g| !valid_goods(g))
            || self.chosen.len() > 128
            || self
                .chosen
                .iter()
                .enumerate()
                .any(|(i, g)| !valid_goods(g) || self.chosen[..i].contains(g))
            || self.results.len()
                > if self.mode == CraftMode::Material {
                    1
                } else {
                    4
                }
            || self.results.iter().enumerate().any(|(i, r)| {
                !(1..=4).contains(&r.slot)
                    || !valid_goods(&r.goods)
                    || self.results[..i]
                        .iter()
                        .any(|o| o.slot == r.slot || o.outcome != CraftOutcome::Started)
                    || (self.mode == CraftMode::Material
                        && self.material.as_ref() != Some(&r.goods))
            })
            || self.commits.len() > 128
            || self.commits.iter().enumerate().any(|(i, c)| {
                !(1..=4).contains(&c.slot)
                    || !valid_goods(&c.goods)
                    || (c.state == CommitState::Intent && self.pending != Some(i as u8))
                    || (c.kind == CraftCommitKind::Assign
                        && c.bangboo.is_none_or(|n| !(2..=8).contains(&n)))
                    || (c.kind == CraftCommitKind::Start && c.amount.is_none_or(|n| n == 0))
            })
            || self.pending.is_some_and(|i| {
                usize::from(i) + 1 != self.commits.len()
                    || self.commits[usize::from(i)].state != CommitState::Intent
            })
            || self.job.as_ref().is_some_and(|j| {
                !(1..=4).contains(&j.slot)
                    || !(2..=9).contains(&j.candidate)
                    || j.scrolls > self.config.craft_drag_times
                    || j.unchanged_reads > 2
                    || j.last_names.len() > 32
                    || j.last_names.iter().any(|g| !valid_goods(g))
                    || j.goods.as_ref().is_some_and(|g| !valid_goods(g))
                    || j.wanted.as_ref().is_some_and(|g| !valid_goods(g))
                    || (j.owned && j.goods.is_none())
                    || self.results.iter().any(|r| r.slot == j.slot)
            })
            || matches!(
                self.phase,
                CraftPhase::Choose | CraftPhase::Picker | CraftPhase::Confirm
            ) != self.job.is_some()
            || self.returned_entry
                != (self.phase == CraftPhase::Done && self.mode == CraftMode::Workshop)
            || self.returned_material
                != (self.phase == CraftPhase::Done && self.mode == CraftMode::Material)
            || (self.phase == CraftPhase::Done
                && (self.pending.is_some() || (!self.overview_checked && self.results.is_empty())))
        {
            return Err("制造检查点缺少冻结目标、顺序或返回证明");
        }
        if let Some(i) = self.pending {
            let c = &self.commits[usize::from(i)];
            let j = self.job.as_ref().ok_or("制造提交没有对应制造台")?;
            let phase = match c.kind {
                CraftCommitKind::Assign => CraftPhase::Picker,
                CraftCommitKind::Start => CraftPhase::Choose,
                CraftCommitKind::ConfirmPlan => CraftPhase::Confirm,
            };
            if c.slot != j.slot || j.goods.as_ref() != Some(&c.goods) || self.phase != phase {
                return Err("制造提交不属于当前台位与商品");
            }
        }
        if self.phase == CraftPhase::Confirm
            && self.job.as_ref().is_none_or(|j| {
                !self.commits.iter().any(|c| {
                    c.slot == j.slot
                        && j.goods.as_ref() == Some(&c.goods)
                        && c.kind == CraftCommitKind::Start
                        && c.state == CommitState::Observed
                })
            })
            || self.results.iter().any(|r| {
                r.outcome == CraftOutcome::Started
                    && !self.commits.iter().any(|c| {
                        c.slot == r.slot
                            && c.goods == r.goods
                            && matches!(
                                c.kind,
                                CraftCommitKind::Start | CraftCommitKind::ConfirmPlan
                            )
                            && c.state == CommitState::Observed
                    })
            })
        {
            return Err("制造缺少本次提交或实际运行后态");
        }
        Ok(())
    }
    pub fn completed(&self) -> bool {
        self.phase == CraftPhase::Done && self.validate().is_ok()
    }
    pub fn check_update(&self, next: &Self) -> Result<(), &'static str> {
        self.validate()?;
        next.validate()?;
        if self.config != next.config
            || self.mode != next.mode
            || self.material != next.material
            || !next.chosen.starts_with(&self.chosen)
            || next.chosen.len() > self.chosen.len() + 1
            || !next.results.starts_with(&self.results)
            || next.results.len() > self.results.len() + 1
            || next.commits.len() < self.commits.len()
            || next.commits.len() > self.commits.len() + 1
            || (self.overview_checked && !next.overview_checked)
            || (self.completed() && self != next)
        {
            return Err("不能改写制造冻结设置或历史证明");
        }
        for (old, new) in self.commits.iter().zip(&next.commits) {
            let mut expected = old.clone();
            expected.state = new.state;
            if expected != *new || (old.state != CommitState::Intent && old != new) {
                return Err("制造回执不能改写");
            }
        }
        if next.commits.len() > self.commits.len()
            && (self.pending.is_some()
                || next
                    .commits
                    .last()
                    .is_none_or(|c| c.state != CommitState::Intent))
        {
            return Err("制造必须先保存提交意图");
        }
        if next.results.len() > self.results.len() {
            let r = next.results.last().unwrap();
            if self
                .job
                .as_ref()
                .is_none_or(|j| j.slot != r.slot || j.goods.as_ref() != Some(&r.goods))
            {
                return Err("制造结果不是本次目标");
            }
            if r.outcome == CraftOutcome::Started
                && !self.pending.is_some_and(|i| {
                    matches!(
                        self.commits[usize::from(i)].kind,
                        CraftCommitKind::Start | CraftCommitKind::ConfirmPlan
                    ) && next.commits[usize::from(i)].state == CommitState::Observed
                })
            {
                return Err("制造成功没有本次提交及运行后态");
            }
        }
        if let (Some(old), Some(new)) = (&self.job, &next.job)
            && (old.slot != new.slot || old.scrolls > new.scrolls || old.candidate > new.candidate)
        {
            return Err("不能替换制造台位或撤回搜索次数");
        }
        Ok(())
    }
    pub fn prepare(&mut self, action: &CraftAction) -> Result<(), &'static str> {
        self.validate()?;
        let Some(kind) = action.kind() else {
            return Ok(());
        };
        let j = self.job.as_ref().ok_or("制造未选择台位")?;
        let goods = j.goods.clone().ok_or("制造未选择商品")?;
        let phase = match kind {
            CraftCommitKind::Assign => CraftPhase::Picker,
            CraftCommitKind::Start => CraftPhase::Choose,
            CraftCommitKind::ConfirmPlan => CraftPhase::Confirm,
        };
        if self.phase != phase
            || self.pending.is_some()
            || self.commits.len() >= 128
            || (kind == CraftCommitKind::Start
                && self.chosen.len() >= 128
                && !self.chosen.contains(&goods))
            || (!j.owned && kind != CraftCommitKind::Assign)
            || matches!(action, CraftAction::Start(0))
            || matches!(action,CraftAction::Assign(n) if !(2..=8).contains(n)||*n!=j.candidate)
        {
            return Err("制造提交阶段或选择证明不符");
        }
        let bangboo = if let CraftAction::Assign(n) = action {
            Some(*n)
        } else {
            None
        };
        let amount = if let CraftAction::Start(n) = action {
            Some(*n)
        } else {
            None
        };
        if kind == CraftCommitKind::Start && !self.chosen.contains(&goods) {
            self.chosen.push(goods.clone());
        }
        self.pending = Some(self.commits.len() as u8);
        self.commits.push(CraftCommit {
            kind,
            slot: j.slot,
            goods,
            bangboo,
            amount,
            state: CommitState::Intent,
        });
        self.validate()
    }
    pub fn not_posted(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if let Some(i) = self.pending.take() {
            self.commits[usize::from(i)].state = CommitState::NotPosted;
        }
        self.validate()
    }
    fn observed(&mut self) {
        let i = self.pending.take().expect("制造提交存在");
        self.commits[usize::from(i)].state = CommitState::Observed;
    }
    pub fn allows(&self, action: &CraftAction, view: &CraftView) -> bool {
        let mut f = self.clone();
        if let Some(i) = f.pending.take() {
            if action.kind() != Some(f.commits[usize::from(i)].kind) {
                return false;
            }
            f.commits.pop();
        } else if action.kind().is_some() {
            return false;
        }
        f.next(view).is_ok_and(|a| a.as_ref() == Some(action))
    }
    pub fn posted_navigation(&mut self, action: &CraftAction) -> Result<(), &'static str> {
        if *action == CraftAction::Scroll {
            let j = self.job.as_mut().ok_or("没有制造商品列表")?;
            if j.scrolls >= self.config.craft_drag_times {
                return Err("制造下拉次数超限");
            }
            j.scrolls += 1;
            j.after_scroll = true;
            j.unchanged_reads = 0;
        }
        self.validate()
    }
    fn finish(&mut self, outcome: CraftOutcome) -> Result<(), &'static str> {
        let j = self.job.take().ok_or("制造结果无台位")?;
        self.results.push(CraftResult {
            slot: j.slot,
            goods: j.goods.ok_or("制造结果缺少商品")?,
            outcome,
        });
        self.phase = if outcome == CraftOutcome::Started || self.mode == CraftMode::Material {
            CraftPhase::ReturnJob
        } else {
            CraftPhase::ReturnEntry
        };
        Ok(())
    }
    pub fn next(&mut self, view: &CraftView) -> Result<Option<CraftAction>, &'static str> {
        self.validate()?;
        let before = self.clone();
        let result = self.advance(view);
        if result.is_err() {
            *self = before;
            return result;
        }
        if let Err(e) = before.check_update(self) {
            *self = before;
            return Err(e);
        }
        result
    }
    fn advance(&mut self, view: &CraftView) -> Result<Option<CraftAction>, &'static str> {
        use CraftAction as A;
        use CraftPhase as P;
        use CraftView as V;
        if matches!(view, V::Unknown) {
            return Ok(None);
        }
        if self.completed() {
            return Ok(None);
        }
        if let Some(index) = self.pending {
            let c = self.commits[usize::from(index)].clone();
            let running = match view {
                V::Running { slot, goods } => *slot == c.slot && *goods == c.goods,
                V::Overview(slots) => slots.iter().any(|s| {
                    s.slot == c.slot
                        && s.state == SlotState::Running
                        && s.goods.as_ref() == Some(&c.goods)
                }),
                _ => false,
            };
            match (c.kind, view) {
                (CraftCommitKind::Assign, V::Selection(p))
                    if p.slot == c.slot
                        && p.goods == c.goods
                        && p.bangboo == CraftBangboo::Ready =>
                {
                    self.observed();
                    self.phase = P::Choose;
                }
                (CraftCommitKind::Assign, V::AssignRejected) => {
                    self.observed();
                    self.job.as_mut().unwrap().candidate = c.bangboo.ok_or("派驻缺少邦布")? + 1;
                }
                (CraftCommitKind::Start, V::PlanConfirm) => {
                    self.observed();
                    self.phase = P::Confirm;
                }
                (CraftCommitKind::Start | CraftCommitKind::ConfirmPlan, _) if running => {
                    self.observed();
                    self.finish(CraftOutcome::Started)?;
                }
                _ => return Ok(None),
            }
            return Ok(None);
        }
        for _ in 0..6 {
            match self.phase {
                P::Inspect => match view {
                    V::Managed => return Err("托管中不能进入手动制造"),
                    V::Entry if self.mode == CraftMode::Workshop => {
                        return Ok(Some(A::OpenManagement));
                    }
                    V::Management if self.mode == CraftMode::Workshop => {
                        return Ok(Some(A::OpenWorkshop));
                    }
                    V::Overview(_) if self.mode == CraftMode::Workshop => self.phase = P::Open,
                    V::Selection(p) => {
                        if !(1..=4).contains(&p.slot)
                            || !valid_goods(&p.goods)
                            || self.material.as_ref().is_some_and(|m| m != &p.goods)
                        {
                            return Err("制造不是父流程指定的商品或台位");
                        }
                        self.job = Some(CraftJob::new(p.slot));
                        self.phase = P::Choose;
                    }
                    _ => return Err("制造起点不是入口、制造坊或指定材料制造页"),
                },
                P::Open => {
                    if let V::Overview(slots) = view {
                        if slots.len() != 4
                            || slots.iter().enumerate().any(|(i, s)| s.slot != i as u8 + 1)
                        {
                            return Err("制造台位未按四个固定位置完整读取");
                        }
                        if let Some(s) = slots.iter().find(|s| {
                            matches!(s.state, SlotState::Open | SlotState::Paused)
                                && !self.results.iter().any(|r| r.slot == s.slot)
                        }) {
                            self.job = Some(CraftJob::new(s.slot));
                            self.phase = P::Choose;
                            return Ok(Some(A::OpenSlot(s.slot)));
                        } else if slots.iter().all(|s| s.state != SlotState::Unknown) {
                            self.overview_checked = true;
                            self.phase = P::ReturnEntry;
                        } else {
                            return Ok(None);
                        }
                    } else {
                        return Ok(None);
                    }
                }
                P::Choose => match view {
                    V::Overview(slots) => {
                        let slot = self.job.as_ref().unwrap().slot;
                        return Ok(slots
                            .iter()
                            .any(|s| {
                                s.slot == slot
                                    && matches!(s.state, SlotState::Open | SlotState::Paused)
                            })
                            .then_some(A::OpenSlot(slot)));
                    }
                    V::Selection(p) => {
                        let j = self.job.as_mut().unwrap();
                        if p.slot != j.slot || !valid_goods(&p.goods) {
                            return Err("制造商品页台位或名称不符");
                        }
                        if let Some(wanted) = j.wanted.clone() {
                            if wanted != p.goods {
                                return Ok(p
                                    .rows
                                    .iter()
                                    .any(|g| g.name == wanted && g.white && g.makeable)
                                    .then_some(A::SelectGood(wanted)));
                            }
                            j.goods = Some(wanted);
                            j.wanted = None;
                            j.owned = true;
                        }
                        if j.goods.is_none() {
                            j.goods = Some(p.goods.clone());
                            j.owned =
                                self.mode == CraftMode::Material || !self.chosen.contains(&p.goods);
                            // 先保存本次目标；下一新帧才形成输入或无工作结果。
                            return Ok(None);
                        }
                        if j.goods.as_ref() != Some(&p.goods) {
                            return Err("已选制造商品发生变化");
                        }
                        match p.bangboo {
                            CraftBangboo::Missing | CraftBangboo::Exhausted => {
                                self.phase = P::Picker;
                                return Ok(Some(A::OpenPicker));
                            }
                            CraftBangboo::Unknown => return Ok(None),
                            CraftBangboo::Ready => {}
                        }
                        if p.materials == Some(true) && j.owned {
                            return Ok((p.can_start && p.amount.is_some_and(|n| n > 0))
                                .then(|| A::Start(p.amount.unwrap())));
                        }
                        if p.materials.is_none() {
                            return Ok(None);
                        }
                        if self.mode == CraftMode::Material {
                            self.finish(CraftOutcome::NoMaterials)?;
                            return Ok(None);
                        }
                        if !p.list_readable {
                            return Ok(None);
                        }
                        if p.rows.len() > 32 || p.rows.iter().any(|g| !valid_goods(&g.name)) {
                            return Err("制造商品列表无效");
                        }
                        if let Some(g) = p.rows.iter().find(|g| {
                            g.white
                                && g.makeable
                                && !self.chosen.contains(&g.name)
                                && g.name != p.goods
                        }) {
                            self.chosen.push(g.name.clone());
                            j.wanted = Some(g.name.clone());
                            j.after_scroll = false;
                            j.unchanged_reads = 0;
                            return Ok(Some(A::SelectGood(g.name.clone())));
                        }
                        let names: Vec<_> = p.rows.iter().map(|g| g.name.clone()).collect();
                        if j.after_scroll && j.last_names == names {
                            j.unchanged_reads = (j.unchanged_reads + 1).min(2);
                            if j.unchanged_reads < 2 {
                                return Ok(None);
                            }
                            self.finish(CraftOutcome::NoGoods)?;
                            return Ok(None);
                        }
                        j.after_scroll = false;
                        j.last_names = names;
                        if j.scrolls >= self.config.craft_drag_times {
                            self.finish(CraftOutcome::SearchLimit)?;
                            return Ok(None);
                        }
                        return Ok(Some(A::Scroll));
                    }
                    _ => return Ok(None),
                },
                P::Picker => match view {
                    V::Selection(p)
                        if self.job.as_ref().is_some_and(|j| {
                            j.slot == p.slot && j.goods.as_ref() == Some(&p.goods)
                        }) =>
                    {
                        if p.bangboo == CraftBangboo::Ready {
                            self.phase = P::Choose;
                            continue;
                        }
                        return Ok(Some(A::OpenPicker));
                    }
                    V::Picker { cards, selected } => {
                        let j = self.job.as_mut().unwrap();
                        while j.candidate <= 8 {
                            match cards[usize::from(j.candidate - 1)] {
                                CraftCard::Busy | CraftCard::Exhausted => j.candidate += 1,
                                CraftCard::Unknown => return Ok(None),
                                CraftCard::Available => {
                                    return Ok(Some(if *selected == Some(j.candidate) {
                                        A::Assign(j.candidate)
                                    } else {
                                        A::SelectBangboo(j.candidate)
                                    }));
                                }
                            }
                        }
                        self.finish(CraftOutcome::NoBangboo)?;
                        return Ok(None);
                    }
                    V::AssignRejected
                        if self.commits.last().is_some_and(|c| {
                            c.kind == CraftCommitKind::Assign && c.state == CommitState::Observed
                        }) =>
                    {
                        return Ok(Some(A::Back));
                    }
                    _ => return Ok(None),
                },
                P::Confirm => return Ok(matches!(view, V::PlanConfirm).then_some(A::ConfirmPlan)),
                P::ReturnJob => {
                    if self.mode == CraftMode::Material {
                        if let V::MaterialMenu(name) = view {
                            if self.material.as_ref() != Some(name) {
                                return Err("制造返回了其他材料");
                            }
                            self.phase = P::Done;
                            self.returned_material = true;
                            return Ok(None);
                        }
                        if matches!(
                            view,
                            V::Selection(_) | V::Picker { .. } | V::Overview(_) | V::Running { .. }
                        ) {
                            return Ok(Some(A::Back));
                        }
                        return Ok(None);
                    } else {
                        if matches!(view, V::Overview(_)) {
                            self.phase = P::Open;
                            continue;
                        }
                        if matches!(view, V::Selection(_) | V::Picker { .. } | V::Running { .. }) {
                            return Ok(Some(A::Back));
                        }
                        return Ok(None);
                    }
                }
                P::ReturnEntry => match view {
                    V::Entry => {
                        self.returned_entry = true;
                        self.phase = P::Done;
                        return Ok(None);
                    }
                    V::Overview(_)
                    | V::Selection(_)
                    | V::Picker { .. }
                    | V::Management
                    | V::Running { .. } => return Ok(Some(A::Back)),
                    _ => return Ok(None),
                },
                P::Done => return Ok(None),
            }
        }
        Err("制造单帧推进超过上限")
    }
}
