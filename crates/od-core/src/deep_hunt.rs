//! 体力深度追猎独立目标、周常阻挡、模式及消费前置。
use crate::{
    battle_result::NormalBattleResult,
    charge_plan::{ChargePlanItem, RestoreCharge},
    charge_transport::{
        TrainingArrival, TrainingTransportReceipt, TrainingTransportRequest, canonical_name,
    },
    predefined_teams::TeamConfig,
    restore_charge::{RestoreFlow, RestoreRequest},
    simulation_cards::CardsCharge,
    simulation_deploy::DeploymentContext,
    simulation_settlement::{SettlementAction, SettlementView},
    simulation_teams::{TeamsRequest, TeamsResult},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HuntRequest {
    pub plan: ChargePlanItem,
    pub restore_charge: RestoreCharge,
    pub teams: TeamConfig,
    pub timeout_ms: u32,
}
impl HuntRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        TrainingTransportRequest {
            plan: self.plan.clone(),
            timeout_ms: self.timeout_ms,
        }
        .validate()?;
        if self.plan.category_name != "恶名狩猎" {
            return Err("深度追猎只接受普通恶名狩猎计划");
        }
        self.team_request()?.validate()
    }
    pub fn team_request(&self) -> Result<TeamsRequest, &'static str> {
        Ok(TeamsRequest {
            predefined_team_idx: self.plan.predefined_team_idx,
            teams: self.teams.clone(),
            script_name: self.plan.script(&self.teams)?.into(),
            timeout_ms: self.timeout_ms,
        })
    }
    pub fn accepts_title(&self, title: &str) -> bool {
        same_mission(self.plan.execution_mission_type(), title)
    }
}
pub fn same_mission(expected: &str, actual: &str) -> bool {
    let expected = canonical_name("恶名狩猎", expected);
    expected.is_some() && expected == canonical_name("恶名狩猎", actual)
}
pub fn valid_charge(c: CardsCharge) -> bool {
    c.validate().is_ok() && c.required_charge == 60
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyState {
    Remaining(u8),
    Exhausted,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeepState {
    On,
    Off,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HuntView {
    Unknown,
    Blocked,
    List {
        matches: u8,
    },
    Detail {
        mission_type: String,
        weekly: WeeklyState,
        mode: DeepState,
        charge: Option<CardsCharge>,
        no_reward: bool,
        back: bool,
        level: Option<String>,
    },
    Levels {
        names: Vec<String>,
    },
    ConfirmEnableDeep,
    RestorePopup,
    Deployment,
    Settlement,
    Reward,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HuntEntryStage {
    Inspect,
    AwaitDetail,
    AwaitList,
    ModeChange,
    AwaitPopup,
    Restore,
    Verify,
    AwaitLevelList,
    AwaitLevel,
    AwaitDeployment,
    Ready,
    Insufficient,
    WeeklyBlocked,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HuntEntryAction {
    OpenLevel,
    SelectLevel,
    BackToList,
    SelectMission,
    EnableDeep,
    ConfirmEnableDeep,
    Next,
    OpenRestore,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Receipt {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HuntEntryFlow {
    request: HuntRequest,
    route: TrainingTransportReceipt,
    stage: HuntEntryStage,
    before: Option<CardsCharge>,
    select: Option<Receipt>,
    back: Option<Receipt>,
    back_candidate: bool,
    enable: Option<Receipt>,
    confirm: Option<Receipt>,
    open: Option<Receipt>,
    next: Option<Receipt>,
    restored: bool,
    verified: bool,
    select_candidate: bool,
    enable_candidate: bool,
    confirm_seen: bool,
    unknown_frames: u8,
    verification_required: bool,
    weekly_left: Option<u8>,
    level_open: Option<Receipt>,
    level_choose: Option<Receipt>,
    level_before: Option<String>,
    level_verified: bool,
}
impl HuntEntryFlow {
    pub fn new(
        request: HuntRequest,
        route: TrainingTransportReceipt,
    ) -> Result<Self, &'static str> {
        let level_default = request.plan.level == "默认等级";
        let f = Self {
            request,
            route,
            stage: HuntEntryStage::Inspect,
            before: None,
            select: None,
            back: None,
            back_candidate: false,
            enable: None,
            confirm: None,
            open: None,
            next: None,
            restored: false,
            verified: false,
            select_candidate: false,
            enable_candidate: false,
            confirm_seen: false,
            unknown_frames: 0,
            verification_required: false,
            weekly_left: None,
            level_open: None,
            level_choose: None,
            level_before: None,
            level_verified: level_default,
        };
        f.validate()?;
        Ok(f)
    }
    pub fn request(&self) -> &HuntRequest {
        &self.request
    }
    pub fn stage(&self) -> HuntEntryStage {
        self.stage
    }
    pub fn charge(&self) -> Option<CardsCharge> {
        self.before
    }
    pub fn ready(&self) -> bool {
        self.stage == HuntEntryStage::Ready
    }
    pub fn insufficient(&self) -> bool {
        self.stage == HuntEntryStage::Insufficient
    }
    pub fn needs_verification(&self) -> bool {
        self.verification_required
    }
    pub fn weekly_blocked(&self) -> Option<u8> {
        (self.stage == HuntEntryStage::WeeklyBlocked)
            .then_some(self.weekly_left)
            .flatten()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        use HuntEntryStage as S;
        self.request.validate()?;
        if self.route.category_name != "恶名狩猎"
            || !self.request.accepts_title(&self.route.mission_type)
            || self.route.selection_verified
            || !matches!(&self.route.arrival, TrainingArrival::HuntList)
                && !matches!(&self.route.arrival,TrainingArrival::HuntDetail{mission_type} if canonical_name("恶名狩猎",mission_type).is_some())
            || self.before.is_some_and(|c| !valid_charge(c))
            || self.ready()
                && (!self.verified
                    || self.before.is_none_or(|c| c.current_charge < 60)
                    || self.next.is_none())
            || self.insufficient()
                && (!self.verified || self.before.is_none_or(|c| c.current_charge >= 60))
            || matches!(self.stage, S::AwaitPopup | S::Restore) && self.open.is_none()
            || self.stage == S::AwaitDeployment && self.next.is_none()
            || self.stage == S::AwaitDetail && self.select.is_none()
            || self.stage == S::AwaitList && self.back.is_none()
            || self.stage == S::ModeChange && self.enable.is_none()
            || self.confirm.is_some() && (self.enable.is_none() || !self.confirm_seen)
            || self.open.is_some() && self.request.restore_charge == RestoreCharge::None
            || self.restored && self.open.is_none()
            || self.unknown_frames > 3
            || self.weekly_left.is_some_and(|n| !(1..=3).contains(&n))
            || self.stage == S::WeeklyBlocked && self.weekly_left.is_none()
            || self.ready() && !self.level_verified
            || self.insufficient() && !self.level_verified
            || self.level_choose.is_some() && self.level_open.is_none()
            || matches!(self.stage, S::AwaitLevelList | S::AwaitLevel) && self.level_open.is_none()
            || self
                .level_before
                .as_ref()
                .is_some_and(|v| crate::charge_level::canonical_level(v).as_ref() != Some(v))
        {
            return Err("深度追猎检查点缺少本次目标、模式、电量或动作意图");
        }
        Ok(())
    }
    fn unknown(&mut self) {
        self.unknown_frames = (self.unknown_frames + 1).min(3);
        self.verification_required = self.unknown_frames == 3;
    }
    pub fn next(&mut self, v: &HuntView) -> Result<Option<HuntEntryAction>, &'static str> {
        self.validate()?;
        use HuntEntryAction as A;
        use HuntEntryStage as S;
        if self.ready()
            || self.insufficient()
            || self.weekly_blocked().is_some()
            || self.needs_verification()
        {
            return Ok(None);
        }
        match v {
            HuntView::Unknown => return Ok(None),
            HuntView::Blocked | HuntView::Settlement | HuntView::Reward | HuntView::Failed => {
                return Err("深度追猎准备出现未支持弹窗、结果或战斗");
            }
            _ => (),
        }
        if matches!(
            self.stage,
            S::Inspect
                | S::AwaitDetail
                | S::AwaitList
                | S::Verify
                | S::ModeChange
                | S::AwaitLevelList
                | S::AwaitLevel
        ) {
            match v {
                HuntView::List { matches }
                    if matches!(self.stage, S::Inspect | S::AwaitDetail | S::AwaitList) =>
                {
                    if *matches != 1 {
                        self.unknown();
                        return Ok(None);
                    }
                    if self.select.is_some() {
                        return Ok(None);
                    }
                    self.stage = S::Inspect;
                    self.select_candidate = true;
                    return Ok(Some(A::SelectMission));
                }
                HuntView::Levels { names } => {
                    if self.level_open.is_none()
                        || self.request.plan.level == "默认等级"
                        || !matches!(self.stage, S::AwaitLevelList | S::AwaitLevel)
                    {
                        return Err("不能采用已有等级菜单");
                    }
                    let reading = crate::charge_level::LevelReading {
                        current: None,
                        choices: names.clone(),
                    };
                    reading.validate()?;
                    if self.level_choose.is_some() {
                        return Ok(None);
                    }
                    if !names.contains(&self.request.plan.level) {
                        self.unknown();
                        return Ok(None);
                    }
                    return Ok(Some(A::SelectLevel));
                }
                HuntView::ConfirmEnableDeep => {
                    if self.stage != S::ModeChange || self.enable.is_none() {
                        return Err("不能采用既有深度追猎确认");
                    }
                    self.confirm_seen = true;
                    return Ok(self.confirm.is_none().then_some(A::ConfirmEnableDeep));
                }
                HuntView::Detail {
                    mission_type,
                    weekly,
                    mode,
                    charge,
                    no_reward,
                    back,
                    level,
                } => {
                    if !self.request.accepts_title(mission_type) {
                        if self.stage == S::AwaitList {
                            return Ok(None);
                        }
                        if self.stage != S::Inspect || self.back.is_some() || self.select.is_some()
                        {
                            return Err("重新选定的追猎详情发生目标变化");
                        }
                        if !*back || canonical_name("恶名狩猎", mission_type).is_none() {
                            self.unknown();
                            return Ok(None);
                        }
                        self.back_candidate = true;
                        return Ok(Some(A::BackToList));
                    }
                    if (matches!(self.route.arrival, TrainingArrival::HuntList)
                        || matches!(&self.route.arrival,TrainingArrival::HuntDetail{mission_type} if !self.request.accepts_title(mission_type))
                        || self.back.is_some())
                        && self.select.is_none()
                    {
                        return Err("深度追猎详情不属于本次完整目标选择");
                    }
                    match weekly {
                        WeeklyState::Remaining(n) if (1..=3).contains(n) => {
                            self.weekly_left = Some(*n);
                            self.stage = S::WeeklyBlocked;
                            self.verified = false;
                            return Ok(None);
                        }
                        WeeklyState::Exhausted => (),
                        _ => {
                            self.unknown();
                            return Ok(None);
                        }
                    }
                    match mode {
                        DeepState::Unknown => {
                            self.unknown();
                            return Ok(None);
                        }
                        DeepState::Off if *no_reward => {
                            if self.stage == S::ModeChange {
                                return Ok(None);
                            }
                            if self.enable.is_some() {
                                return Err("已开启的深度追猎重新关闭，不能重复投递");
                            }
                            self.enable_candidate = true;
                            self.verified = false;
                            self.before = None;
                            return Ok(Some(A::EnableDeep));
                        }
                        DeepState::Off => {
                            self.unknown();
                            return Ok(None);
                        }
                        DeepState::On => {
                            if *no_reward {
                                self.unknown();
                                return Ok(None);
                            }
                            if self.request.plan.level != "默认等级" {
                                let Some(current) = level.as_ref().filter(|l| {
                                    crate::charge_level::canonical_level(l).as_ref() == Some(*l)
                                }) else {
                                    self.unknown();
                                    return Ok(None);
                                };
                                if current != &self.request.plan.level {
                                    self.level_verified = false;
                                    if self.level_choose.is_some() {
                                        self.unknown();
                                        return Ok(None);
                                    }
                                    if self.level_open.is_some() {
                                        return Ok(None);
                                    }
                                    self.level_before = Some(current.clone());
                                    return Ok(Some(A::OpenLevel));
                                }
                                self.level_verified = true;
                            }
                            let Some(c) = charge.filter(|c| valid_charge(*c)) else {
                                self.unknown();
                                return Ok(None);
                            };
                            if self.confirm_seen && self.confirm.is_none() {
                                return Err("缺少本次深度追猎开启确认意图");
                            }
                            self.verified = true;
                            self.before = Some(c);
                            self.unknown_frames = 0;
                            self.stage = if self.restored { S::Verify } else { S::Inspect };
                            if c.current_charge >= 60 {
                                return Ok(Some(A::Next));
                            }
                            if self.request.restore_charge == RestoreCharge::None || self.restored {
                                self.stage = S::Insufficient;
                                return Ok(None);
                            }
                            return Ok(Some(A::OpenRestore));
                        }
                    }
                }
                _ => return Err("深度追猎没有本次下一步意图，不能采用已有出战或恢复弹窗"),
            }
        }
        match (self.stage, v) {
            (S::AwaitPopup, HuntView::RestorePopup) => self.stage = S::Restore,
            (S::AwaitPopup, HuntView::Deployment) => return Err("恢复意图后意外进入出战页"),
            (S::AwaitDeployment, HuntView::Deployment) => self.stage = S::Ready,
            (S::AwaitDeployment, HuntView::RestorePopup) => return Err("足额下一步后出现恢复弹窗"),
            _ => (),
        }
        self.validate()?;
        Ok(None)
    }
    fn receipt(&mut self, a: HuntEntryAction) -> &mut Option<Receipt> {
        match a {
            HuntEntryAction::OpenLevel => &mut self.level_open,
            HuntEntryAction::SelectLevel => &mut self.level_choose,
            HuntEntryAction::BackToList => &mut self.back,
            HuntEntryAction::SelectMission => &mut self.select,
            HuntEntryAction::EnableDeep => &mut self.enable,
            HuntEntryAction::ConfirmEnableDeep => &mut self.confirm,
            HuntEntryAction::Next => &mut self.next,
            HuntEntryAction::OpenRestore => &mut self.open,
        }
    }
    pub fn prepare(&mut self, a: HuntEntryAction) -> Result<(), &'static str> {
        self.validate()?;
        use HuntEntryAction as A;
        use HuntEntryStage as S;
        let valid = match a {
            A::OpenLevel => {
                matches!(
                    self.stage,
                    S::Inspect | S::Verify | S::ModeChange | S::AwaitDetail
                ) && self.request.plan.level != "默认等级"
                    && self.level_before.is_some()
                    && !self.level_verified
            }
            A::SelectLevel => self.stage == S::AwaitLevelList && self.level_open.is_some(),
            A::BackToList => {
                self.stage == S::Inspect && self.back_candidate && self.select.is_none()
            }
            A::SelectMission => self.stage == S::Inspect && self.select_candidate,
            A::EnableDeep => {
                matches!(self.stage, S::Inspect | S::AwaitDetail | S::Verify)
                    && self.enable_candidate
            }
            A::ConfirmEnableDeep => {
                self.stage == S::ModeChange && self.enable.is_some() && self.confirm_seen
            }
            A::Next => {
                self.verified
                    && self.level_verified
                    && self.before.is_some_and(|c| c.current_charge >= 60)
                    && matches!(self.stage, S::Inspect | S::Verify)
            }
            A::OpenRestore => {
                self.verified
                    && self.level_verified
                    && self.before.is_some_and(|c| c.current_charge < 60)
                    && self.request.restore_charge != RestoreCharge::None
                    && !self.restored
            }
        };
        if !valid || self.receipt(a).is_some() || self.verification_required {
            return Err("没有待保存的深度追猎动作");
        }
        *self.receipt(a) = Some(Receipt::Intent);
        self.stage = match a {
            A::OpenLevel => S::AwaitLevelList,
            A::SelectLevel => S::AwaitLevel,
            A::BackToList => S::AwaitList,
            A::SelectMission => S::AwaitDetail,
            A::EnableDeep => S::ModeChange,
            A::ConfirmEnableDeep => S::ModeChange,
            A::Next => S::AwaitDeployment,
            A::OpenRestore => S::AwaitPopup,
        };
        self.validate()
    }
    pub fn allows(&self, a: HuntEntryAction, v: &HuntView) -> bool {
        let mut copy = self.clone();
        if *copy.receipt(a) != Some(Receipt::Intent) {
            return false;
        }
        match (a, v) {
            (
                HuntEntryAction::OpenLevel,
                HuntView::Detail {
                    mission_type,
                    weekly: WeeklyState::Exhausted,
                    mode: DeepState::On,
                    no_reward: false,
                    level: Some(level),
                    ..
                },
            ) => {
                self.request.accepts_title(mission_type)
                    && self.level_before.as_ref() == Some(level)
                    && level != &self.request.plan.level
            }
            (HuntEntryAction::SelectLevel, HuntView::Levels { names }) => {
                self.level_open.is_some()
                    && crate::charge_level::LevelReading {
                        current: None,
                        choices: names.clone(),
                    }
                    .validate()
                    .is_ok()
                    && names.contains(&self.request.plan.level)
            }
            (
                HuntEntryAction::BackToList,
                HuntView::Detail {
                    mission_type,
                    back: true,
                    ..
                },
            ) => {
                !self.request.accepts_title(mission_type)
                    && canonical_name("恶名狩猎", mission_type).is_some()
            }
            (HuntEntryAction::SelectMission, HuntView::List { matches: 1 }) => true,
            (
                HuntEntryAction::EnableDeep,
                HuntView::Detail {
                    mission_type,
                    weekly: WeeklyState::Exhausted,
                    mode: DeepState::Off,
                    no_reward: true,
                    ..
                },
            ) => self.request.accepts_title(mission_type),
            (HuntEntryAction::ConfirmEnableDeep, HuntView::ConfirmEnableDeep) => {
                self.enable.is_some() && self.confirm_seen
            }
            (
                HuntEntryAction::Next | HuntEntryAction::OpenRestore,
                HuntView::Detail {
                    mission_type,
                    weekly: WeeklyState::Exhausted,
                    mode: DeepState::On,
                    charge: Some(c),
                    no_reward: false,
                    level,
                    ..
                },
            ) => {
                self.request.accepts_title(mission_type)
                    && Some(*c) == self.before
                    && valid_charge(*c)
                    && (self.request.plan.level == "默认等级"
                        || level.as_deref() == Some(self.request.plan.level.as_str()))
            }
            _ => false,
        }
    }
    pub fn posted(&mut self, a: HuntEntryAction) -> Result<(), &'static str> {
        if *self.receipt(a) != Some(Receipt::Intent) {
            return Err("深度追猎动作意图不符");
        }
        *self.receipt(a) = Some(Receipt::Posted);
        self.validate()
    }
    pub fn not_posted(&mut self, a: HuntEntryAction) -> Result<(), &'static str> {
        if *self.receipt(a) != Some(Receipt::Intent) {
            return Err("已投递深度追猎动作不能撤销");
        }
        *self.receipt(a) = None;
        if a == HuntEntryAction::SelectLevel {
            self.stage = HuntEntryStage::AwaitLevelList;
            return self.validate();
        }
        if a != HuntEntryAction::ConfirmEnableDeep {
            self.stage = if self.restored {
                HuntEntryStage::Verify
            } else {
                HuntEntryStage::Inspect
            };
            self.verified = false;
            self.before = None;
        }
        self.validate()
    }
    pub fn restore_request(&self) -> Result<RestoreRequest, &'static str> {
        self.validate()?;
        if self.stage != HuntEntryStage::Restore {
            return Err("深度追猎尚未见到本次恢复主弹窗");
        }
        let r = RestoreRequest {
            policy: self.request.restore_charge,
            current_charge: self.before.ok_or("深度追猎电量未知")?.current_charge,
            required_charge: 60,
            timeout_ms: self.request.timeout_ms.min(300_000),
        };
        r.validate()?;
        Ok(r)
    }
    pub fn restored(&mut self, f: &RestoreFlow) -> Result<(), &'static str> {
        f.validate()?;
        if !f.completed() || f.request() != self.restore_request()? || f.charge_after().is_none() {
            return Err("深度追猎恢复证明不一致");
        }
        self.restored = true;
        self.level_verified = self.request.plan.level == "默认等级";
        self.verified = false;
        self.before = None;
        self.stage = HuntEntryStage::Verify;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.verification_required = false;
        if self.request.plan.level != "默认等级"
            && !matches!(
                self.stage,
                HuntEntryStage::AwaitDeployment | HuntEntryStage::Ready
            )
        {
            self.level_verified = false;
        }
        self.unknown_frames = 0;
        if self.ready() {
            self.stage = HuntEntryStage::AwaitDeployment;
        } else if self.insufficient() || self.weekly_blocked().is_some() {
            self.stage = HuntEntryStage::Verify;
            self.weekly_left = None;
        }
        if matches!(
            self.stage,
            HuntEntryStage::Inspect
                | HuntEntryStage::Verify
                | HuntEntryStage::AwaitDetail
                | HuntEntryStage::ModeChange
        ) {
            self.verified = false;
            self.before = None;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HuntDeployRequest {
    pub entry: HuntEntryFlow,
    pub team: TeamsResult,
    pub timeout_ms: u32,
}
impl DeploymentContext for HuntDeployRequest {
    fn validate(&self) -> Result<(), &'static str> {
        self.entry.validate()?;
        let expected = self.entry.request.team_request()?;
        if !self.entry.ready()
            || !(1000..=900_000).contains(&self.timeout_ms)
            || self.team.predefined_team_idx != expected.predefined_team_idx
            || self.team.script_name != expected.script_name
            || self.team.selection_verified
        {
            return Err("深度追猎出战缺少本次准备与选队证明");
        }
        Ok(())
    }
    fn team(&self) -> &TeamsResult {
        &self.team
    }
    fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }
    fn requires_forward(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HuntSettlementFlow {
    battle: NormalBattleResult,
    timeout_ms: u32,
    mission_type: String,
    finish: Option<Receipt>,
    settlement_seen: bool,
    reward_seen: bool,
    confirm: Option<Receipt>,
    returned: Option<CardsCharge>,
}
impl HuntSettlementFlow {
    pub fn new(
        battle: NormalBattleResult,
        timeout_ms: u32,
        mission_type: String,
    ) -> Result<Self, &'static str> {
        let f = Self {
            battle,
            mission_type,
            timeout_ms,
            finish: None,
            settlement_seen: false,
            reward_seen: false,
            confirm: None,
            returned: None,
        };
        f.validate()?;
        Ok(f)
    }
    pub fn mission_type(&self) -> &str {
        &self.mission_type
    }
    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }
    pub fn completed(&self) -> bool {
        self.returned.is_some()
    }
    pub fn charge(&self) -> Option<CardsCharge> {
        self.returned
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if canonical_name("恶名狩猎", &self.mission_type).is_none()
            || self.battle != NormalBattleResult::Completed
            || !(1000..=300_000).contains(&self.timeout_ms)
            || (self.finish.is_some() && !self.settlement_seen)
            || (self.reward_seen && self.finish.is_none())
            || (self.confirm.is_some() && !self.reward_seen)
            || (self.returned.is_some()
                && (self.finish.is_none() || (self.reward_seen && self.confirm.is_none())))
            || self.returned.is_some_and(|c| !valid_charge(c))
        {
            return Err("深度追猎结算缺少本次战斗、完成或返回证明");
        }
        Ok(())
    }
    pub fn next(&mut self, v: &SettlementView) -> Result<Option<SettlementAction>, &'static str> {
        self.validate()?;
        if self.completed() {
            return Ok(None);
        }
        match v {
            SettlementView::Unknown | SettlementView::Blocked => Ok(None),
            SettlementView::Failed | SettlementView::Other => Err("深度追猎结算进入失败或无关页面"),
            SettlementView::Settlement if self.reward_seen => Err("领奖后返回旧结算"),
            SettlementView::Settlement if self.finish.is_none() => {
                self.settlement_seen = true;
                Ok(Some(SettlementAction::Finish))
            }
            SettlementView::Settlement => Ok(None),
            SettlementView::Reward if self.finish.is_none() => {
                Err("没有本次完成意图，不能采用既有奖励")
            }
            SettlementView::Reward => {
                self.reward_seen = true;
                Ok(self
                    .confirm
                    .is_none()
                    .then_some(SettlementAction::ConfirmReward))
            }
            SettlementView::Returned(c) => {
                if !valid_charge(*c)
                    || self.finish.is_none()
                    || (self.reward_seen && self.confirm.is_none())
                {
                    return Err("深度追猎未完成本次结算或奖励确认");
                }
                self.returned = Some(*c);
                Ok(None)
            }
        }
    }
    fn receipt(&mut self, a: SettlementAction) -> &mut Option<Receipt> {
        match a {
            SettlementAction::Finish => &mut self.finish,
            SettlementAction::ConfirmReward => &mut self.confirm,
        }
    }
    pub fn prepare(&mut self, a: SettlementAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.completed()
            || match a {
                SettlementAction::Finish => !self.settlement_seen || self.finish.is_some(),
                SettlementAction::ConfirmReward => !self.reward_seen || self.confirm.is_some(),
            }
        {
            return Err("深度追猎结算动作不能重复或跨步骤");
        }
        *self.receipt(a) = Some(Receipt::Intent);
        Ok(())
    }
    pub fn allows(&self, a: SettlementAction, v: &SettlementView) -> bool {
        self.validate().is_ok()
            && !self.completed()
            && match (a, v) {
                (SettlementAction::Finish, SettlementView::Settlement) => {
                    self.finish == Some(Receipt::Intent) && !self.reward_seen
                }
                (SettlementAction::ConfirmReward, SettlementView::Reward) => {
                    self.reward_seen && self.confirm == Some(Receipt::Intent)
                }
                _ => false,
            }
    }
    pub fn posted(&mut self, a: SettlementAction) -> Result<(), &'static str> {
        if *self.receipt(a) != Some(Receipt::Intent) {
            return Err("深度追猎结算投递不匹配");
        }
        *self.receipt(a) = Some(Receipt::Posted);
        Ok(())
    }
    pub fn not_posted(&mut self, a: SettlementAction) -> Result<(), &'static str> {
        if *self.receipt(a) != Some(Receipt::Intent) {
            return Err("已投递结算不能撤销");
        }
        *self.receipt(a) = None;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.returned = None;
        Ok(())
    }
}
