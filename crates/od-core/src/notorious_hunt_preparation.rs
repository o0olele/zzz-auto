//! 免费周常从已观察详情到出战页；只准备等级和队伍，不证明挑战完成。
use crate::{
    charge_level::LevelReading,
    charge_plan::ChargePlanItem,
    deep_hunt::same_mission,
    notorious_hunt_entry::{WeeklyEntryFlow, WeeklyEntryRequest},
    predefined_teams::TeamConfig,
    simulation_teams::TeamsRequest,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyPreparationRequest {
    pub plan: ChargePlanItem,
    pub teams: TeamConfig,
    pub timeout_ms: u32,
}
impl WeeklyPreparationRequest {
    pub fn entry_request(&self) -> WeeklyEntryRequest {
        WeeklyEntryRequest {
            plan: self.plan.clone(),
            timeout_ms: self.timeout_ms,
        }
    }
    pub fn team_request(&self) -> Result<TeamsRequest, &'static str> {
        Ok(TeamsRequest {
            predefined_team_idx: self.plan.predefined_team_idx,
            teams: self.teams.clone(),
            script_name: self.plan.script(&self.teams)?.into(),
            timeout_ms: self.timeout_ms,
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.entry_request().validate()?;
        self.team_request()?.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyPreparationView {
    Unknown,
    Blocked,
    Detail {
        mission_type: String,
        rewards: Option<u8>,
        level: Option<String>,
    },
    Levels {
        mission_type: String,
        rewards: u8,
        names: Vec<String>,
    },
    Deployment,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyPreparationAction {
    OpenLevel,
    SelectLevel,
    Next,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyPreparationStage {
    Inspect,
    AwaitLevels,
    AwaitLevel,
    AwaitDeployment,
    Ready,
    AwaitingVerification,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Delivery {
    Intent,
    Posted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyPreparationFlow {
    request: WeeklyPreparationRequest,
    rewards_left: u8,
    stage: WeeklyPreparationStage,
    open: Option<Delivery>,
    select: Option<Delivery>,
    next: Option<Delivery>,
    candidate: Option<WeeklyPreparationAction>,
    stable: u8,
    unknown: u8,
}
impl WeeklyPreparationFlow {
    pub fn after_entry(
        request: WeeklyPreparationRequest,
        entry: &WeeklyEntryFlow,
    ) -> Result<Self, &'static str> {
        request.validate()?;
        entry.validate()?;
        if request.entry_request() != *entry.request() {
            return Err("周常准备与本次入口的冻结计划不一致");
        }
        let rewards_left = entry
            .rewards()
            .filter(|n| (1..=3).contains(n))
            .ok_or("周常准备需要本次入口观察到的免费余量")?;
        Ok(Self {
            request,
            rewards_left,
            stage: WeeklyPreparationStage::Inspect,
            open: None,
            select: None,
            next: None,
            candidate: None,
            stable: 0,
            unknown: 0,
        })
    }
    pub fn request(&self) -> &WeeklyPreparationRequest {
        &self.request
    }
    pub fn stage(&self) -> WeeklyPreparationStage {
        self.stage
    }
    pub fn rewards_left(&self) -> u8 {
        self.rewards_left
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        use WeeklyPreparationStage as S;
        self.request.validate()?;
        if !(1..=3).contains(&self.rewards_left)
            || self.stable > 2
            || self.unknown > 3
            || (self.candidate.is_none() != (self.stable == 0))
            || self.select.is_some() && self.open.is_none()
            || self.request.plan.level == "默认等级"
                && (self.open.is_some() || self.select.is_some())
            || self.stage == S::AwaitLevels
                && (self.open.is_none() || self.select.is_some() || self.next.is_some())
            || self.stage == S::AwaitLevel && (self.select.is_none() || self.next.is_some())
            || matches!(self.stage, S::AwaitDeployment | S::Ready) && self.next.is_none()
            || self.stage == S::Inspect
                && (self.open.is_some() || self.select.is_some() || self.next.is_some())
            || self.stage == S::AwaitingVerification && self.unknown != 3
            || self.next.is_some()
                && !matches!(
                    self.stage,
                    S::AwaitDeployment | S::Ready | S::AwaitingVerification
                )
        {
            return Err("周常准备检查点缺少对应的免费余量或输入意图");
        }
        Ok(())
    }
    fn reset_observations(&mut self) {
        self.candidate = None;
        self.stable = 0;
    }
    fn uncertain(&mut self) {
        self.reset_observations();
        self.unknown = (self.unknown + 1).min(3);
        if self.unknown == 3 {
            self.stage = WeeklyPreparationStage::AwaitingVerification;
        }
    }
    fn matches_detail(&self, v: &WeeklyPreparationView) -> bool {
        matches!(v, WeeklyPreparationView::Detail { mission_type, rewards: Some(n), .. }
            if same_mission(&self.request.plan.mission_type_name, mission_type) && *n == self.rewards_left)
    }
    fn level_selected(&self, v: &WeeklyPreparationView) -> bool {
        matches!(v, WeeklyPreparationView::Detail { level, .. }
            if self.request.plan.level == "默认等级" || level.as_ref() == Some(&self.request.plan.level))
    }
    fn matching_menu(&self, v: &WeeklyPreparationView) -> bool {
        matches!(v, WeeklyPreparationView::Levels { mission_type, rewards, names }
            if same_mission(&self.request.plan.mission_type_name, mission_type)
                && *rewards == self.rewards_left
                && (LevelReading { current: None, choices: names.clone() }).validate().is_ok()
                && names.contains(&self.request.plan.level))
    }
    pub fn next(
        &mut self,
        v: &WeeklyPreparationView,
    ) -> Result<Option<WeeklyPreparationAction>, &'static str> {
        self.validate()?;
        use WeeklyPreparationAction as A;
        use WeeklyPreparationStage as S;
        if matches!(self.stage, S::Ready | S::AwaitingVerification) {
            return Ok(None);
        }
        let action = match self.stage {
            S::Inspect | S::AwaitLevel if self.matches_detail(v) => {
                if self.level_selected(v) {
                    Some(A::Next)
                } else if self.open.is_none()
                    && matches!(v, WeeklyPreparationView::Detail { level: Some(l), .. }
                    if crate::charge_level::canonical_level(l).as_ref() == Some(l))
                {
                    Some(A::OpenLevel)
                } else {
                    None
                }
            }
            S::AwaitLevels if self.matching_menu(v) => Some(A::SelectLevel),
            S::AwaitDeployment if *v == WeeklyPreparationView::Deployment => {
                self.reset_observations();
                self.unknown = 0;
                self.stage = S::Ready;
                return Ok(None);
            }
            // 已投递后等实际后态，不靠重复点击推进，也不接受外部打开的出战页。
            S::AwaitDeployment if self.matches_detail(v) && self.level_selected(v) => {
                self.reset_observations();
                self.unknown = 0;
                return Ok(None);
            }
            _ => None,
        };
        let Some(action) = action else {
            self.uncertain();
            return Ok(None);
        };
        self.unknown = 0;
        if self.candidate == Some(action) {
            self.stable = (self.stable + 1).min(2);
        } else {
            self.candidate = Some(action);
            self.stable = 1;
        }
        Ok((self.stable == 2).then_some(action))
    }
    pub fn prepare(&mut self, action: WeeklyPreparationAction) -> Result<(), &'static str> {
        self.validate()?;
        use WeeklyPreparationAction as A;
        use WeeklyPreparationStage as S;
        if self.candidate != Some(action)
            || self.stable != 2
            || !match action {
                A::OpenLevel => self.stage == S::Inspect && self.open.is_none(),
                A::SelectLevel => self.stage == S::AwaitLevels && self.select.is_none(),
                A::Next => matches!(self.stage, S::Inspect | S::AwaitLevel) && self.next.is_none(),
            }
        {
            return Err("周常准备缺少两张新帧证明或输入已登记");
        }
        let (receipt, stage) = match action {
            A::OpenLevel => (&mut self.open, S::AwaitLevels),
            A::SelectLevel => (&mut self.select, S::AwaitLevel),
            A::Next => (&mut self.next, S::AwaitDeployment),
        };
        *receipt = Some(Delivery::Intent);
        self.stage = stage;
        self.reset_observations();
        Ok(())
    }
    pub fn allows(&self, action: WeeklyPreparationAction, v: &WeeklyPreparationView) -> bool {
        use WeeklyPreparationAction as A;
        use WeeklyPreparationStage as S;
        self.validate().is_ok()
            && match action {
                A::OpenLevel => {
                    self.stage == S::AwaitLevels
                        && self.open == Some(Delivery::Intent)
                        && self.matches_detail(v)
                        && !self.level_selected(v)
                        && matches!(v, WeeklyPreparationView::Detail { level: Some(l), .. }
                    if crate::charge_level::canonical_level(l).as_ref() == Some(l))
                }
                A::SelectLevel => {
                    self.stage == S::AwaitLevel
                        && self.select == Some(Delivery::Intent)
                        && self.matching_menu(v)
                }
                A::Next => {
                    self.stage == S::AwaitDeployment
                        && self.next == Some(Delivery::Intent)
                        && self.matches_detail(v)
                        && self.level_selected(v)
                }
            }
    }
    fn delivery(&mut self, a: WeeklyPreparationAction) -> &mut Option<Delivery> {
        match a {
            WeeklyPreparationAction::OpenLevel => &mut self.open,
            WeeklyPreparationAction::SelectLevel => &mut self.select,
            WeeklyPreparationAction::Next => &mut self.next,
        }
    }
    pub fn posted(&mut self, a: WeeklyPreparationAction) -> Result<(), &'static str> {
        if *self.delivery(a) != Some(Delivery::Intent) {
            return Err("周常准备动作未保存意图");
        }
        *self.delivery(a) = Some(Delivery::Posted);
        Ok(())
    }
    pub fn not_posted(&mut self, a: WeeklyPreparationAction) -> Result<(), &'static str> {
        if *self.delivery(a) != Some(Delivery::Intent) {
            return Err("不能撤销已投递的周常准备动作");
        }
        *self.delivery(a) = None;
        self.stage = match a {
            WeeklyPreparationAction::OpenLevel => WeeklyPreparationStage::Inspect,
            WeeklyPreparationAction::SelectLevel => WeeklyPreparationStage::AwaitLevels,
            WeeklyPreparationAction::Next if self.select.is_some() => {
                WeeklyPreparationStage::AwaitLevel
            }
            WeeklyPreparationAction::Next => WeeklyPreparationStage::Inspect,
        };
        self.reset_observations();
        self.unknown = 0;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.stage == WeeklyPreparationStage::AwaitingVerification {
            return Ok(());
        }
        self.reset_observations();
        self.unknown = 0;
        if self.stage == WeeklyPreparationStage::Ready {
            self.stage = WeeklyPreparationStage::AwaitDeployment;
        }
        Ok(())
    }
}
