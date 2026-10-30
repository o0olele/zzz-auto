//! 武备逐槽读取、复核和携带；首槽身份用于同层 NPC 判重。
use crate::{
    lost_void::{
        ArtifactCandidate, ArtifactSelection, LostVoidChallenge, artifacts, choose_artifacts,
    },
    lost_void_choice::{LostVoidChoiceReceipt, LostVoidChoiceRequest},
    lost_void_scene::{LostVoidObservation, LostVoidScreen as S},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearSlot {
    pub slot: u8,
    pub owned: bool,
    pub signature: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearPage {
    pub slots: Vec<GearSlot>,
    pub name: Option<ArtifactCandidate>,
    pub carry_enabled: bool,
    pub back: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidGearView {
    pub scene: LostVoidObservation,
    pub page: Option<GearPage>,
}
pub fn valid_gear(c: &ArtifactCandidate) -> bool {
    artifacts().is_ok_and(|list| {
        list.iter().any(|a| {
            a.is_gear && a.category == c.category && a.name == c.name && a.level == c.level
        })
    }) && c.is_primary
        && !c.is_new
}
fn same_gear(a: &ArtifactCandidate, b: &ArtifactCandidate) -> bool {
    a.category == b.category && a.name == b.name && a.level == b.level
}
impl GearPage {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.slots.is_empty()
            || self.slots.len() > 12
            || self.slots.iter().enumerate().any(|(i, s)| {
                s.slot != i as u8
                    || s.signature.len() != 64
                    || !s.signature.bytes().all(|c| c.is_ascii_hexdigit())
            })
            || self
                .name
                .as_ref()
                .is_some_and(|c| c.slot != 0 || !valid_gear(c))
        {
            return Err("迷失之地武备槽位或名称无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidGearRequest {
    pub run_id: Uuid,
    pub interaction_id: Uuid,
    pub challenge: LostVoidChallenge,
    pub options: ArtifactSelection,
    pub entry: bool,
    pub completed_first: Vec<ArtifactCandidate>,
    pub timeout_ms: u32,
}
impl LostVoidGearRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        LostVoidChoiceRequest {
            run_id: self.run_id,
            interaction_id: self.interaction_id,
            challenge: self.challenge.clone(),
            options: self.options.clone(),
            return_to: vec![S::World],
            timeout_ms: self.timeout_ms,
        }
        .validate()?;
        if self.completed_first.len() > 2
            || (!self.entry && !self.completed_first.is_empty())
            || self
                .completed_first
                .iter()
                .any(|a| a.slot != 0 || !valid_gear(a))
            || self
                .completed_first
                .iter()
                .enumerate()
                .any(|(i, a)| self.completed_first[..i].iter().any(|b| same_gear(a, b)))
        {
            return Err("迷失之地武备入口完成名单无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GearAction {
    View { slot: u8, pass: u8 },
    Carry,
    Confirm,
    Back,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GearStep {
    Input(GearAction),
    Choice,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    Inspect,
    Scan,
    Verify,
    Select,
    Carry,
    Confirm,
    Exit,
    Done,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    action: GearAction,
    posted: bool,
    before: Option<ArtifactCandidate>,
    stable: Option<ArtifactCandidate>,
    reads: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidGearReceipt {
    pub run_id: Uuid,
    pub interaction_id: Uuid,
    pub returned_to: S,
    pub first: Option<ArtifactCandidate>,
    pub selected: Option<ArtifactCandidate>,
    pub repeated: bool,
    pub append_priority: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidGearFlow {
    request: LostVoidGearRequest,
    stage: Stage,
    layout: Vec<GearSlot>,
    names: Vec<ArtifactCandidate>,
    index: u8,
    selected: Option<ArtifactCandidate>,
    pending: Option<Pending>,
    last: Option<GearPage>,
    ack: bool,
    repeated: bool,
    child: Option<Uuid>,
    returned: bool,
    receipt: Option<LostVoidGearReceipt>,
}
impl LostVoidGearFlow {
    pub fn new(request: LostVoidGearRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            stage: Stage::Inspect,
            layout: vec![],
            names: vec![],
            index: 0,
            selected: None,
            pending: None,
            last: None,
            ack: false,
            repeated: false,
            child: None,
            returned: false,
            receipt: None,
        })
    }
    pub fn request(&self) -> &LostVoidGearRequest {
        &self.request
    }
    pub fn completed(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn receipt(&self) -> Option<&LostVoidGearReceipt> {
        self.receipt.as_ref()
    }
    fn same_layout(&self, p: &GearPage) -> bool {
        self.layout.len() == p.slots.len()
            && self
                .layout
                .iter()
                .zip(&p.slots)
                .all(|(a, b)| a.slot == b.slot && a.signature == b.signature)
    }
    fn phase_allows(&self, a: GearAction) -> bool {
        match (self.stage, a) {
            (Stage::Scan, GearAction::View { slot, pass: 0 })
            | (Stage::Verify, GearAction::View { slot, pass: 1 }) => slot == self.index,
            (Stage::Select, GearAction::View { slot, pass: 2 }) => {
                self.selected.as_ref().is_some_and(|a| a.slot == slot)
            }
            (Stage::Carry, GearAction::Carry)
            | (Stage::Confirm, GearAction::Confirm)
            | (Stage::Exit, GearAction::Back) => true,
            _ => false,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if !self.layout.is_empty() {
            GearPage {
                slots: self.layout.clone(),
                name: None,
                carry_enabled: false,
                back: false,
            }
            .validate()?;
        }
        if self.names.len() > self.layout.len()
            || self.index as usize > self.layout.len()
            || self.names.iter().enumerate().any(|(i, a)| {
                a.slot != i as u8
                    || !valid_gear(a)
                    || self.names[..i].iter().any(|b| same_gear(a, b))
            })
            || self
                .selected
                .as_ref()
                .is_some_and(|a| !self.names.contains(a))
            || self.completed() != self.receipt.is_some()
            || self.pending.as_ref().is_some_and(|p| {
                !self.phase_allows(p.action)
                    || p.reads > 2
                    || (p.reads == 0) != p.stable.is_none()
                    || p.before.as_ref().is_some_and(|a| !valid_gear(a))
                    || p.stable.as_ref().is_some_and(|a| !valid_gear(a))
            })
            || (self.stage == Stage::Inspect
                && (!self.layout.is_empty() || !self.names.is_empty() || self.index != 0))
            || (matches!(self.stage, Stage::Scan | Stage::Verify)
                && self.index as usize >= self.layout.len())
            || (matches!(
                self.stage,
                Stage::Verify | Stage::Select | Stage::Carry | Stage::Confirm
            ) && self.names.len() != self.layout.len())
            || (matches!(self.stage, Stage::Select | Stage::Carry | Stage::Confirm)
                && self.selected.is_none())
            || (self.ack && self.selected.is_none())
            || (self.returned && !matches!(self.stage, Stage::Exit | Stage::Done))
            || (self.repeated && (!self.request.entry || self.selected.is_some() || self.ack))
            || self.child.is_some_and(|id| {
                id.is_nil()
                    || !self.pending.as_ref().is_some_and(|p| {
                        matches!(p.action, GearAction::Carry | GearAction::Confirm)
                    })
            })
            || self.receipt.as_ref().is_some_and(|r| {
                r.run_id != self.request.run_id
                    || ![S::World, S::Event].contains(&r.returned_to)
                    || r.interaction_id != self.request.interaction_id
                    || r.repeated != self.repeated
                    || r.selected != self.selected
                    || r.first != self.names.first().cloned()
                    || !self.returned
                    || r.append_priority
                        != self.selected.as_ref().is_some_and(|a| {
                            self.request.challenge.chase_new_mode
                                && !self.layout[a.slot as usize].owned
                        })
            })
        {
            return Err("迷失之地武备检查点不一致");
        }
        if let Some(p) = &self.last {
            p.validate()?;
        }
        Ok(())
    }
    fn finish(&mut self, returned_to: S) {
        self.stage = Stage::Done;
        self.returned = true;
        self.pending = None;
        self.child = None;
        self.receipt = Some(LostVoidGearReceipt {
            run_id: self.request.run_id,
            interaction_id: self.request.interaction_id,
            returned_to,
            first: self.names.first().cloned(),
            selected: self.selected.clone(),
            repeated: self.repeated,
            append_priority: self.selected.as_ref().is_some_and(|a| {
                self.request.challenge.chase_new_mode && !self.layout[a.slot as usize].owned
            }),
        });
    }
    fn choose(&mut self) -> Result<(), &'static str> {
        let mut candidates = self.names.clone();
        if self.request.challenge.chase_new_mode && self.layout.iter().any(|s| !s.owned) {
            candidates.retain(|a| !self.layout[a.slot as usize].owned);
        }
        let mut options = self.request.options.clone();
        options.fallback = true;
        let result = choose_artifacts(&self.request.challenge, &candidates, 1, &options)?;
        let slot = result.first().ok_or("迷失之地没有可选择的武备")?.slot;
        self.selected = self.names.iter().find(|a| a.slot == slot).cloned();
        self.stage = Stage::Select;
        Ok(())
    }
    pub fn next(&mut self, v: &LostVoidGearView) -> Result<Option<GearStep>, &'static str> {
        self.validate()?;
        if self.completed() {
            return Ok(None);
        }
        if v.scene.screen == S::Unknown {
            return Ok(None);
        }
        if [S::World, S::Event].contains(&v.scene.screen) {
            let posted_exit = self.pending.as_ref().is_some_and(|p| {
                matches!(
                    p.action,
                    GearAction::Back | GearAction::Carry | GearAction::Confirm
                )
            });
            if (self.repeated || self.selected.is_some())
                && (posted_exit || self.ack || self.returned)
            {
                self.ack = !self.repeated;
                self.finish(v.scene.screen);
            }
            return Ok(None);
        }
        if v.scene.screen == S::CommonChoice {
            if !self
                .pending
                .as_ref()
                .is_some_and(|p| matches!(p.action, GearAction::Carry | GearAction::Confirm))
            {
                return Err("迷失之地武备获得物缺少本次携带意图");
            }
            if self.child.is_none() {
                self.child = Some(Uuid::new_v4());
            }
            return Ok(Some(GearStep::Choice));
        }
        if v.scene.screen == S::GearConfirm {
            if self
                .pending
                .as_ref()
                .is_some_and(|p| p.action == GearAction::Carry)
            {
                self.pending = None;
                self.stage = Stage::Confirm;
            }
            if self.stage != Stage::Confirm {
                return Err("迷失之地不能采用已有武备确认框");
            }
            return Ok(self
                .pending
                .is_none()
                .then_some(GearStep::Input(GearAction::Confirm)));
        }
        if v.scene.screen != S::Gear {
            return Err("迷失之地武备遇到非预期页面");
        }
        let Some(p) = &v.page else { return Ok(None) };
        p.validate()?;
        if self.stage == Stage::Inspect {
            self.layout = p.slots.clone();
            self.stage = Stage::Scan;
            if self.request.entry && self.request.completed_first.len() == 2 {
                self.repeated = true;
                self.stage = Stage::Exit;
            }
        }
        if !self.same_layout(p) {
            return Err("迷失之地武备列表身份发生变化");
        }
        if matches!(
            self.stage,
            Stage::Scan | Stage::Verify | Stage::Select | Stage::Carry
        ) && self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.action != GearAction::Carry)
            && self.layout != p.slots
        {
            return Err("迷失之地武备获取状态发生变化");
        }
        if let Some(pending) = &mut self.pending {
            match pending.action {
                GearAction::View { slot, pass } => {
                    let Some(name) = &p.name else { return Ok(None) };
                    if pass != 0
                        && (self
                            .names
                            .get(slot as usize)
                            .is_none_or(|a| !same_gear(a, name))
                            || (self.layout.len() > 1
                                && pending
                                    .before
                                    .as_ref()
                                    .is_some_and(|before| same_gear(before, name))))
                    {
                        return Ok(None);
                    }
                    if pending
                        .stable
                        .as_ref()
                        .is_some_and(|old| same_gear(old, name))
                    {
                        pending.reads += 1;
                    } else {
                        pending.stable = Some(name.clone());
                        pending.reads = 1;
                    }
                    if pending.reads < 2 {
                        return Ok(None);
                    }
                    let mut name = name.clone();
                    name.slot = slot;
                    if pass == 0 {
                        if self.names.iter().any(|a| same_gear(a, &name)) {
                            return Err("迷失之地武备名称重复，不能确定槽位");
                        }
                        self.names.push(name);
                        self.index += 1;
                        if self.names.len() == self.layout.len() {
                            self.stage = Stage::Verify;
                            self.index = 0;
                        }
                    } else if pass == 1 {
                        if slot == 0
                            && self
                                .request
                                .completed_first
                                .iter()
                                .any(|a| same_gear(a, &name))
                        {
                            self.repeated = true;
                            self.stage = Stage::Exit;
                        } else {
                            self.index += 1;
                            if self.index as usize == self.layout.len() {
                                self.choose()?;
                            }
                        }
                    } else {
                        self.stage = Stage::Carry;
                    }
                    self.pending = None;
                }
                GearAction::Carry => {
                    if p.name
                        .as_ref()
                        .zip(self.selected.as_ref())
                        .is_some_and(|(a, b)| same_gear(a, b))
                        && !p.carry_enabled
                    {
                        self.ack = true;
                        self.pending = None;
                        self.stage = Stage::Exit;
                    } else {
                        return Ok(None);
                    }
                }
                GearAction::Confirm => {
                    self.ack = true;
                    self.pending = None;
                    self.stage = Stage::Exit;
                }
                GearAction::Back => return Ok(None),
            }
        }
        self.last = Some(p.clone());
        if self.stage == Stage::Select
            && p.name
                .as_ref()
                .zip(self.selected.as_ref())
                .is_some_and(|(a, b)| same_gear(a, b))
        {
            self.stage = Stage::Carry;
        }
        let action = match self.stage {
            Stage::Scan => GearAction::View {
                slot: self.index,
                pass: 0,
            },
            Stage::Verify => GearAction::View {
                slot: self.index,
                pass: 1,
            },
            Stage::Select => GearAction::View {
                slot: self.selected.as_ref().ok_or("迷失之地所选武备缺失")?.slot,
                pass: 2,
            },
            Stage::Carry if p.carry_enabled => GearAction::Carry,
            Stage::Exit if p.back => GearAction::Back,
            _ => return Ok(None),
        };
        Ok(Some(GearStep::Input(action)))
    }
    pub fn choice_request(&self) -> Result<LostVoidChoiceRequest, &'static str> {
        self.validate()?;
        Ok(LostVoidChoiceRequest {
            run_id: self.request.run_id,
            interaction_id: self.child.ok_or("迷失之地武备选择子流程身份缺失")?,
            challenge: self.request.challenge.clone(),
            options: self.request.options.clone(),
            return_to: vec![S::World, S::Event, S::Gear],
            timeout_ms: self.request.timeout_ms,
        })
    }
    pub fn accept_choice(&mut self, r: &LostVoidChoiceReceipt) -> Result<(), &'static str> {
        self.validate()?;
        if self.child != Some(r.interaction_id)
            || r.run_id != self.request.run_id
            || ![S::World, S::Event, S::Gear].contains(&r.returned_to)
            || !(1..=16).contains(&r.confirmed_pages)
        {
            return Err("迷失之地武备选择回执不属于本次交互");
        }
        self.child = None;
        self.ack = true;
        self.pending = None;
        self.stage = Stage::Exit;
        if [S::World, S::Event].contains(&r.returned_to) {
            self.finish(r.returned_to);
        }
        Ok(())
    }
    pub fn prepare(&mut self, a: GearAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some() || !self.phase_allows(a) {
            return Err("迷失之地没有待保存的武备动作");
        }
        if a == GearAction::Carry
            && self.last.as_ref().is_none_or(|p| {
                !p.carry_enabled
                    || !p
                        .name
                        .as_ref()
                        .zip(self.selected.as_ref())
                        .is_some_and(|(a, b)| same_gear(a, b))
            })
        {
            return Err("迷失之地携带动作缺少选中名称和可用按钮");
        }
        self.pending = Some(Pending {
            action: a,
            posted: false,
            before: self.last.as_ref().and_then(|p| p.name.clone()),
            stable: None,
            reads: 0,
        });
        Ok(())
    }
    pub fn allows(&self, a: GearAction, v: &LostVoidGearView) -> bool {
        if self.validate().is_err()
            || !self
                .pending
                .as_ref()
                .is_some_and(|p| p.action == a && !p.posted)
        {
            return false;
        }
        if a == GearAction::Confirm {
            return v.scene.screen == S::GearConfirm;
        }
        let Some(p) = &v.page else { return false };
        v.scene.screen == S::Gear
            && p.validate().is_ok()
            && self.same_layout(p)
            && match a {
                GearAction::View { slot, .. } => {
                    (slot as usize) < p.slots.len()
                        && p.name == self.pending.as_ref().and_then(|p| p.before.clone())
                }
                GearAction::Carry => {
                    p.carry_enabled
                        && p.name
                            .as_ref()
                            .zip(self.selected.as_ref())
                            .is_some_and(|(a, b)| same_gear(a, b))
                        && p.slots == self.layout
                }
                GearAction::Back => p.back && (self.repeated || self.ack),
                GearAction::Confirm => false,
            }
    }
    pub fn posted(&mut self, a: GearAction) -> Result<(), &'static str> {
        let p = self.pending.as_mut().ok_or("迷失之地武备动作没有意图")?;
        if p.action != a || p.posted {
            return Err("迷失之地武备投递回执不匹配");
        }
        p.posted = true;
        self.validate()
    }
    pub fn not_posted(&mut self, a: GearAction) -> Result<(), &'static str> {
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| p.action == a && !p.posted)
        {
            return Err("迷失之地已投递武备动作不能撤销");
        }
        self.pending = None;
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.completed() {
            self.receipt = None;
            self.stage = Stage::Exit;
        }
        Ok(())
    }
}
