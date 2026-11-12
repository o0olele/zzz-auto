//! 本次战斗失败后的撤退、退出和原副本免费次数核对。
use crate::{
    battle_result::NormalBattleResult,
    charge_transport::canonical_name,
    deep_hunt::same_mission,
    notorious_hunt_timeout::{WeeklyTimeoutFlow, WeeklyTimeoutResult},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyRecoveryView {
    Unknown,
    Blocked,
    Other,
    Retreat,
    Exit,
    Returned { mission_type: String, rewards: u8 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyRecoveryAction {
    Retreat,
    Exit,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Delivery {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyRecoveryFlow {
    battle: Option<NormalBattleResult>,
    #[serde(default)]
    timeout_exit: Option<WeeklyTimeoutFlow>,
    timeout_ms: u32,
    mission_type: String,
    rewards_before: u8,
    retreat_seen: bool,
    retreat: Option<Delivery>,
    exit_seen: bool,
    exit: Option<Delivery>,
    stable: u8,
    returned: bool,
}
impl WeeklyRecoveryFlow {
    pub fn new(
        battle: NormalBattleResult,
        timeout_ms: u32,
        mission_type: String,
        rewards_before: u8,
    ) -> Result<Self, &'static str> {
        let flow = Self {
            battle: Some(battle),
            timeout_exit: None,
            timeout_ms,
            mission_type,
            rewards_before,
            retreat_seen: false,
            retreat: None,
            exit_seen: false,
            exit: None,
            stable: 0,
            returned: false,
        };
        flow.validate()?;
        Ok(flow)
    }
    pub fn after_timeout(timeout_exit: WeeklyTimeoutFlow) -> Result<Self, &'static str> {
        timeout_exit.validate()?;
        if !timeout_exit.completed() {
            return Err("超时退出尚无实际失败结果页");
        }
        let request = timeout_exit.request();
        let flow = Self {
            battle: None,
            timeout_ms: request.timeout_ms,
            mission_type: request.mission_type.clone(),
            rewards_before: request.rewards_before,
            timeout_exit: Some(timeout_exit),
            retreat_seen: false,
            retreat: None,
            exit_seen: false,
            exit: None,
            stable: 0,
            returned: false,
        };
        flow.validate()?;
        Ok(flow)
    }
    fn requires_retreat(&self) -> bool {
        self.timeout_exit
            .as_ref()
            .is_none_or(|flow| flow.result() == Some(WeeklyTimeoutResult::Retreat))
    }
    pub fn mission_type(&self) -> &str {
        &self.mission_type
    }
    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }
    pub fn rewards_before(&self) -> u8 {
        self.rewards_before
    }
    pub fn completed(&self) -> bool {
        self.returned
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let origin = match (&self.battle, &self.timeout_exit) {
            (Some(NormalBattleResult::Retreat), None) => true,
            (None, Some(flow)) => {
                flow.validate().is_ok()
                    && flow.completed()
                    && flow.request().mission_type == self.mission_type
                    && flow.request().rewards_before == self.rewards_before
                    && flow.request().timeout_ms == self.timeout_ms
            }
            _ => false,
        };
        if !origin
            || canonical_name("恶名狩猎", &self.mission_type).is_none()
            || !(1000..=300_000).contains(&self.timeout_ms)
            || !(1..=3).contains(&self.rewards_before)
            || (self.retreat.is_some() && !self.retreat_seen)
            || (self.retreat_seen && !self.requires_retreat())
            || (self.exit_seen && self.requires_retreat() && self.retreat.is_none())
            || (self.exit.is_some() && !self.exit_seen)
            || self.stable > 2
            || (self.stable > 0 && self.exit.is_none())
            || self.returned != (self.stable == 2)
        {
            return Err("周常失败收尾缺少本次撤退、退出或原免费次数证明");
        }
        Ok(())
    }
    pub fn next(
        &mut self,
        view: &WeeklyRecoveryView,
    ) -> Result<Option<WeeklyRecoveryAction>, &'static str> {
        use WeeklyRecoveryAction as A;
        use WeeklyRecoveryView as V;
        self.validate()?;
        if self.completed() {
            return Ok(None);
        }
        if !matches!(view, V::Returned { .. }) {
            self.stable = 0;
        }
        match view {
            V::Unknown | V::Blocked => Ok(None),
            V::Other => Err("周常失败收尾进入无关页面"),
            V::Retreat if self.exit_seen || !self.requires_retreat() => {
                Err("周常退出后回到旧失败页面")
            }
            V::Retreat => {
                self.retreat_seen = true;
                Ok(self.retreat.is_none().then_some(A::Retreat))
            }
            V::Exit if self.requires_retreat() && self.retreat.is_none() => {
                Err("没有本次撤退意图，不能采用既有退出页面")
            }
            V::Exit => {
                self.exit_seen = true;
                Ok(self.exit.is_none().then_some(A::Exit))
            }
            V::Returned {
                mission_type,
                rewards,
            } => {
                if self.exit.is_none()
                    || !same_mission(&self.mission_type, mission_type)
                    || *rewards != self.rewards_before
                {
                    self.stable = 0;
                    return Err("周常失败退出后的副本或免费次数不匹配");
                }
                self.stable = (self.stable + 1).min(2);
                self.returned = self.stable == 2;
                Ok(None)
            }
        }
    }
    fn delivery(&mut self, a: WeeklyRecoveryAction) -> &mut Option<Delivery> {
        match a {
            WeeklyRecoveryAction::Retreat => &mut self.retreat,
            WeeklyRecoveryAction::Exit => &mut self.exit,
        }
    }
    pub fn prepare(&mut self, a: WeeklyRecoveryAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.completed()
            || match a {
                WeeklyRecoveryAction::Retreat => {
                    !self.retreat_seen || self.retreat.is_some() || self.exit_seen
                }
                WeeklyRecoveryAction::Exit => !self.exit_seen || self.exit.is_some(),
            }
        {
            return Err("周常失败收尾动作不能重复或跨步骤");
        }
        *self.delivery(a) = Some(Delivery::Intent);
        Ok(())
    }
    pub fn allows(&self, a: WeeklyRecoveryAction, view: &WeeklyRecoveryView) -> bool {
        self.validate().is_ok()
            && !self.completed()
            && match (a, view) {
                (WeeklyRecoveryAction::Retreat, WeeklyRecoveryView::Retreat) => {
                    self.retreat == Some(Delivery::Intent) && !self.exit_seen
                }
                (WeeklyRecoveryAction::Exit, WeeklyRecoveryView::Exit) => {
                    self.exit == Some(Delivery::Intent)
                }
                _ => false,
            }
    }
    pub fn posted(&mut self, a: WeeklyRecoveryAction) -> Result<(), &'static str> {
        if *self.delivery(a) != Some(Delivery::Intent) {
            return Err("周常失败收尾投递不匹配");
        }
        *self.delivery(a) = Some(Delivery::Posted);
        Ok(())
    }
    pub fn not_posted(&mut self, a: WeeklyRecoveryAction) -> Result<(), &'static str> {
        if *self.delivery(a) != Some(Delivery::Intent)
            || match a {
                WeeklyRecoveryAction::Retreat => self.exit_seen,
                WeeklyRecoveryAction::Exit => self.stable > 0,
            }
        {
            return Err("已投递或已推进的周常失败收尾不能撤销");
        }
        *self.delivery(a) = None;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.stable = 0;
        self.returned = false;
        Ok(())
    }
}
