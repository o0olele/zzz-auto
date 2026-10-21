//! 手动游历：领取、原副本重派和四个配置副本分开保存。未知画面不提供完成证明。
use crate::suibian_temple::{AdventureDuration, AdventureMission, SuibianTempleConfig};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AdventureMode {
    ClaimOnly,
    ClaimAndDispatch,
    DispatchOnly,
    Material,
}
impl AdventureMode {
    pub fn claims(self) -> bool {
        matches!(self, Self::ClaimOnly | Self::ClaimAndDispatch)
    }
    pub fn dispatches(self) -> bool {
        self != Self::ClaimOnly
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DispatchOutcome {
    Dispatched,
    AlreadyRunning,
    InsufficientEnergy,
    NoTeams,
    DurationUnavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AdventureClaim {
    pub mission: String,
    pub count_before: u8,
    pub reward_seen: bool,
    pub collected: bool,
    pub redispatch: Option<DispatchOutcome>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AdventureDispatch {
    pub mission: String,
    pub outcome: DispatchOutcome,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CommitKind {
    Claim,
    Reward,
    Duration,
    Dispatch,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CommitState {
    Intent,
    NotPosted,
    Observed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AdventureCommit {
    pub kind: CommitKind,
    pub mission: String,
    pub state: CommitState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdventureAction {
    Open,
    Squads,
    OpenCompleted(String),
    SelectMission(String),
    SelectStage(String),
    Pan(bool),
    ClosePanel,
    OpenDuration,
    SelectDuration(AdventureDuration),
    ConfirmDuration,
    AutoSelect,
    ConfirmBangboo,
    Claim,
    ConfirmReward,
    Dispatch,
    CloseRejection,
    Back,
}
impl AdventureAction {
    pub fn commit(&self) -> Option<CommitKind> {
        match self {
            Self::Claim => Some(CommitKind::Claim),
            Self::ConfirmReward => Some(CommitKind::Reward),
            Self::ConfirmDuration => Some(CommitKind::Duration),
            Self::Dispatch => Some(CommitKind::Dispatch),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdventurePage {
    pub count: u8,
    pub squads_open: bool,
    pub squads_readable: bool,
    pub completed: Vec<String>,
    pub visible_titles: Vec<String>,
    pub selected: Option<String>,
    pub panel: bool,
    pub claimable: bool,
    pub running: bool,
    pub ready: bool,
    pub bangboo_ready: bool,
    pub energy_shortage: bool,
    pub duration: Option<AdventureDuration>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdventureView {
    Unknown,
    Entry,
    Managed,
    Page(AdventurePage),
    Reward,
    Duration {
        options: Vec<AdventureDuration>,
        selected: Option<AdventureDuration>,
    },
    Bangboo {
        ready: bool,
    },
    Rejection(DispatchOutcome),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TempleAdventure {
    pub config: SuibianTempleConfig,
    pub mode: AdventureMode,
    pub material_mission: Option<String>,
    pub stage: String,
    pub claims_checked: bool,
    pub claims: Vec<AdventureClaim>,
    pub current_claim: Option<u8>,
    pub claim_target: Option<String>,
    pub dispatches: Vec<AdventureDispatch>,
    pub dispatch_target: Option<String>,
    pub duration_confirmed: bool,
    pub bangboo_confirmed: bool,
    pub pans: u8,
    pub commits: Vec<AdventureCommit>,
    pub pending: Option<u8>,
    pub returned_entry: bool,
}
impl TempleAdventure {
    pub fn new(config: SuibianTempleConfig, mode: AdventureMode) -> Result<Self, &'static str> {
        if mode == AdventureMode::Material {
            return Err("材料补充须指定已选游历副本");
        }
        Self::create(config, mode, None)
    }
    pub fn material(config: SuibianTempleConfig, mission: String) -> Result<Self, &'static str> {
        Self::create(config, AdventureMode::Material, Some(mission))
    }
    fn create(
        config: SuibianTempleConfig,
        mode: AdventureMode,
        material_mission: Option<String>,
    ) -> Result<Self, &'static str> {
        config.validate()?;
        let value = Self {
            config,
            mode,
            material_mission,
            stage: "Inspect".into(),
            claims_checked: !mode.claims(),
            claims: vec![],
            current_claim: None,
            claim_target: None,
            dispatches: vec![],
            dispatch_target: None,
            duration_confirmed: false,
            bangboo_confirmed: false,
            pans: 0,
            commits: vec![],
            pending: None,
            returned_entry: false,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate()?;
        let valid = |id: &str| AdventureMission::parse(id).is_some();
        if !matches!(
            self.stage.as_str(),
            "Inspect"
                | "ScanSquads"
                | "OpenCompleted"
                | "Claim"
                | "Reward"
                | "ChooseMission"
                | "PrepareDispatch"
                | "Duration"
                | "AutoSelect"
                | "Dispatch"
                | "FinishMaterial"
                | "Return"
                | "Done"
        ) || ((self.mode == AdventureMode::Material) != self.material_mission.is_some())
            || self.material_mission.as_ref().is_some_and(|m| !valid(m))
            || (!self.mode.claims()
                && (!self.claims_checked
                    || !self.claims.is_empty()
                    || self.current_claim.is_some()))
            || self.claims.len() > 4
            || self.claims.iter().enumerate().any(|(i, c)| {
                !valid(&c.mission)
                    || c.count_before == 0
                    || c.count_before > 4
                    || self.claims[..i].iter().any(|old| old.mission == c.mission)
                    || (c.collected && !c.reward_seen)
                    || (c.redispatch.is_some() && !c.collected)
            })
            || self
                .current_claim
                .is_some_and(|i| usize::from(i) >= self.claims.len())
            || self.claim_target.as_ref().is_some_and(|m| !valid(m))
            || self.dispatch_target.as_ref().is_some_and(|m| !valid(m))
            || self.pans > 12
            || self.commits.len() > 64
            || self.commits.iter().any(|c| !valid(&c.mission))
            || self
                .commits
                .iter()
                .enumerate()
                .any(|(i, c)| c.state == CommitState::Intent && self.pending != Some(i as u8))
            || self.pending.is_some_and(|i| {
                usize::from(i) + 1 != self.commits.len()
                    || self.commits[usize::from(i)].state != CommitState::Intent
            })
            || (self.returned_entry
                && (self.stage != "Done" || self.mode == AdventureMode::Material))
            || (self.stage == "Done"
                && (!self.claims_checked
                    || self.pending.is_some()
                    || self.current_claim.is_some()
                    || (self.mode != AdventureMode::Material && !self.returned_entry)))
        {
            return Err("游历检查点缺少冻结模式、顺序或提交后态证明");
        }
        if self.claims.iter().any(|c| {
            (c.reward_seen
                && !self.commits.iter().any(|r| {
                    r.kind == CommitKind::Claim
                        && r.mission == c.mission
                        && r.state == CommitState::Observed
                }))
                || (c.collected
                    && !self.commits.iter().any(|r| {
                        r.kind == CommitKind::Reward
                            && r.mission == c.mission
                            && r.state == CommitState::Observed
                    }))
                || (self.mode == AdventureMode::ClaimOnly && c.redispatch.is_some())
        }) || (self.stage == "Done"
            && self.claims.iter().any(|c| {
                !c.collected
                    || (self.mode == AdventureMode::ClaimAndDispatch && c.redispatch.is_none())
            }))
        {
            return Err("游历收获缺少本次领取、确认或重派回执");
        }
        let claim = self.current_claim.map(|i| &self.claims[usize::from(i)]);
        if (self.stage == "Claim"
            && claim.is_none_or(|c| c.collected || self.claim_target.as_ref() != Some(&c.mission)))
            || (self.stage == "Reward" && claim.is_none_or(|c| !c.reward_seen || c.collected))
            || (matches!(
                self.stage.as_str(),
                "PrepareDispatch" | "Duration" | "AutoSelect" | "Dispatch"
            ) && (!self.mode.dispatches()
                || self.dispatch_target.is_none()
                || claim.is_some_and(|c| !c.collected || c.redispatch.is_some())))
            || (self.bangboo_confirmed && !self.duration_confirmed)
            || (self.duration_confirmed
                && self.dispatch_target.as_ref().is_none_or(|m| {
                    !self.commits.iter().any(|c| {
                        c.kind == CommitKind::Duration
                            && &c.mission == m
                            && c.state == CommitState::Observed
                    })
                }))
        {
            return Err("游历阶段与本次收获、时长或派遣证明不符");
        }
        if let Some(index) = self.pending {
            let c = &self.commits[usize::from(index)];
            let expected = match self.stage.as_str() {
                "Claim" => Some(CommitKind::Claim),
                "Reward" => Some(CommitKind::Reward),
                "Duration" => Some(CommitKind::Duration),
                "Dispatch" => Some(CommitKind::Dispatch),
                _ => None,
            };
            let mission = if matches!(c.kind, CommitKind::Claim | CommitKind::Reward) {
                claim.map(|c| &c.mission)
            } else {
                self.dispatch_target.as_ref()
            };
            if expected != Some(c.kind) || mission != Some(&c.mission) {
                return Err("游历待确认提交不属于当前阶段和副本");
            }
        }
        let targets: Vec<_> = if let Some(m) = &self.material_mission {
            vec![m.clone()]
        } else if self.mode.dispatches() {
            self.config.adventure_missions.to_vec()
        } else {
            vec![]
        };
        if self.dispatches.len() > targets.len()
            || self.dispatches.iter().enumerate().any(|(i, r)| {
                targets[i] != r.mission
                    || (i + 1 < self.dispatches.len() && r.outcome != DispatchOutcome::Dispatched)
            })
            || (self.stage == "Done"
                && self.mode.dispatches()
                && self.dispatches.len() != targets.len()
                && self
                    .dispatches
                    .last()
                    .is_none_or(|r| r.outcome == DispatchOutcome::Dispatched))
        {
            return Err("游历派遣结果不符合冻结顺序或缺少明确停止原因");
        }
        Ok(())
    }
    fn view_valid(view: &AdventureView) -> bool {
        match view {
            AdventureView::Page(p) => {
                p.count <= 4
                    && p.completed.len() <= 4
                    && p.completed.iter().enumerate().all(|(i, m)| {
                        AdventureMission::parse(m).is_some() && !p.completed[..i].contains(m)
                    })
                    && p.selected
                        .as_ref()
                        .is_none_or(|m| AdventureMission::parse(m).is_some())
                    && [p.ready, p.claimable, p.running]
                        .into_iter()
                        .filter(|v| *v)
                        .count()
                        <= 1
            }
            AdventureView::Duration { options, selected } => {
                options.len() <= 7
                    && options
                        .iter()
                        .enumerate()
                        .all(|(i, d)| !options[..i].contains(d))
                    && selected.is_none_or(|d| options.contains(&d))
            }
            AdventureView::Rejection(reason) => matches!(
                reason,
                DispatchOutcome::NoTeams | DispatchOutcome::InsufficientEnergy
            ),
            _ => true,
        }
    }
    pub fn completed(&self) -> bool {
        self.stage == "Done" && self.validate().is_ok()
    }
    pub fn check_update(&self, next: &Self) -> Result<(), &'static str> {
        self.validate()?;
        next.validate()?;
        if self.config != next.config
            || self.mode != next.mode
            || self.material_mission != next.material_mission
            || (self.claims_checked && !next.claims_checked)
            || !next.dispatches.starts_with(&self.dispatches)
            || next.dispatches.len() > self.dispatches.len() + 1
            || next.claims.len() < self.claims.len()
            || next.claims.len() > self.claims.len() + 1
            || next.commits.len() < self.commits.len()
            || next.commits.len() > self.commits.len() + 1
            || (self.completed() && self != next)
        {
            return Err("不能改写已冻结的游历设置或结果");
        }
        for (old, new) in self.claims.iter().zip(&next.claims) {
            if old.mission != new.mission
                || old.count_before != new.count_before
                || (old.reward_seen && !new.reward_seen)
                || (old.collected && !new.collected)
                || old.redispatch.is_some_and(|r| new.redispatch != Some(r))
            {
                return Err("不能撤回游历收获或重派结果");
            }
        }
        for (old, new) in self.commits.iter().zip(&next.commits) {
            if old.kind != new.kind
                || old.mission != new.mission
                || (old.state != CommitState::Intent && old != new)
            {
                return Err("游历提交回执不能重写");
            }
        }
        if next.commits.len() > self.commits.len()
            && (self.pending.is_some()
                || next
                    .commits
                    .last()
                    .is_none_or(|c| c.state != CommitState::Intent))
        {
            return Err("游历须先保存新的提交意图，不能直接添加成功回执");
        }
        let observed = |kind, mission: &str| {
            self.pending.is_some_and(|i| {
                let old = &self.commits[usize::from(i)];
                let new = &next.commits[usize::from(i)];
                old.kind == kind
                    && old.mission == mission
                    && old.state == CommitState::Intent
                    && new.state == CommitState::Observed
            })
        };
        for (old, new) in self.claims.iter().zip(&next.claims) {
            if (!old.reward_seen && new.reward_seen && !observed(CommitKind::Claim, &old.mission))
                || (!old.collected && new.collected && !observed(CommitKind::Reward, &old.mission))
                || (old.redispatch != Some(DispatchOutcome::Dispatched)
                    && new.redispatch == Some(DispatchOutcome::Dispatched)
                    && !observed(CommitKind::Dispatch, &old.mission))
            {
                return Err("游历收获或重派缺少本次待确认提交");
            }
        }
        if next.dispatches.len() > self.dispatches.len()
            && next.dispatches.last().is_some_and(|r| {
                r.outcome == DispatchOutcome::Dispatched
                    && !observed(CommitKind::Dispatch, &r.mission)
            })
        {
            return Err("新增派遣成功没有本次投递和实际后态证明");
        }
        Ok(())
    }
    pub fn prepare(&mut self, action: &AdventureAction) -> Result<(), &'static str> {
        self.validate()?;
        if let Some(kind) = action.commit() {
            let expected = match self.stage.as_str() {
                "Claim" => Some(CommitKind::Claim),
                "Reward" => Some(CommitKind::Reward),
                "Duration" => Some(CommitKind::Duration),
                "Dispatch" => Some(CommitKind::Dispatch),
                _ => None,
            };
            if expected != Some(kind)
                || (kind == CommitKind::Dispatch
                    && (!self.duration_confirmed || !self.bangboo_confirmed))
            {
                return Err("游历提交不属于当前阶段或缺少选择证明");
            }

            if self.pending.is_some() || self.commits.len() >= 64 {
                return Err("游历提交尚未确认或次数达到上限");
            }
            let mission = match kind {
                CommitKind::Claim | CommitKind::Reward => self
                    .current_claim
                    .and_then(|i| self.claims.get(usize::from(i)))
                    .map(|c| c.mission.clone()),
                _ => self.dispatch_target.clone(),
            }
            .ok_or("游历提交缺少本次副本")?;
            self.pending = Some(self.commits.len() as u8);
            self.commits.push(AdventureCommit {
                kind,
                mission,
                state: CommitState::Intent,
            });
        }
        self.validate()
    }
    pub fn not_posted(&mut self) -> Result<(), &'static str> {
        if let Some(i) = self.pending.take() {
            self.commits[usize::from(i)].state = CommitState::NotPosted;
        }
        self.validate()
    }
    fn observed(&mut self) {
        let i = self.pending.take().expect("本次提交存在");
        self.commits[usize::from(i)].state = CommitState::Observed;
    }
    pub fn allows(&self, action: &AdventureAction, view: &AdventureView) -> bool {
        let mut probe = self.clone();
        if let Some(i) = probe.pending.take() {
            if action.commit() != Some(probe.commits[usize::from(i)].kind) {
                return false;
            }
            probe.commits.pop();
        } else if action.commit().is_some() {
            return false;
        }
        probe
            .next(view)
            .is_ok_and(|expected| expected.as_ref() == Some(action))
    }
    pub fn posted_navigation(&mut self, action: &AdventureAction) -> Result<(), &'static str> {
        if matches!(action, AdventureAction::Pan(_)) {
            if self.pans >= 12 {
                return Err("游历找图次数超限");
            }
            self.pans += 1;
        }
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        // 参数和标记每次输入前重新读帧；已投递的意图只等待自己的后态。
        self.validate()
    }
    fn dispatch_result(&mut self, outcome: DispatchOutcome) -> Result<(), &'static str> {
        let mission = self.dispatch_target.take().ok_or("缺少本次游历派遣副本")?;
        if let Some(i) = self.current_claim.take() {
            self.claims[usize::from(i)].redispatch = Some(outcome);
            self.claim_target = None;
            self.stage = "ScanSquads".into();
        } else {
            self.dispatches.push(AdventureDispatch { mission, outcome });
            self.stage = if self.mode == AdventureMode::Material {
                "FinishMaterial"
            } else if outcome != DispatchOutcome::Dispatched || self.dispatches.len() == 4 {
                "Return"
            } else {
                "ChooseMission"
            }
            .into();
        }
        self.duration_confirmed = false;
        self.bangboo_confirmed = false;
        self.pans = 0;
        Ok(())
    }
    pub fn next(&mut self, view: &AdventureView) -> Result<Option<AdventureAction>, &'static str> {
        self.validate()?;
        if !Self::view_valid(view) {
            return Err("游历读帧含冲突状态、非法副本或小队数");
        }
        let before = self.clone();
        let result = self.advance(view);
        if result.is_err() {
            *self = before;
            return result;
        }
        if let Err(error) = before.check_update(self) {
            *self = before;
            return Err(error);
        }
        result
    }
    fn advance(&mut self, view: &AdventureView) -> Result<Option<AdventureAction>, &'static str> {
        use AdventureAction as A;
        if matches!(view, AdventureView::Unknown) {
            return Ok(None);
        }
        if let Some(index) = self.pending {
            let commit = self.commits[usize::from(index)].clone();
            match (commit.kind, view) {
                (CommitKind::Claim, AdventureView::Reward) => {
                    self.observed();
                    self.claims[usize::from(self.current_claim.ok_or("无本次收获")?)].reward_seen =
                        true;
                    self.stage = "Reward".into();
                }
                (CommitKind::Reward, AdventureView::Page(page)) => {
                    let i = usize::from(self.current_claim.ok_or("无本次收获")?);
                    let claim = &self.claims[i];
                    if page.count + 1 != claim.count_before || page.claimable {
                        return Ok(None);
                    }
                    self.observed();
                    self.claims[i].collected = true;
                    if self.mode.dispatches() {
                        self.dispatch_target = Some(self.claims[i].mission.clone());
                        self.stage = "PrepareDispatch".into();
                    } else {
                        self.current_claim = None;
                        self.claim_target = None;
                        self.stage = "ScanSquads".into();
                    }
                }
                (CommitKind::Duration, AdventureView::Page(page))
                    if page.selected.as_ref() == Some(&commit.mission)
                        && page.ready
                        && page.duration == Some(self.config.adventure_duration) =>
                {
                    self.observed();
                    self.duration_confirmed = true;
                    self.stage = "AutoSelect".into();
                }
                (CommitKind::Dispatch, AdventureView::Page(page))
                    if page.selected.as_ref() == Some(&commit.mission)
                        && page.running
                        && !page.claimable =>
                {
                    self.observed();
                    self.dispatch_result(DispatchOutcome::Dispatched)?;
                }
                (CommitKind::Dispatch, AdventureView::Rejection(reason))
                    if matches!(
                        reason,
                        DispatchOutcome::InsufficientEnergy | DispatchOutcome::NoTeams
                    ) =>
                {
                    self.observed();
                    self.dispatch_result(*reason)?;
                    return Ok(Some(A::CloseRejection));
                }
                _ => return Ok(None),
            }
            // 单次检查点先保存已观察结果，不在同一更新中再准备另一项提交。
            return Ok(None);
        }
        if self.completed() {
            return Ok(None);
        }
        if let AdventureView::Rejection(reason) = view {
            if self.dispatches.last().is_some_and(|r| r.outcome == *reason)
                || self
                    .claims
                    .last()
                    .is_some_and(|r| r.redispatch == Some(*reason))
            {
                return Ok(Some(A::CloseRejection));
            }
            return Err("未提交的游历拒绝窗口不能被认领");
        }
        if matches!(self.stage.as_str(), "AutoSelect" | "Dispatch") {
            if let AdventureView::Page(page) = view {
                if page.count == 4 && !page.running {
                    self.dispatch_result(DispatchOutcome::NoTeams)?;
                    return Ok(None);
                }
                if page.selected != self.dispatch_target
                    || page.duration != Some(self.config.adventure_duration)
                {
                    self.duration_confirmed = false;
                    self.bangboo_confirmed = false;
                    self.stage = "PrepareDispatch".into();
                    return Ok(None);
                }
                if self.stage == "Dispatch" && !page.bangboo_ready {
                    self.bangboo_confirmed = false;
                    self.stage = "AutoSelect".into();
                    return Ok(None);
                }
            } else if matches!(view, AdventureView::Duration { .. }) {
                self.duration_confirmed = false;
                self.bangboo_confirmed = false;
                self.stage = "Duration".into();
                return Ok(None);
            }
        }
        for _ in 0..8 {
            match self.stage.as_str() {
                "Inspect" => match view {
                    AdventureView::Entry => return Ok(Some(A::Open)),
                    AdventureView::Managed => return Err("仍在托管，无法进入手动游历"),
                    AdventureView::Page(_) => {
                        self.stage = if self.mode.claims() {
                            "ScanSquads"
                        } else {
                            "ChooseMission"
                        }
                        .into()
                    }
                    _ => return Err("游历起点不是入口或游历页面"),
                },
                "ScanSquads" => {
                    if let AdventureView::Page(page) = view {
                        if page.panel {
                            return Ok(Some(A::ClosePanel));
                        }
                        if page.count == 0 {
                            self.claims_checked = true;
                            self.stage = "ChooseMission".into();
                            continue;
                        }
                        if !page.squads_open {
                            return Ok(Some(A::Squads));
                        }
                        if !page.squads_readable {
                            return Ok(None);
                        }
                        if let Some(mission) = page
                            .completed
                            .iter()
                            .find(|m| !self.claims.iter().any(|c| &c.mission == *m))
                        {
                            self.claim_target = Some(mission.clone());
                            self.stage = "OpenCompleted".into();
                        } else {
                            self.claims_checked = true;
                            self.stage = "ChooseMission".into();
                        }
                    } else {
                        return Ok(None);
                    }
                }
                "OpenCompleted" => {
                    if let AdventureView::Page(page) = view {
                        let target = self.claim_target.as_ref().ok_or("缺少待收获副本")?;
                        if page.selected.as_ref() == Some(target)
                            && page.claimable
                            && page.count > 0
                        {
                            self.current_claim = Some(self.claims.len() as u8);
                            self.claims.push(AdventureClaim {
                                mission: target.clone(),
                                count_before: page.count,
                                reward_seen: false,
                                collected: false,
                                redispatch: None,
                            });
                            self.stage = "Claim".into();
                        } else if page.panel {
                            return Ok(Some(A::ClosePanel));
                        } else if !page.squads_open {
                            return Ok(Some(A::Squads));
                        } else if page.completed.contains(target) {
                            return Ok(Some(A::OpenCompleted(target.clone())));
                        } else {
                            return Ok(None);
                        }
                    } else {
                        return Ok(None);
                    }
                }
                "Claim" => {
                    if let AdventureView::Page(page) = view {
                        if page.claimable
                            && page.selected.as_deref()
                                == self
                                    .current_claim
                                    .map(|i| self.claims[usize::from(i)].mission.as_str())
                        {
                            return Ok(Some(A::Claim));
                        }
                        return Ok(None);
                    } else {
                        return Err("未投递收获却出现其他游历窗口");
                    }
                }
                "Reward" => {
                    if matches!(view, AdventureView::Reward) {
                        return Ok(Some(A::ConfirmReward));
                    } else {
                        return Ok(None);
                    }
                }
                "ChooseMission" => {
                    if !self.mode.dispatches() {
                        self.stage = "Return".into();
                        continue;
                    }
                    let target = if let Some(m) = &self.material_mission {
                        Some(m)
                    } else {
                        self.config.adventure_missions.get(self.dispatches.len())
                    };
                    if let Some(target) = target {
                        self.dispatch_target = Some(target.clone());
                        self.stage = "PrepareDispatch".into();
                    } else {
                        self.stage = "Return".into();
                    }
                }
                "PrepareDispatch" => {
                    if let AdventureView::Page(page) = view {
                        let target = self.dispatch_target.as_ref().ok_or("缺少待派遣副本")?;
                        if page.squads_open {
                            return Ok(Some(A::Squads));
                        }
                        if page.selected.as_ref() == Some(target) && page.running && !page.claimable
                        {
                            self.dispatch_result(DispatchOutcome::AlreadyRunning)?;
                            return Ok(None);
                        }
                        if page.count == 4 {
                            self.dispatch_result(DispatchOutcome::NoTeams)?;
                            return Ok(None);
                        }
                        if page.selected.as_ref() == Some(target) {
                            if page.energy_shortage {
                                self.dispatch_result(DispatchOutcome::InsufficientEnergy)?;
                                return Ok(None);
                            }
                            if page.ready {
                                self.stage = "Duration".into();
                                return Ok(Some(A::OpenDuration));
                            }
                            if page.claimable {
                                return Err("配置副本仍可收获，不能把未完成收获作为已派遣");
                            }
                            return Ok(None);
                        }
                        if self.mode == AdventureMode::Material {
                            return Err("补料游历不是父流程指定的副本");
                        }
                        if page
                            .selected
                            .as_ref()
                            .is_some_and(|m| district_key(m) == district_key(target))
                        {
                            return Ok(Some(A::SelectStage(target.clone())));
                        }
                        let mission = AdventureMission::parse(target).ok_or("副本配置无效")?;
                        if page.visible_titles.contains(&mission.title()) {
                            return Ok(Some(A::SelectMission(target.clone())));
                        }
                        if page.panel {
                            return Ok(Some(A::ClosePanel));
                        }
                        if self.pans >= 12 {
                            return Err("游历左右找图达到上限，未找到配置副本");
                        }
                        return Ok(Some(A::Pan(self.pans.is_multiple_of(2))));
                    } else {
                        return Ok(None);
                    }
                }
                "Duration" => match view {
                    AdventureView::Duration { options, selected } => {
                        if !options.contains(&self.config.adventure_duration) {
                            if [
                                AdventureDuration::Min15,
                                AdventureDuration::Hour1,
                                AdventureDuration::Hour2,
                                AdventureDuration::Hour6,
                                AdventureDuration::Hour12,
                                AdventureDuration::Hour20,
                            ]
                            .iter()
                            .all(|d| options.contains(d))
                            {
                                self.dispatch_result(DispatchOutcome::DurationUnavailable)?;
                                return Ok(None);
                            }
                            return Ok(None);
                        }
                        return Ok(Some(if *selected == Some(self.config.adventure_duration) {
                            A::ConfirmDuration
                        } else {
                            A::SelectDuration(self.config.adventure_duration)
                        }));
                    }
                    AdventureView::Page(page)
                        if page.selected == self.dispatch_target && page.ready =>
                    {
                        return Ok(Some(A::OpenDuration));
                    }
                    _ => return Ok(None),
                },
                "AutoSelect" => match view {
                    AdventureView::Page(page) if page.selected == self.dispatch_target => {
                        if page.energy_shortage {
                            self.dispatch_result(DispatchOutcome::InsufficientEnergy)?;
                            return Ok(None);
                        }
                        if page.ready
                            && page.duration == Some(self.config.adventure_duration)
                            && page.bangboo_ready
                        {
                            self.bangboo_confirmed = true;
                            self.stage = "Dispatch".into();
                        } else if page.ready {
                            return Ok(Some(A::AutoSelect));
                        } else {
                            return Ok(None);
                        }
                    }
                    AdventureView::Bangboo { ready } => {
                        return Ok(Some(if *ready {
                            A::ConfirmBangboo
                        } else {
                            A::AutoSelect
                        }));
                    }
                    _ => return Ok(None),
                },
                "Dispatch" => {
                    if let AdventureView::Page(page) = view {
                        if page.selected == self.dispatch_target
                            && page.ready
                            && page.bangboo_ready
                            && self.duration_confirmed
                            && self.bangboo_confirmed
                            && page.duration == Some(self.config.adventure_duration)
                        {
                            return Ok(Some(A::Dispatch));
                        }
                        return Ok(None);
                    } else {
                        return Ok(None);
                    }
                }
                "FinishMaterial" => match view {
                    AdventureView::Page(page)
                        if self
                            .dispatches
                            .last()
                            .is_some_and(|r| page.selected.as_ref() == Some(&r.mission)) =>
                    {
                        self.stage = "Done".into();
                        return Ok(None);
                    }
                    AdventureView::Duration { .. } | AdventureView::Bangboo { .. } => {
                        return Ok(Some(A::Back));
                    }
                    _ => return Ok(None),
                },
                "Return" => match view {
                    AdventureView::Entry => {
                        self.returned_entry = true;
                        self.stage = "Done".into();
                        return Ok(None);
                    }
                    AdventureView::Page(_)
                    | AdventureView::Duration { .. }
                    | AdventureView::Bangboo { .. } => return Ok(Some(A::Back)),
                    _ => return Ok(None),
                },
                _ => return Err("游历阶段未登记"),
            }
        }
        Err("游历单帧状态推进超过上限")
    }
}
pub fn district_key(mission: &str) -> Option<&str> {
    AdventureMission::parse(mission).map(|_| &mission[..mission.len() - 2])
}
pub fn parse_district_label(label: &str) -> Option<String> {
    let label = label
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .replace(['－', '—', '−'], "-");
    let area = label.chars().last()?;
    if !matches!(area, '1' | '2' | '3') {
        return None;
    }
    let prefix = label[..label.len() - 1].trim_end_matches('-');
    if prefix.chars().last().is_some_and(|c| c.is_ascii_digit())
        || prefix.chars().filter(char::is_ascii_digit).count() > 1
    {
        return None;
    }
    let mut candidates = vec![];
    for (word, key) in [
        ("制造区", "CRAFT"),
        ("社区旧址", "COMMUNITY"),
        ("科研院旧址", "RESEARCH"),
    ] {
        let length = prefix.chars().count();
        let expected = word.chars().count();
        if length.abs_diff(expected) <= 1
            && crate::ocr::matches_text(word, prefix, crate::ocr::TextMatchMode::Lcs, false, 0.8)
            && crate::ocr::matches_text(prefix, word, crate::ocr::TextMatchMode::Lcs, false, 0.8)
        {
            candidates.push(key);
        }
    }
    (candidates.len() == 1).then(|| format!("{}_{area}", candidates[0]))
}
pub fn parse_mission_label(label: &str) -> Option<String> {
    let label = label
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .replace(['－', '—', '−'], "-");
    let (district, stage) = label.rsplit_once('-')?;
    if !matches!(stage, "1" | "2" | "3" | "4") {
        return None;
    }
    Some(format!("{}_{stage}", parse_district_label(district)?))
}
pub fn parse_count(text: &str) -> Option<u8> {
    let text = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    let rest = text
        .strip_prefix("游历小队")?
        .trim_start_matches([':', '：']);
    match rest.strip_suffix("/4")? {
        "0" => Some(0),
        "1" => Some(1),
        "2" => Some(2),
        "3" => Some(3),
        "4" => Some(4),
        _ => None,
    }
}
pub fn parse_preview_duration(text: &str) -> Option<AdventureDuration> {
    let text = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    let text = text.strip_prefix("剩余")?;
    match text {
        "03分00秒" | "3分00秒" => Some(AdventureDuration::Min3),
        "15分00秒" => Some(AdventureDuration::Min15),
        "01时00分" | "1时00分" => Some(AdventureDuration::Hour1),
        "02时00分" | "2时00分" => Some(AdventureDuration::Hour2),
        "06时00分" | "6时00分" => Some(AdventureDuration::Hour6),
        "12时00分" => Some(AdventureDuration::Hour12),
        "20时00分" => Some(AdventureDuration::Hour20),
        _ => None,
    }
}
