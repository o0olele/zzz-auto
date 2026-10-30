//! 普通出战页选队，复用完整二十槽位选择；矩阵使用独立页面。
use crate::{
    lost_void::{LostVoidChallenge, LostVoidMission as M},
    lost_void_team_plan::{LostVoidTeamKind, LostVoidTeamPlan, plan_team},
    predefined_teams::TeamConfig,
    simulation_teams::{TeamsFlow, TeamsRequest, TeamsStage, TeamsView},
    team_selection::TeamSelection,
    team_selection_slots::SlotIdentity,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidClassicTeamsRequest {
    pub run_id: Uuid,
    pub mission: M,
    pub challenge: LostVoidChallenge,
    pub teams: TeamConfig,
    pub up_agents: Option<Vec<String>>,
    pub up_completed_this_week: bool,
    pub timeout_ms: u32,
}
impl LostVoidClassicTeamsRequest {
    pub fn plan(&self) -> Result<LostVoidTeamPlan, &'static str> {
        if self.run_id.is_nil()
            || self.mission == M::MatrixAction
            || !(1000..=900_000).contains(&self.timeout_ms)
        {
            return Err("迷失之地普通选队身份、副本或预算无效");
        }
        if self.mission != M::TaskForce && self.up_agents.is_some() {
            return Err("普通战线肃清不能附加特遣 UP 身份");
        }
        plan_team(
            self.mission,
            &self.challenge,
            &self.teams,
            self.up_agents.as_deref(),
            self.up_completed_this_week,
        )?
        .ok_or("迷失之地 UP 选队仍缺完整本周名单")
    }
    fn child(&self) -> Result<TeamsRequest, &'static str> {
        let plan = self.plan()?;
        let predefined_team_idx = match plan.kind {
            LostVoidTeamKind::KeepCurrent => -1,
            LostVoidTeamKind::Predefined { index } => index as i8,
            _ => return Err("矩阵手选不能进入普通预备编队列表"),
        };
        Ok(TeamsRequest {
            predefined_team_idx,
            teams: self.teams.clone(),
            script_name: plan.script_name,
            timeout_ms: self.timeout_ms,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidClassicTeamsReceipt {
    pub run_id: Uuid,
    pub mission: M,
    pub plan: LostVoidTeamPlan,
    /// 列表中实际核对的槽位、名称和人数；不是实际代理人身份。
    pub slot: Option<SlotIdentity>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidClassicTeamsFlow {
    request: LostVoidClassicTeamsRequest,
    plan: LostVoidTeamPlan,
    child: TeamsFlow,
    ready_reads: u8,
    receipt: Option<LostVoidClassicTeamsReceipt>,
}
impl LostVoidClassicTeamsFlow {
    pub fn new(request: LostVoidClassicTeamsRequest) -> Result<Self, &'static str> {
        let child = TeamsFlow::new(request.child()?)?;
        let plan = request.plan()?;
        Ok(Self {
            request,
            plan,
            child,
            ready_reads: 0,
            receipt: None,
        })
    }
    pub fn request(&self) -> &LostVoidClassicTeamsRequest {
        &self.request
    }
    pub fn plan(&self) -> &LostVoidTeamPlan {
        &self.plan
    }
    pub fn stage(&self) -> TeamsStage {
        self.child.stage()
    }
    pub fn selection(&self) -> Option<&TeamSelection> {
        self.child.selection()
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn receipt(&self) -> Option<&LostVoidClassicTeamsReceipt> {
        self.receipt.as_ref()
    }
    fn selected_slot(&self) -> Result<Option<SlotIdentity>, &'static str> {
        match self.plan.kind {
            LostVoidTeamKind::KeepCurrent => Ok(None),
            LostVoidTeamKind::Predefined { index } => self
                .child
                .selection()
                .and_then(|s| s.slot_cards.iter().find(|c| c.index == index))
                .cloned()
                .map(Some)
                .ok_or("迷失之地选队缺完整槽位身份"),
            _ => Err("普通选队不能包含矩阵手选"),
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.child.validate()?;
        if self.plan != self.request.plan()?
            || self.child.request() != &self.request.child()?
            || self.ready_reads > 2
            || self.ready_reads > 0 && !self.child.completed()
            || self.completed() != (self.ready_reads == 2)
            || self.receipt.as_ref().is_some_and(|r| {
                r.run_id != self.request.run_id
                    || r.mission != self.request.mission
                    || r.plan != self.plan
                    || !self.selected_slot().is_ok_and(|s| s == r.slot)
            })
        {
            return Err("迷失之地普通选队检查点与冻结身份不一致");
        }
        Ok(())
    }
    pub fn observe(&mut self, view: TeamsView) -> Result<bool, &'static str> {
        let mut next = self.clone();
        let open = next.observe_inner(view)?;
        *self = next;
        Ok(open)
    }
    fn observe_inner(&mut self, view: TeamsView) -> Result<bool, &'static str> {
        self.validate()?;
        if self.completed() {
            return Ok(false);
        }
        let ready = self.child.completed();
        let open = self.child.observe(view)?;
        if self.child.completed() && view == TeamsView::Deployment {
            if ready {
                self.ready_reads += 1;
            } else {
                self.ready_reads = 1;
            }
            if self.ready_reads >= 2 {
                self.receipt = Some(LostVoidClassicTeamsReceipt {
                    run_id: self.request.run_id,
                    mission: self.request.mission,
                    plan: self.plan.clone(),
                    slot: self.selected_slot()?,
                });
            }
        } else {
            self.ready_reads = 0;
        }
        self.validate()?;
        Ok(open)
    }
    pub fn prepare_open(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.child.prepare_open()
    }
    pub fn allows_open(&self) -> bool {
        self.validate().is_ok() && self.child.allows_open()
    }
    pub fn posted_open(&mut self) -> Result<(), &'static str> {
        self.child.posted_open()?;
        self.validate()
    }
    pub fn not_posted_open(&mut self) -> Result<(), &'static str> {
        self.child.not_posted_open()?;
        self.validate()
    }
    pub fn update_selection(&mut self, selection: TeamSelection) -> Result<(), &'static str> {
        self.child.update_selection(selection)?;
        self.validate()
    }
    pub fn selected(&mut self) -> Result<(), &'static str> {
        self.child.selected()?;
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.receipt = None;
        self.ready_reads = 0;
        self.child.resume()?;
        self.validate()
    }
}
