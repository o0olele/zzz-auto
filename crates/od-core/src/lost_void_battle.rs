//! 本次遭遇的战斗许可与局内页面交接；不提供换层或整场成功证明。
use crate::{
    lost_void::REGIONS,
    lost_void_navigation::{NavDetection, NavOutcome, NavReceipt},
    lost_void_roster::{RosterMember, RosterReceipt, validate_members},
    lost_void_scene::LostVoidScreen as S,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BattleRequest {
    pub roster: RosterReceipt,
    pub encounter: NavReceipt,
    pub region: String,
    pub timeout_ms: u32,
}
impl BattleRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        let r = &self.roster;
        let e = &self.encounter;
        validate_members(&r.members, r.members.len() as u8)?;
        if r.run_id.is_nil()
            || r.level_id.is_nil()
            || r.run_id != e.run_id
            || r.level_id != e.level_id
            || e.interaction_id.is_nil()
            || e.outcome != NavOutcome::EncounteredBattle
            || !e.battle
            || e.screen != S::World
            || !r.members.iter().any(|m| m.id == r.front)
            || !r
                .plan
                .verify_roster(&r.members.iter().map(|m| m.id.clone()).collect::<Vec<_>>())?
            || !REGIONS.contains(&self.region.as_str())
            || !(1000..=600_000).contains(&self.timeout_ms)
        {
            return Err("局内战斗缺少同次完整队伍、遭遇或区域身份");
        }
        Ok(())
    }
    fn boss(&self) -> bool {
        matches!(self.region.as_str(), "战斗-道中危机" | "战斗-终结之役")
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BattlePage {
    Unknown,
    CombatHud,
    World,
    Content(S),
    Modal,
}
fn content(s: S) -> bool {
    matches!(
        s,
        S::CommonChoice
            | S::Event
            | S::Gear
            | S::GearConfirm
            | S::Store
            | S::Lottery
            | S::RouteChange
            | S::BattleResult
            | S::BattleFail
    )
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitProbe {
    pub sequence: u64,
    pub detections: Vec<NavDetection>,
}
impl ExitProbe {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.sequence == 0 || self.detections.len() > 64 {
            return Err("战斗脱战探测序号或数量无效");
        }
        for d in &self.detections {
            d.validate()?;
            if d.score <= 0.9 {
                return Err("战斗脱战探测必须超过 0.9");
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BattleView {
    pub page: BattlePage,
    pub members: Option<Vec<RosterMember>>,
    pub attack: bool,
    pub chain: bool,
    pub dead: bool,
    pub interact: bool,
    pub next_region_hint: bool,
    /// None 是本帧未探测，不是“探测后没有目标”；重复序号不会重复投票。
    pub exit_probe: Option<ExitProbe>,
}
impl Default for BattleView {
    fn default() -> Self {
        Self {
            page: BattlePage::Unknown,
            members: None,
            attack: false,
            chain: false,
            dead: false,
            interact: false,
            next_region_hint: false,
            exit_probe: None,
        }
    }
}
impl BattleView {
    pub fn validate(&self) -> Result<(), &'static str> {
        if matches!(self.page,BattlePage::Content(s) if !content(s)) {
            return Err("未知页面不能作为战斗交接页面");
        }
        if let Some(m) = &self.members {
            validate_members(m, m.len() as u8)?;
        }
        if let Some(p) = &self.exit_probe {
            p.validate()?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorationEvidence {
    Interaction,
    NextRegionHint,
    Detector,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BattleOutcome {
    ObservedPage { screen: S },
    AgentDead,
    Exploration { evidence: ExplorationEvidence },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BattleReceipt {
    pub run_id: Uuid,
    pub level_id: Uuid,
    pub encounter_id: Uuid,
    pub outcome: BattleOutcome,
    pub combat_hud_observed: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleGate {
    Hold,
    Proceed,
    Yield,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BattleFlow {
    request: BattleRequest,
    hud: Option<Vec<RosterMember>>,
    hud_reads: u8,
    seen_hud: bool,
    boundary: Option<BattleOutcome>,
    boundary_reads: u8,
    probe: Option<ExitProbe>,
    probe_votes: u8,
    receipt: Option<BattleReceipt>,
}
impl BattleFlow {
    pub fn new(request: BattleRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            hud: None,
            hud_reads: 0,
            seen_hud: false,
            boundary: None,
            boundary_reads: 0,
            probe: None,
            probe_votes: 0,
            receipt: None,
        })
    }
    pub fn request(&self) -> &BattleRequest {
        &self.request
    }
    pub fn receipt(&self) -> Option<&BattleReceipt> {
        self.receipt.as_ref()
    }
    pub fn last_probe_sequence(&self) -> u64 {
        self.probe.as_ref().map_or(0, |p| p.sequence)
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if let Some(p) = &self.probe {
            p.validate()?;
        }
        if self.hud_reads > 2
            || self.boundary_reads > 3
            || self.probe_votes > 10
            || self.hud.is_some() != (self.hud_reads > 0)
            || self.boundary.is_some() != (self.boundary_reads > 0)
            || self.hud_reads == 2 && !self.seen_hud
            || self.receipt.is_some()
                != self
                    .boundary
                    .is_some_and(|b| self.boundary_reads >= Self::required(b))
            || self.probe_votes > 0
                && (self.probe.as_ref().is_none_or(|p| p.detections.is_empty())
                    || self.request.boss())
            || self.hud.as_ref().is_some_and(|h| !self.same_members(h))
            || self.boundary.is_some_and(
                |b| matches!(b,BattleOutcome::ObservedPage{screen} if !content(screen)),
            )
            || matches!(
                self.boundary,
                Some(BattleOutcome::Exploration {
                    evidence: ExplorationEvidence::Detector
                })
            ) && self.probe_votes != 10
            || self.receipt.as_ref().is_some_and(|r| {
                r.run_id != self.request.roster.run_id
                    || r.level_id != self.request.roster.level_id
                    || r.encounter_id != self.request.encounter.interaction_id
                    || Some(r.outcome) != self.boundary
                    || r.combat_hud_observed != self.seen_hud
                    || self.boundary_reads < Self::required(r.outcome)
            })
        {
            return Err("局内战斗检查点与本次身份或观察不一致");
        }
        Ok(())
    }
    fn same_members(&self, actual: &[RosterMember]) -> bool {
        actual.len() == self.request.roster.members.len()
            && actual
                .iter()
                .all(|m| self.request.roster.members.contains(m))
            && validate_members(actual, actual.len() as u8).is_ok()
    }
    fn required(outcome: BattleOutcome) -> u8 {
        match outcome {
            BattleOutcome::AgentDead
            | BattleOutcome::Exploration {
                evidence: ExplorationEvidence::NextRegionHint,
            } => 2,
            BattleOutcome::ObservedPage { .. }
            | BattleOutcome::Exploration {
                evidence: ExplorationEvidence::Interaction,
            } => 3,
            BattleOutcome::Exploration {
                evidence: ExplorationEvidence::Detector,
            } => 1,
        }
    }
    pub fn invalidate_observation(&mut self) {
        self.hud = None;
        self.hud_reads = 0;
        self.boundary = None;
        self.boundary_reads = 0;
        self.probe_votes = 0;
        self.receipt = None;
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.invalidate_observation();
        self.validate()
    }
    pub fn next(&mut self, view: &BattleView) -> Result<BattleGate, &'static str> {
        // 一次观察失败不能留下半次投票或替换已确认的队伍。
        let mut next = self.clone();
        let gate = next.next_inner(view)?;
        *self = next;
        Ok(gate)
    }
    fn next_inner(&mut self, v: &BattleView) -> Result<BattleGate, &'static str> {
        self.validate()?;
        v.validate()?;
        if self.completed() {
            return Ok(BattleGate::Yield);
        }
        let world = matches!(v.page, BattlePage::World | BattlePage::CombatHud);
        if let Some(p) = &v.exit_probe {
            if let Some(old) = &self.probe
                && (p.sequence < old.sequence || p.sequence == old.sequence && p != old)
            {
                return Err("战斗脱战探测重复变更或序号倒退");
            }
            if self
                .probe
                .as_ref()
                .is_none_or(|old| p.sequence > old.sequence)
            {
                self.probe_votes = if world && !self.request.boss() && !p.detections.is_empty() {
                    (self.probe_votes + 1).min(10)
                } else {
                    0
                };
                self.probe = Some(p.clone());
            }
        }
        if !world {
            self.probe_votes = 0;
        }
        let boundary = if v.dead {
            Some(BattleOutcome::AgentDead)
        } else {
            match v.page {
                BattlePage::Content(screen) => Some(BattleOutcome::ObservedPage { screen }),
                _ if world && v.next_region_hint => Some(BattleOutcome::Exploration {
                    evidence: ExplorationEvidence::NextRegionHint,
                }),
                _ if world && v.interact => Some(BattleOutcome::Exploration {
                    evidence: ExplorationEvidence::Interaction,
                }),
                _ if world && self.probe_votes >= 10 => Some(BattleOutcome::Exploration {
                    evidence: ExplorationEvidence::Detector,
                }),
                _ => None,
            }
        };
        if let Some(outcome) = boundary {
            self.hud = None;
            self.hud_reads = 0;
            self.boundary_reads = if self.boundary == Some(outcome) {
                (self.boundary_reads + 1).min(3)
            } else {
                1
            };
            self.boundary = Some(outcome);
            if self.boundary_reads >= Self::required(outcome) {
                self.receipt = Some(BattleReceipt {
                    run_id: self.request.roster.run_id,
                    level_id: self.request.roster.level_id,
                    encounter_id: self.request.encounter.interaction_id,
                    outcome,
                    combat_hud_observed: self.seen_hud,
                });
                self.validate()?;
                return Ok(BattleGate::Yield);
            }
            return Ok(BattleGate::Hold);
        }
        self.boundary = None;
        self.boundary_reads = 0;
        if !world || !(v.attack || v.chain) {
            self.hud = None;
            self.hud_reads = 0;
            return Ok(BattleGate::Hold);
        }
        let Some(members) = &v.members else {
            self.hud = None;
            self.hud_reads = 0;
            return Ok(BattleGate::Hold);
        };
        if !self.same_members(members) {
            return Err("实际战斗主战身份或类型与本次完整队伍不一致");
        }
        self.hud_reads = if self.hud.as_ref() == Some(members) {
            (self.hud_reads + 1).min(2)
        } else {
            1
        };
        self.hud = Some(members.clone());
        if self.hud_reads >= 2 {
            self.seen_hud = true;
            self.validate()?;
            Ok(BattleGate::Proceed)
        } else {
            Ok(BattleGate::Hold)
        }
    }
}
