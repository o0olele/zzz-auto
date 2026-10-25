//! 模拟室只做选队准备；不提供关卡选中或战斗完成证明。
use crate::{
    charge_plan::ChargePlanItem,
    predefined_teams::TeamConfig,
    team_selection::{SelectionAction, SelectionPhase, TeamSelection},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamsRequest {
    pub predefined_team_idx: i8,
    pub teams: TeamConfig,
    pub script_name: String,
    pub timeout_ms: u32,
}
impl TeamsRequest {
    pub fn from_plan(
        plan: &ChargePlanItem,
        teams: &TeamConfig,
        timeout_ms: u32,
    ) -> Result<Self, &'static str> {
        plan.validate()?;
        if plan.category_name != "实战模拟室"
            || plan.level != "默认等级"
            || plan.mission_type_name == "代理人方案培养"
        {
            return Err("模拟室选队仅支持默认等级的普通材料及自定义模板");
        }
        let request = Self {
            predefined_team_idx: plan.predefined_team_idx,
            teams: teams.clone(),
            script_name: plan.script(teams)?.into(),
            timeout_ms,
        };
        request.validate()?;
        Ok(request)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.teams.validate()?;
        if !(-1..20).contains(&self.predefined_team_idx)
            || !(1000..=900_000).contains(&self.timeout_ms)
        {
            return Err("模拟室选队编号或预算无效");
        }
        crate::intel_board::IntelBoardConfig {
            predefined_team_idx: self.predefined_team_idx,
            auto_battle_config: self.script_name.clone(),
            exp_grind_mode: false,
        }
        .validate()?;
        if self.predefined_team_idx >= 0
            && self.teams.get(self.predefined_team_idx as u8)?.auto_battle != self.script_name
        {
            return Err("模拟室脚本与冻结编队不一致");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamsView {
    Deployment,
    Selection,
    Blocked,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamsStage {
    Entry,
    Opening,
    Selecting,
    Verifying,
    Ready,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OpenReceipt {
    Intent,
    Posted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamsResult {
    pub predefined_team_idx: i8,
    pub script_name: String,
    pub selection_verified: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamsFlow {
    request: TeamsRequest,
    stage: TeamsStage,
    open: Option<OpenReceipt>,
    selection: Option<TeamSelection>,
}
impl TeamsFlow {
    pub fn new(request: TeamsRequest) -> Result<Self, &'static str> {
        request.validate()?;
        let selection = if request.predefined_team_idx >= 0 {
            Some(TeamSelection::new(vec![request.predefined_team_idx as u8])?)
        } else {
            None
        };
        Ok(Self {
            request,
            stage: TeamsStage::Entry,
            open: None,
            selection,
        })
    }
    pub fn request(&self) -> &TeamsRequest {
        &self.request
    }
    pub fn stage(&self) -> TeamsStage {
        self.stage
    }
    pub fn selection(&self) -> Option<&TeamSelection> {
        self.selection.as_ref()
    }
    pub fn completed(&self) -> bool {
        self.stage == TeamsStage::Ready
    }
    pub fn result(&self) -> Option<TeamsResult> {
        self.completed().then(|| TeamsResult {
            predefined_team_idx: self.request.predefined_team_idx,
            script_name: self.request.script_name.clone(),
            selection_verified: false,
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        match &self.selection {
            Some(s)
                if self.request.predefined_team_idx >= 0
                    && s.targets == [self.request.predefined_team_idx as u8] =>
            {
                s.validate()?
            }
            None if self.request.predefined_team_idx == -1 => {}
            _ => return Err("模拟室选队检查点与冻结目标不一致"),
        }
        let confirmed = self
            .selection
            .as_ref()
            .is_some_and(|s| s.phase == SelectionPhase::Confirmed);
        let confirming = self
            .selection
            .as_ref()
            .is_some_and(|s| s.pending == Some(SelectionAction::Confirm));
        let valid = match self.stage {
            TeamsStage::Entry => self.open.is_none() && !confirmed,
            TeamsStage::Opening => self.selection.is_some() && !confirmed,
            TeamsStage::Selecting => self.selection.is_some() && self.open.is_some(),
            TeamsStage::Verifying => {
                (self.selection.is_none() && self.open.is_none())
                    || (self.open.is_some() && (confirmed || confirming))
            }
            TeamsStage::Ready => {
                (self.selection.is_none() && self.open.is_none())
                    || (confirmed && self.open.is_some())
            }
        };
        if !valid {
            return Err("模拟室选队阶段缺少输入或全表证明");
        }
        Ok(())
    }
    /// 返回 true 只表示需要打开列表，调用方须先保存 prepare_open 的意图。
    pub fn observe(&mut self, view: TeamsView) -> Result<bool, &'static str> {
        self.validate()?;
        match (self.stage, view) {
            (TeamsStage::Entry, TeamsView::Deployment) => {
                self.stage = if self.selection.is_some() {
                    TeamsStage::Opening
                } else {
                    TeamsStage::Ready
                };
            }
            (TeamsStage::Opening, TeamsView::Selection) if self.open.is_some() => {
                self.stage = TeamsStage::Selecting
            }
            (TeamsStage::Verifying, TeamsView::Deployment) => {
                if let Some(selection) = &mut self.selection
                    && selection.phase != SelectionPhase::Confirmed
                {
                    selection.observe_deployment()?;
                }
                self.stage = TeamsStage::Ready;
            }
            _ => {}
        }
        Ok(self.stage == TeamsStage::Opening
            && self.open.is_none()
            && view == TeamsView::Deployment)
    }
    pub fn prepare_open(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.stage != TeamsStage::Opening || self.open.is_some() {
            return Err("没有待打开的模拟室编队列表");
        }
        self.open = Some(OpenReceipt::Intent);
        Ok(())
    }
    pub fn allows_open(&self) -> bool {
        self.stage == TeamsStage::Opening && self.open == Some(OpenReceipt::Intent)
    }
    pub fn posted_open(&mut self) -> Result<(), &'static str> {
        if !self.allows_open() {
            return Err("没有模拟室打开列表意图");
        }
        self.open = Some(OpenReceipt::Posted);
        Ok(())
    }
    pub fn not_posted_open(&mut self) -> Result<(), &'static str> {
        if !self.allows_open() {
            return Err("没有可撤销的打开列表意图");
        }
        self.open = None;
        Ok(())
    }
    pub fn update_selection(&mut self, selection: TeamSelection) -> Result<(), &'static str> {
        let mut next = self.clone();
        if self.stage != TeamsStage::Selecting {
            return Err("模拟室尚未交接选队");
        }
        next.selection = Some(selection);
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn selected(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.stage != TeamsStage::Selecting
            || self
                .selection
                .as_ref()
                .is_none_or(|s| s.phase != SelectionPhase::Confirmed)
        {
            return Err("模拟室选队尚未完成全表确认");
        }
        self.stage = TeamsStage::Verifying;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.completed()
            || (self.stage == TeamsStage::Selecting
                && self.selection.as_ref().is_some_and(|s| {
                    s.phase == SelectionPhase::Confirmed
                        || s.pending == Some(SelectionAction::Confirm)
                }))
        {
            self.stage = TeamsStage::Verifying;
        }
        Ok(())
    }
}
