//! 本次出口、明确离开及新帧实际队伍共同确认跨层；等待期间不产生输入。
use crate::{
    lost_void_event::{EventPage, EventPageKind, LostVoidEventReceipt, challenge_confirmation},
    lost_void_loading::LoadingObservation,
    lost_void_navigation::{NavOutcome, NavReceipt},
    lost_void_npc::NpcTarget,
    lost_void_roster::{RosterMember, RosterReceipt, validate_members},
    lost_void_scene::LostVoidScreen as S,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn event_screen(screen: S) -> bool {
    matches!(
        screen,
        S::Event
            | S::CommonChoice
            | S::Gear
            | S::GearConfirm
            | S::Store
            | S::Lottery
            | S::RouteChange
    )
}
fn result_screen(screen: S) -> bool {
    matches!(screen, S::BattleResult | S::BattleFail)
}
fn matching_challenge(region: &str, page: &EventPage) -> bool {
    let text =
        crate::lost_void_event::dialogue_text(&page.text).replace(['·', '：', ':', '－'], "-");
    page.kind == EventPageKind::ChallengeConfirm
        && challenge_confirmation(&page.text)
        && ["挑战-限时", "挑战-无伤", "挑战-收割"]
            .iter()
            .all(|named| !text.contains(named) || *named == region)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "proof",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum DepartureProof {
    Loading(LoadingObservation),
    ChallengeConfirmed {
        interaction_id: Uuid,
        page: EventPage,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionRequest {
    pub roster: RosterReceipt,
    pub exit: NavReceipt,
    pub next_level_id: Uuid,
    pub timeout_ms: u32,
}
impl TransitionRequest {
    pub fn region(&self) -> Result<&str, &'static str> {
        match &self.exit.outcome {
            NavOutcome::InteractionSubmitted {
                target: NpcTarget::Entry(region),
            } => Ok(region),
            _ => Err("跨层缺少本次门牌及交互意图"),
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let r = &self.roster;
        let e = &self.exit;
        validate_members(&r.members, r.members.len() as u8)?;
        let region = self.region()?;
        if r.run_id.is_nil()
            || r.level_id.is_nil()
            || e.run_id != r.run_id
            || e.level_id != r.level_id
            || e.interaction_id.is_nil()
            || self.next_level_id.is_nil()
            || self.next_level_id == r.level_id
            || e.next_region_hint.as_deref() != Some(region)
            || !crate::lost_void::REGIONS.contains(&region)
            || !r.members.iter().any(|m| m.id == r.front)
            || !r
                .plan
                .verify_roster(&r.members.iter().map(|m| m.id.clone()).collect::<Vec<_>>())?
            || !(1000..=300_000).contains(&self.timeout_ms)
            || !(e.screen == S::World
                || event_screen(e.screen)
                || result_screen(e.screen)
                || e.screen == S::Modal)
        {
            return Err("跨层运行、队伍、出口或预算身份无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArrivalProof {
    pub level_id: Uuid,
    pub region: String,
    pub members: Vec<RosterMember>,
    pub front: String,
    pub departure: DepartureProof,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "proof",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TransitionOutcome {
    Arrived(ArrivalProof),
    Event { screen: S },
    Result { screen: S },
    SameLevelEvent,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionReceipt {
    pub run_id: Uuid,
    pub level_id: Uuid,
    pub interaction_id: Uuid,
    pub outcome: TransitionOutcome,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionView {
    pub screen: S,
    pub loading: Option<LoadingObservation>,
    pub members: Option<Vec<RosterMember>>,
}
impl TransitionView {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(loading) = &self.loading {
            loading.validate()?;
            if self.screen != S::Unknown || self.members.is_some() {
                return Err("加载观察不能充当世界到达");
            }
        }
        if let Some(members) = &self.members {
            validate_members(members, members.len() as u8)?;
            if self.screen != S::World {
                return Err("到达队伍必须来自迷失之地世界");
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionFlow {
    request: TransitionRequest,
    departure: Option<DepartureProof>,
    world: Option<Vec<RosterMember>>,
    world_reads: u8,
    content: Option<(S, u8)>,
    accepted_event: Option<LostVoidEventReceipt>,
    event_visits: u16,
    receipt: Option<TransitionReceipt>,
}
impl TransitionFlow {
    pub fn new(
        request: TransitionRequest,
        loading: Option<LoadingObservation>,
    ) -> Result<Self, &'static str> {
        request.validate()?;
        let state = Self {
            request,
            departure: loading.map(DepartureProof::Loading),
            world: None,
            world_reads: 0,
            content: None,
            accepted_event: None,
            event_visits: 0,
            receipt: None,
        };
        state.validate()?;
        Ok(state)
    }
    pub fn request(&self) -> &TransitionRequest {
        &self.request
    }
    pub fn departure(&self) -> Option<&DepartureProof> {
        self.departure.as_ref()
    }
    pub fn receipt(&self) -> Option<&TransitionReceipt> {
        self.receipt.as_ref()
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    fn same_members(&self, members: &[RosterMember]) -> bool {
        members.len() == self.request.roster.members.len()
            && validate_members(members, members.len() as u8).is_ok()
            && members
                .iter()
                .all(|m| self.request.roster.members.contains(m))
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if let Some(departure) = &self.departure {
            match departure {
                DepartureProof::Loading(p) => p.validate()?,
                DepartureProof::ChallengeConfirmed {
                    interaction_id,
                    page,
                } => {
                    page.validate()?;
                    if *interaction_id != self.request.exit.interaction_id
                        || !self.request.region()?.starts_with("挑战-")
                        || self.accepted_event.is_none()
                        || !matching_challenge(self.request.region()?, page)
                    {
                        return Err("跨层挑战确认与本次出口事件不符");
                    }
                }
            }
        }
        if self.world_reads > 2
            || self.world.is_some() != (self.world_reads > 0)
            || self.world.as_ref().is_some_and(|m| !self.same_members(m))
            || self.content.is_some_and(|(s, n)| {
                !(1..=2).contains(&n) || !(event_screen(s) || result_screen(s))
            })
            || self.world.is_some() && self.content.is_some()
            || self.event_visits > 128
            || self.accepted_event.is_some() != (self.event_visits > 0)
        {
            return Err("跨层稳定计数或交互历史无效");
        }
        if let Some(event) = &self.accepted_event {
            self.validate_event(event)?;
        }
        if let Some(receipt) = &self.receipt {
            if receipt.run_id != self.request.exit.run_id
                || receipt.level_id != self.request.exit.level_id
                || receipt.interaction_id != self.request.exit.interaction_id
            {
                return Err("跨层回执身份无效");
            }
            let valid = match &receipt.outcome {
                TransitionOutcome::Arrived(p) => {
                    self.world_reads == 2
                        && self.world.as_ref() == Some(&p.members)
                        && p.level_id == self.request.next_level_id
                        && p.region == self.request.region()?
                        && p.members.first().is_some_and(|m| m.id == p.front)
                        && self.departure.as_ref() == Some(&p.departure)
                }
                TransitionOutcome::Event { screen } => {
                    event_screen(*screen) && self.content == Some((*screen, 2))
                }
                TransitionOutcome::Result { screen } => {
                    result_screen(*screen) && self.content == Some((*screen, 2))
                }
                TransitionOutcome::SameLevelEvent => {
                    self.accepted_event.is_some()
                        && self.departure.is_none()
                        && self.world_reads == 2
                }
            };
            if !valid {
                return Err("跨层回执缺少对应的新帧后态");
            }
        }
        Ok(())
    }
    fn validate_event(&self, event: &LostVoidEventReceipt) -> Result<(), &'static str> {
        if event.run_id != self.request.exit.run_id
            || event.interaction_id != self.request.exit.interaction_id
            || event.returned_to != S::World
            || event.actions == 0 && event.children == 0
            || event.actions > 128
            || event.children > 128
            || event.gear_repeats > 128
            || event.completed_first.len() > 2
            || event.completed_first.iter().enumerate().any(|(i, a)| {
                a.slot != 0
                    || !crate::lost_void_gear::valid_gear(a)
                    || event.completed_first[..i].contains(a)
            })
        {
            return Err("跨层事件未安全返回本次世界");
        }
        Ok(())
    }
    fn finish(&mut self, outcome: TransitionOutcome) {
        self.receipt = Some(TransitionReceipt {
            run_id: self.request.exit.run_id,
            level_id: self.request.exit.level_id,
            interaction_id: self.request.exit.interaction_id,
            outcome,
        });
    }
    pub fn next(&mut self, view: &TransitionView) -> Result<(), &'static str> {
        let mut next = self.clone();
        next.next_inner(view)?;
        next.validate()?;
        *self = next;
        Ok(())
    }
    fn next_inner(&mut self, view: &TransitionView) -> Result<(), &'static str> {
        self.validate()?;
        view.validate()?;
        if self.completed() {
            return Ok(());
        }
        if let Some(loading) = &view.loading {
            self.departure = Some(DepartureProof::Loading(loading.clone()));
            self.invalidate_observation();
            return Ok(());
        }
        if event_screen(view.screen) || result_screen(view.screen) {
            self.world = None;
            self.world_reads = 0;
            let n = if self.content.is_some_and(|(s, _)| s == view.screen) {
                (self.content.unwrap().1 + 1).min(2)
            } else {
                1
            };
            self.content = Some((view.screen, n));
            if n == 2 {
                self.finish(if event_screen(view.screen) {
                    TransitionOutcome::Event {
                        screen: view.screen,
                    }
                } else {
                    TransitionOutcome::Result {
                        screen: view.screen,
                    }
                });
            }
            return Ok(());
        }
        self.content = None;
        if view.screen != S::World || view.members.is_none() {
            self.world = None;
            self.world_reads = 0;
            return Ok(());
        }
        let members = view.members.as_ref().unwrap();
        if !self.same_members(members) {
            return Err("到达世界的完整队伍与本次出口前不一致");
        }
        if self.departure.is_none() && self.accepted_event.is_none() {
            self.world = None;
            self.world_reads = 0;
            return Ok(());
        }
        self.world_reads = if self.world.as_ref() == Some(members) {
            (self.world_reads + 1).min(2)
        } else {
            1
        };
        self.world = Some(members.clone());
        if self.world_reads == 2 {
            if let Some(departure) = &self.departure {
                self.finish(TransitionOutcome::Arrived(ArrivalProof {
                    level_id: self.request.next_level_id,
                    region: self.request.region()?.into(),
                    members: members.clone(),
                    front: members[0].id.clone(),
                    departure: departure.clone(),
                }));
            } else {
                self.finish(TransitionOutcome::SameLevelEvent);
            }
        }
        Ok(())
    }
    pub fn record_loading(&mut self, loading: LoadingObservation) -> Result<(), &'static str> {
        self.validate()?;
        loading.validate()?;
        if self
            .receipt
            .as_ref()
            .is_some_and(|r| !matches!(r.outcome, TransitionOutcome::Event { .. }))
        {
            return Err("已经交回的跨层结果不能追加加载");
        }
        self.departure = Some(DepartureProof::Loading(loading));
        self.validate()
    }
    pub fn accept_event(
        &mut self,
        event: &LostVoidEventReceipt,
        loading: Option<LoadingObservation>,
        challenge_confirmed: Option<EventPage>,
    ) -> Result<(), &'static str> {
        let mut next = self.clone();
        next.validate()?;
        next.validate_event(event)?;
        if !matches!(
            next.receipt.as_ref().map(|r| &r.outcome),
            Some(TransitionOutcome::Event { .. })
        ) || next.event_visits >= 128
        {
            return Err("跨层没有待交回事件或次数已达上限");
        }
        if let Some(old) = &next.accepted_event
            && (event.actions < old.actions
                || event.children < old.children
                || event.gear_repeats < old.gear_repeats
                || !event.completed_first.starts_with(&old.completed_first)
                || !event.append_priority.starts_with(&old.append_priority))
        {
            return Err("跨层事件历史不能倒退");
        }
        next.accepted_event = Some(event.clone());
        next.event_visits += 1;
        if let Some(loading) = loading {
            loading.validate()?;
            next.departure = Some(DepartureProof::Loading(loading));
        } else if let Some(page) = challenge_confirmed {
            if !next.request.region()?.starts_with("挑战-")
                || !matching_challenge(next.request.region()?, &page)
            {
                return Err("本次出口不是挑战确认区域");
            }
            page.validate()?;
            if next.departure.is_none() {
                next.departure = Some(DepartureProof::ChallengeConfirmed {
                    interaction_id: next.request.exit.interaction_id,
                    page,
                });
            }
        }
        next.invalidate_observation();
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn invalidate_observation(&mut self) {
        self.world = None;
        self.world_reads = 0;
        self.content = None;
        self.receipt = None;
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.invalidate_observation();
        self.validate()
    }
}
