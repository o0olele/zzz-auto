//! 情报板主流程检查点。Pipeline、选队、移动和战斗均在外层保存后才投递输入。
use crate::{
    battle_result::NormalBattleResult,
    challenge_move::{ChallengeMove, MoveStage},
    intel_board::{CommissionKind, IntelLedger},
    team_selection::{SelectionPhase, TeamSelection},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum IntelOutcome {
    WeeklyGoal,
    NoCommissions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct IntelFlow {
    pub stage: String,
    pub filter_step: u8,
    pub filtered: bool,
    pub scrolls: u8,
    pub failed_accepts: u8,
    pub candidate_changes: u8,
    pub transport_attempted: bool,
    pub arrived: bool,
    pub fallback_team: bool,
    pub selection: Option<TeamSelection>,
    pub movement: Option<ChallengeMove>,
    pub outcome: Option<IntelOutcome>,
}
impl IntelFlow {
    pub fn new(ledger: &IntelLedger) -> Result<Self, &'static str> {
        ledger.validate()?;
        if ledger.pending.is_some() || !ledger.settled.is_empty() {
            return Err("新情报板流程不能继承未结束的单次委托");
        }
        let done = ledger.totals().completed(&ledger.config);
        Ok(Self {
            stage: if done { "Return" } else { "Inspect" }.into(),
            filter_step: 0,
            filtered: false,
            scrolls: 0,
            failed_accepts: 0,
            candidate_changes: 0,
            transport_attempted: false,
            arrived: false,
            fallback_team: false,
            selection: None,
            movement: None,
            outcome: done.then_some(IntelOutcome::WeeklyGoal),
        })
    }
    pub fn entry(&self) -> String {
        format!("Intel.{}", self.stage)
    }
    pub fn validate(&self, ledger: &IntelLedger) -> Result<(), &'static str> {
        ledger.validate()?;
        let stage = self.stage.as_str();
        if !matches!(
            stage,
            "Inspect"
                | "OpenMap"
                | "SelectArea"
                | "SelectPoint"
                | "ConfirmTransport"
                | "WaitArrival"
                | "OpenGuide"
                | "OpenBoard"
                | "ReadBoard"
                | "OpenFilter"
                | "ResetFilter"
                | "SelectNotorious"
                | "SelectExpert"
                | "CloseFilter"
                | "Refresh"
                | "FindCommission"
                | "Scroll"
                | "OpenCommission"
                | "Accept"
                | "Next"
                | "ConfirmFailed"
                | "OpenTeams"
                | "SelectTeam"
                | "Deploy"
                | "WaitDeploy"
                | "ConfirmNoAgents"
                | "Move"
                | "Combat"
                | "Settlement"
                | "Return"
                | "Done"
        ) || self.filter_step > 3
            || self.scrolls > 5
            || self.failed_accepts > 16
            || self.candidate_changes > 32
            || (self.filtered && self.filter_step != 3)
            || (self.arrived && !self.transport_attempted)
        {
            return Err("情报板流程阶段或次数无效");
        }
        if let Some(selection) = &self.selection {
            selection.validate()?;
            let target = if self.fallback_team {
                0
            } else {
                u8::try_from(ledger.config.predefined_team_idx).map_err(|_| "未指定情报板编队")?
            };
            if selection.targets != [target] {
                return Err("情报板选择不是冻结的编队");
            }
        }
        let pending = ledger.pending.as_ref();
        if matches!(stage, "OpenTeams" | "SelectTeam") && self.selection.is_none() {
            return Err("选队阶段缺少冻结目标");
        }
        if stage == "Move" && self.movement.is_none() {
            return Err("移动阶段缺少当前委托进度");
        }
        if let Some(movement) = &self.movement {
            movement.validate()?;
            if pending.is_none_or(|p| {
                !p.deployed || movement.expert != (p.kind == CommissionKind::ExpertChallenge)
            }) {
                return Err("战前移动与当前委托不符");
            }
        }
        if matches!(
            stage,
            "FindCommission"
                | "Scroll"
                | "OpenCommission"
                | "Accept"
                | "Next"
                | "ConfirmFailed"
                | "SelectTeam"
                | "OpenTeams"
                | "Deploy"
                | "WaitDeploy"
                | "ConfirmNoAgents"
                | "Move"
                | "Combat"
                | "Settlement"
        ) && !self.filtered
        {
            return Err("尚未完成两种委托筛选");
        }
        if matches!(
            stage,
            "OpenCommission"
                | "Accept"
                | "Next"
                | "ConfirmFailed"
                | "OpenTeams"
                | "SelectTeam"
                | "Deploy"
                | "WaitDeploy"
                | "ConfirmNoAgents"
                | "Move"
                | "Combat"
                | "Settlement"
        ) && pending.is_none()
        {
            return Err("情报板阶段缺少本次委托");
        }
        if matches!(
            stage,
            "WaitDeploy" | "ConfirmNoAgents" | "Move" | "Combat" | "Settlement"
        ) && pending.is_none_or(|p| !p.deployed)
        {
            return Err("尚未保存出战检查点");
        }
        if stage == "Combat"
            && self
                .movement
                .as_ref()
                .is_none_or(|m| m.stage != MoveStage::Done)
        {
            return Err("战前移动尚未完成");
        }
        if stage == "Settlement" && pending.is_none_or(|p| p.result.is_none()) {
            return Err("结算阶段缺少战斗结果");
        }
        if matches!(
            stage,
            "Deploy" | "WaitDeploy" | "Move" | "Combat" | "Settlement"
        ) && (ledger.config.predefined_team_idx >= 0 || self.fallback_team)
            && self
                .selection
                .as_ref()
                .is_none_or(|s| s.phase != SelectionPhase::Confirmed)
        {
            return Err("指定编队尚未完成选择与确认");
        }
        if matches!(stage, "SelectNotorious" | "SelectExpert" | "CloseFilter") {
            let expected = match stage {
                "SelectNotorious" => 1,
                "SelectExpert" => 2,
                _ => 3,
            };
            if self.filter_step != expected || self.filtered {
                return Err("委托筛选顺序无效");
            }
        }
        if self.outcome.is_some() != matches!(stage, "Return" | "Done")
            || (self.outcome.is_some() && pending.is_some())
            || (self.outcome == Some(IntelOutcome::WeeklyGoal)
                && !ledger.totals().completed(&ledger.config))
            || (self.outcome == Some(IntelOutcome::NoCommissions)
                && (!self.filtered || self.scrolls != 5))
        {
            return Err("情报板结束原因缺少目标或无委托证明");
        }
        Ok(())
    }
    fn edit(
        &mut self,
        ledger: &mut IntelLedger,
        change: impl FnOnce(&mut Self, &mut IntelLedger) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        self.validate(ledger)?;
        let mut flow = self.clone();
        let mut next = ledger.clone();
        change(&mut flow, &mut next)?;
        flow.validate(&next)?;
        ledger.check_update(&next)?;
        *self = flow;
        *ledger = next;
        Ok(())
    }
    /// 执行者在发布此检查点成功后才允许投递当前动作。
    pub fn before_step(&mut self, ledger: &mut IntelLedger) -> Result<(), &'static str> {
        self.edit(ledger, |flow, ledger| {
            if flow.stage == "ConfirmTransport" {
                flow.transport_attempted = true;
            }
            if flow.stage == "Deploy" && ledger.pending.as_ref().is_some_and(|p| !p.deployed) {
                ledger.mark_deployed()?;
            }
            Ok(())
        })
    }
    pub fn read_board(
        &mut self,
        ledger: &mut IntelLedger,
        contribution: u16,
    ) -> Result<(), &'static str> {
        self.edit(ledger, |flow, ledger| {
            if flow.stage != "ReadBoard" {
                return Err("不在读取情报板阶段");
            }
            ledger.read_progress(&format!("{contribution}/1000"))?;
            if ledger.pending.is_some() {
                ledger.return_to_board()?;
                flow.selection = None;
                flow.movement = None;
                flow.fallback_team = false;
            }
            flow.stage = if ledger.totals().completed(&ledger.config) {
                flow.outcome = Some(IntelOutcome::WeeklyGoal);
                "Return"
            } else if flow.filtered {
                "Refresh"
            } else {
                "OpenFilter"
            }
            .into();
            Ok(())
        })
    }
    /// 只消费同帧读取的进度和候选类型；位置由原生执行器在输入前重新确认。
    pub fn scan_board(
        &mut self,
        ledger: &mut IntelLedger,
        contribution: u16,
        candidate: Option<CommissionKind>,
    ) -> Result<(), &'static str> {
        self.edit(ledger, |flow, ledger| {
            if flow.stage != "FindCommission" {
                return Err("不在寻找委托阶段");
            }
            ledger.read_progress(&format!("{contribution}/1000"))?;
            if ledger.totals().completed(&ledger.config) {
                flow.outcome = Some(IntelOutcome::WeeklyGoal);
                flow.stage = "Return".into();
            } else if let Some(kind) = candidate {
                ledger.begin(kind)?;
                flow.stage = "OpenCommission".into();
            } else if flow.scrolls == 5 {
                flow.outcome = Some(IntelOutcome::NoCommissions);
                flow.stage = "Return".into();
            } else {
                flow.stage = "Scroll".into();
            }
            Ok(())
        })
    }
    fn prepare_team(&mut self, ledger: &IntelLedger) -> Result<(), &'static str> {
        let target = if self.fallback_team {
            0
        } else {
            ledger.config.predefined_team_idx
        };
        if target >= 0 {
            self.selection = Some(TeamSelection::new(vec![target as u8])?);
            self.stage = "OpenTeams".into();
        } else {
            self.stage = "Deploy".into();
        }
        Ok(())
    }
    pub fn observe(&mut self, ledger: &mut IntelLedger, node: &str) -> Result<(), &'static str> {
        let node = node.strip_prefix("Intel.").ok_or("情报板节点前缀不符")?;
        self.edit(ledger, |flow, ledger| {
            let next = match (flow.stage.as_str(), node) {
                ("Inspect", "World") => {
                    if flow.arrived {
                        "OpenGuide"
                    } else {
                        "OpenMap"
                    }
                }
                ("Inspect" | "OpenMap", "MapOpened") => "SelectArea",
                ("Inspect" | "SelectArea", "AreaSelected") => "SelectPoint",
                ("Inspect" | "SelectPoint", "PointSelected") => "ConfirmTransport",
                ("ConfirmTransport", "Transported") if flow.transport_attempted => "WaitArrival",
                ("WaitArrival", "PointSelected") => "ConfirmTransport",
                ("WaitArrival", "Arrived") => {
                    flow.arrived = true;
                    "OpenGuide"
                }
                ("Inspect" | "OpenGuide", "Guide") => "OpenBoard",
                ("Inspect" | "OpenBoard" | "Settlement", "Board") => "ReadBoard",
                ("Inspect" | "OpenFilter", "Filter") => {
                    flow.filtered = false;
                    flow.filter_step = 0;
                    "ResetFilter"
                }
                ("ResetFilter", "FilterReset") => {
                    flow.filter_step = 1;
                    "SelectNotorious"
                }
                ("SelectNotorious", "NotoriousSelected") => {
                    flow.filter_step = 2;
                    "SelectExpert"
                }
                ("SelectExpert", "ExpertSelected") => {
                    flow.filter_step = 3;
                    "CloseFilter"
                }
                ("CloseFilter", "FilterClosed") => {
                    flow.filtered = true;
                    flow.scrolls = 0;
                    "FindCommission"
                }
                ("Refresh", "Refreshed") => {
                    flow.scrolls = 0;
                    "FindCommission"
                }
                ("Scroll", "Scrolled") if flow.scrolls < 5 => {
                    flow.scrolls += 1;
                    "FindCommission"
                }
                ("OpenCommission", "CommissionSelected") => "Accept",
                ("OpenCommission", "CandidateChanged") => {
                    if flow.candidate_changes == 32 {
                        return Err("情报板候选变化达到上限");
                    }
                    ledger.cancel_unaccepted()?;
                    flow.candidate_changes += 1;
                    "FindCommission"
                }
                ("Accept", "CommissionOpened" | "NextPage") => "Next",
                ("OpenCommission" | "Accept" | "Next", "PreDeployment") => {
                    flow.prepare_team(ledger)?;
                    return Ok(());
                }
                ("OpenTeams", "SelectionOpened") => "SelectTeam",
                ("OpenCommission" | "Accept" | "Next", "AcceptFailed") => "ConfirmFailed",
                ("Accept", "AcceptAdvanced") => "Accept",
                ("Next", "NextAdvanced") => "Next",
                ("ConfirmFailed", "FailureDismissed") => {
                    if flow.failed_accepts == 16 {
                        return Err("接取失败达到上限");
                    }
                    ledger.cancel_unaccepted()?;
                    flow.failed_accepts += 1;
                    "FindCommission"
                }
                ("Deploy", "DeployClicked") => "WaitDeploy",
                ("WaitDeploy", "PreDeployment") => "Deploy",
                ("WaitDeploy", "AgentConfirmed") => "WaitDeploy",
                ("Deploy" | "WaitDeploy", "NoAgents") if !flow.fallback_team => "ConfirmNoAgents",
                ("ConfirmNoAgents", "NoAgentsDismissed") => {
                    flow.fallback_team = true;
                    flow.prepare_team(ledger)?;
                    return Ok(());
                }
                ("WaitDeploy", "BattleReady") => {
                    let kind = ledger.pending.as_ref().ok_or("缺少待出战委托")?.kind;
                    flow.movement = Some(ChallengeMove::new(
                        kind == CommissionKind::ExpertChallenge,
                        3,
                    )?);
                    "Move"
                }
                ("Settlement", "SettlementAdvanced") => "Settlement",
                ("Return", "World") => "Done",
                _ => return Err("情报板节点不属于当前阶段"),
            };
            flow.stage = next.into();
            Ok(())
        })
    }
    pub fn selection_completed(&mut self, ledger: &mut IntelLedger) -> Result<(), &'static str> {
        self.edit(ledger, |flow, _| {
            if flow.stage != "SelectTeam"
                || flow
                    .selection
                    .as_ref()
                    .is_none_or(|s| s.phase != SelectionPhase::Confirmed)
            {
                return Err("选队尚未确认返回配队页");
            }
            flow.stage = "Deploy".into();
            Ok(())
        })
    }
    pub fn movement_completed(&mut self, ledger: &mut IntelLedger) -> Result<(), &'static str> {
        self.edit(ledger, |flow, _| {
            if flow.stage != "Move"
                || flow
                    .movement
                    .as_ref()
                    .is_none_or(|m| m.stage != MoveStage::Done)
            {
                return Err("战前移动尚未完成");
            }
            flow.stage = "Combat".into();
            Ok(())
        })
    }
    pub fn battle_completed(
        &mut self,
        ledger: &mut IntelLedger,
        result: NormalBattleResult,
    ) -> Result<(), &'static str> {
        self.edit(ledger, |flow, ledger| {
            if flow.stage != "Combat" {
                return Err("不在本次委托战斗中");
            }
            ledger.record_battle(result)?;
            flow.stage = "Settlement".into();
            Ok(())
        })
    }
    pub fn resume(&mut self, ledger: &mut IntelLedger) -> Result<(), &'static str> {
        self.edit(ledger, |flow, ledger| {
            if let Some(pending) = &ledger.pending {
                if pending.result.is_some() {
                    flow.stage = "Settlement".into();
                } else if flow.stage == "Move" {
                    flow.movement.as_mut().ok_or("缺少移动进度")?.resume()?;
                } else if matches!(flow.stage.as_str(), "Combat" | "ConfirmNoAgents") {
                } else if matches!(flow.stage.as_str(), "OpenTeams" | "SelectTeam") {
                    let selection = flow.selection.as_mut().ok_or("缺少选队进度")?;
                    if selection.phase == SelectionPhase::Confirmed {
                        flow.stage = "Deploy".into();
                    } else if selection.pending
                        != Some(crate::team_selection::SelectionAction::Confirm)
                    {
                        selection.restart()?;
                    }
                } else if pending.deployed {
                    flow.stage = "WaitDeploy".into();
                }
                // 接取未确认时，只在当前委托的实际按钮仍出现时继续；不能重新选另一个帖子。
                else if flow.stage != "OpenCommission" {
                    flow.stage = "Accept".into();
                }
            } else if flow.outcome.is_some() {
            } else if matches!(
                flow.stage.as_str(),
                "OpenFilter" | "ResetFilter" | "SelectNotorious" | "SelectExpert" | "CloseFilter"
            ) {
                flow.filtered = false;
                flow.filter_step = 0;
                flow.stage = "Inspect".into();
            } else if flow.transport_attempted && !flow.arrived {
                flow.stage = "WaitArrival".into();
            } else if matches!(
                flow.stage.as_str(),
                "FindCommission" | "Scroll" | "Refresh" | "ReadBoard"
            ) {
                flow.stage = "ReadBoard".into();
            } else {
                flow.stage = "Inspect".into();
            }
            Ok(())
        })
    }
}
