//! 周奖励领取及面板关闭；领取操作、奖励展示和返回列表分别记录。
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyRewardsRequest {
    pub run_id: Uuid,
    pub collection_id: Uuid,
    pub timeout_ms: u32,
}
impl WeeklyRewardsRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.run_id.is_nil()
            || self.collection_id.is_nil()
            || self.run_id == self.collection_id
            || !(1000..=120_000).contains(&self.timeout_ms)
        {
            return Err("周奖励领取身份或时限无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyClaimState {
    Unknown,
    Available,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyRewardsView {
    Unknown,
    Blocked,
    Detail { mission_type: String, back: bool },
    List { back: bool },
    Panel { claim: WeeklyClaimState },
    Reward,
}
impl WeeklyRewardsView {
    fn validate(&self) -> Result<(), &'static str> {
        if let Self::Detail { mission_type, .. } = self
            && crate::charge_transport::canonical_name("恶名狩猎", mission_type).is_none()
        {
            return Err("周奖励详情不属于已知恶名狩猎副本");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyRewardsAction {
    BackDetail,
    Open,
    ClaimAll,
    DismissReward,
    ClosePanel,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Delivery {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyRewardsReceipt {
    pub run_id: Uuid,
    pub collection_id: Uuid,
    pub attempted: bool,
    pub reward_seen: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyRewardsFlow {
    request: WeeklyRewardsRequest,
    intents: BTreeMap<WeeklyRewardsAction, Delivery>,
    opened: bool,
    checked: bool,
    reward_seen: bool,
    candidate: Option<WeeklyRewardsView>,
    stable: u8,
    returned: bool,
}
impl WeeklyRewardsFlow {
    pub fn new(request: WeeklyRewardsRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            intents: BTreeMap::new(),
            opened: false,
            checked: false,
            reward_seen: false,
            candidate: None,
            stable: 0,
            returned: false,
        })
    }
    pub fn request(&self) -> &WeeklyRewardsRequest {
        &self.request
    }
    pub fn timeout_ms(&self) -> u32 {
        self.request.timeout_ms
    }
    pub fn action_intents(&self) -> Vec<WeeklyRewardsAction> {
        self.intents.keys().copied().collect()
    }
    pub fn completed(&self) -> bool {
        self.returned
    }
    pub fn receipt(&self) -> Option<WeeklyRewardsReceipt> {
        self.returned.then(|| WeeklyRewardsReceipt {
            run_id: self.request.run_id,
            collection_id: self.request.collection_id,
            attempted: self.intents.contains_key(&WeeklyRewardsAction::ClaimAll),
            reward_seen: self.reward_seen,
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        use WeeklyRewardsAction as A;
        self.request.validate()?;
        if let Some(v) = &self.candidate {
            v.validate()?;
        }
        if self.stable > 2
            || self.candidate.is_some() != (self.stable > 0)
            || self.opened && !self.intents.contains_key(&A::Open)
            || self.checked && !self.opened
            || self.reward_seen && !self.intents.contains_key(&A::ClaimAll)
            || self.intents.contains_key(&A::ClaimAll) && !self.opened
            || self.intents.contains_key(&A::DismissReward) && !self.reward_seen
            || self.intents.contains_key(&A::ClosePanel) && !self.checked
            || self.returned
                && !(self.checked
                    && self.intents.contains_key(&A::ClosePanel)
                    && self.stable == 2
                    && matches!(self.candidate, Some(WeeklyRewardsView::List { back: true })))
        {
            return Err("周奖励检查点与本次领取、关闭或返回证明不符");
        }
        Ok(())
    }
    fn reset(&mut self) {
        self.candidate = None;
        self.stable = 0;
        self.returned = false;
    }
    fn offer(&self, v: &WeeklyRewardsView) -> Option<WeeklyRewardsAction> {
        use WeeklyRewardsAction as A;
        use WeeklyRewardsView as V;
        let action = match v {
            V::Detail { back: true, .. } if !self.opened => A::BackDetail,
            V::List { back: true } if !self.opened => A::Open,
            V::Panel { claim: WeeklyClaimState::Available } if self.opened && !self.checked => A::ClaimAll,
            V::Panel { claim: WeeklyClaimState::Unavailable } if self.opened && self.checked => A::ClosePanel,
            V::Reward if self.reward_seen => A::DismissReward,
            _ => return None,
        };
        (!self.intents.contains_key(&action)).then_some(action)
    }
    pub fn next(&mut self, view: &WeeklyRewardsView) -> Result<Option<WeeklyRewardsAction>, &'static str> {
        use WeeklyRewardsAction as A;
        use WeeklyRewardsView as V;
        self.validate()?;
        view.validate()?;
        if self.returned {
            return Ok(None);
        }
        if matches!(view, V::Unknown | V::Blocked) {
            self.reset();
            return Ok(None);
        }
        match view {
            V::Panel { .. } if !self.intents.contains_key(&A::Open) => {
                return Err("没有本次周奖励打开意图，不能采用已有面板");
            }
            V::Reward if !self.intents.contains_key(&A::ClaimAll) => {
                return Err("没有本次周奖励领取意图，不能采用已有奖励");
            }
            V::Detail { .. } if self.opened => {
                return Err("周奖励未按本次面板关闭返回列表");
            }
            _ => (),
        }
        self.stable = if self.candidate.as_ref() == Some(view) { (self.stable + 1).min(2) } else { 1 };
        self.candidate = Some(view.clone());
        if self.stable < 2 {
            return Ok(None);
        }
        match view {
            V::Panel { claim } => {
                self.opened = true;
                if *claim == WeeklyClaimState::Unavailable {
                    // 完整按钮的明确禁用读数，不从按钮缺失推导已经领完。
                    self.checked = true;
                } else if *claim == WeeklyClaimState::Available && !self.intents.contains_key(&A::ClosePanel) {
                    self.checked = false;
                }
            }
            V::Reward => self.reward_seen = true,
            V::List { back: true } if self.checked && self.intents.contains_key(&A::ClosePanel) => {
                self.returned = true;
            }
            _ => (),
        }
        Ok((!self.returned).then(|| self.offer(view)).flatten())
    }
    pub fn prepare(&mut self, action: WeeklyRewardsAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.returned || self.stable != 2 || self.candidate.as_ref().and_then(|v| self.offer(v)) != Some(action) {
            return Err("周奖励动作缺少本次连续新帧证明");
        }
        self.intents.insert(action, Delivery::Intent);
        self.reset();
        Ok(())
    }
    pub fn allows(&self, action: WeeklyRewardsAction, view: &WeeklyRewardsView) -> bool {
        use WeeklyRewardsAction as A;
        use WeeklyRewardsView as V;
        self.validate().is_ok() && view.validate().is_ok() && !self.returned
            && self.intents.get(&action) == Some(&Delivery::Intent)
            && match (action, view) {
                (A::BackDetail, V::Detail { back: true, .. }) | (A::Open, V::List { back: true }) => !self.opened,
                (A::ClaimAll, V::Panel { claim: WeeklyClaimState::Available }) => self.opened && !self.checked,
                (A::DismissReward, V::Reward) => self.reward_seen,
                (A::ClosePanel, V::Panel { claim: WeeklyClaimState::Unavailable }) => self.checked,
                _ => false,
            }
    }
    pub fn posted(&mut self, action: WeeklyRewardsAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.intents.get(&action) != Some(&Delivery::Intent) {
            return Err("周奖励投递回执不属于当前意图");
        }
        self.intents.insert(action, Delivery::Posted);
        Ok(())
    }
    pub fn not_posted(&mut self, action: WeeklyRewardsAction) -> Result<(), &'static str> {
        use WeeklyRewardsAction as A;
        self.validate()?;
        if self.intents.get(&action) != Some(&Delivery::Intent) || self.candidate.is_some()
            || match action {
                A::BackDetail | A::Open => self.opened,
                A::ClaimAll => self.reward_seen || self.checked,
                A::DismissReward => self.checked,
                A::ClosePanel => self.returned,
            }
        {
            return Err("已投递或已推进的周奖励意图不能撤销");
        }
        self.intents.remove(&action);
        self.reset();
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.reset();
        Ok(())
    }
}

pub fn pipeline() -> serde_json::Value {
    use serde_json::json;
    let mut p = json!({});
    for (name, roi) in [
        ("Claim", [1192, 344, 202, 38]),
        ("Modal", [440, 200, 1040, 590]),
        ("RewardConfirm", [882, 708, 202, 50]),
        ("BattleRewardConfirm", [1091, 701, 78, 55]),
    ] {
        p[name] = json!({"recognition":"OcrText","model":"ppocrv5","threshold":0.6,"roi":roi});
    }
    for (name, lower, upper) in [("Enabled", 200, 255), ("Disabled", 80, 180)] {
        p[name] = json!({"recognition":"ColorMatch","roi":[0,0,1,1],"method":4,"lower":[lower,lower,lower],"upper":[upper,upper,upper],"count":20,"connected":false});
    }
    p
}
