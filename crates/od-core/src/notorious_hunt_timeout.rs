//! 本次周常移动或战斗超时后的菜单退出；动作与返回结果分别确认。
use crate::charge_transport::canonical_name;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyTimeoutCause {
    Movement,
    Combat,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyTimeoutRequest {
    pub cause: WeeklyTimeoutCause,
    pub mission_type: String,
    pub rewards_before: u8,
    pub timeout_ms: u32,
}
impl WeeklyTimeoutRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if canonical_name("恶名狩猎", &self.mission_type).is_none()
            || !(1..=3).contains(&self.rewards_before)
            || !(1000..=60_000).contains(&self.timeout_ms)
        {
            return Err("周常超时退出的副本、免费次数或预算无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyTimeoutView {
    Unknown,
    Blocked,
    Other,
    Hud,
    Menu,
    ConfirmExit,
    Retreat,
    Exit,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyTimeoutAction {
    OpenMenu,
    Leave,
    Confirm,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeeklyTimeoutResult {
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
pub struct WeeklyTimeoutFlow {
    request: WeeklyTimeoutRequest,
    open: Option<Delivery>,
    leave: Option<Delivery>,
    confirm: Option<Delivery>,
    menu_seen: bool,
    confirm_seen: bool,
    exit_seen: bool,
    candidate: Option<WeeklyTimeoutView>,
    stable: u8,
    result: Option<WeeklyTimeoutResult>,
}
impl WeeklyTimeoutFlow {
    pub fn new(request: WeeklyTimeoutRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            open: None,
            leave: None,
            confirm: None,
            menu_seen: false,
            confirm_seen: false,
            exit_seen: false,
            candidate: None,
            stable: 0,
            result: None,
        })
    }
    pub fn request(&self) -> &WeeklyTimeoutRequest {
        &self.request
    }
    pub fn timeout_ms(&self) -> u32 {
        self.request.timeout_ms
    }
    pub fn mission_type(&self) -> &str {
        &self.request.mission_type
    }
    pub fn result(&self) -> Option<WeeklyTimeoutResult> {
        self.result
    }
    pub fn completed(&self) -> bool {
        self.result.is_some()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        use WeeklyTimeoutView as V;
        self.request.validate()?;
        if (self.menu_seen && self.open.is_none())
            || (self.leave.is_some() && !self.menu_seen)
            || (self.confirm_seen && self.leave.is_none())
            || (self.confirm.is_some() && !self.confirm_seen)
            || (self.exit_seen && self.confirm.is_none())
            || self.stable > 2
            || (self.candidate.is_some() != (self.stable > 0))
            || self.candidate.is_some_and(|v| match v {
                V::Hud => self.open.is_some(),
                V::Menu => !self.menu_seen || self.leave.is_some(),
                V::ConfirmExit => !self.confirm_seen || self.confirm.is_some(),
                V::Exit => !self.exit_seen,
                V::Retreat => false,
                _ => true,
            })
            || self.result
                != if self.stable == 2 {
                    match self.candidate {
                        Some(V::Retreat) => Some(WeeklyTimeoutResult::Retreat),
                        Some(V::Exit) => Some(WeeklyTimeoutResult::Exit),
                        _ => None,
                    }
                } else {
                    None
                }
        {
            return Err("周常超时退出的意图、步骤或返回证明无效");
        }
        Ok(())
    }
    fn reset_observation(&mut self) {
        self.candidate = None;
        self.stable = 0;
        self.result = None;
    }
    fn observe(&mut self, v: WeeklyTimeoutView) -> bool {
        self.stable = if self.candidate == Some(v) {
            (self.stable + 1).min(2)
        } else {
            1
        };
        self.candidate = Some(v);
        self.stable == 2
    }
    pub fn next(
        &mut self,
        view: WeeklyTimeoutView,
    ) -> Result<Option<WeeklyTimeoutAction>, &'static str> {
        use WeeklyTimeoutAction as A;
        use WeeklyTimeoutView as V;
        self.validate()?;
        if self.completed() {
            return Ok(None);
        }
        let action = match view {
            V::Unknown | V::Blocked => {
                self.reset_observation();
                return Ok(None);
            }
            V::Other => {
                self.reset_observation();
                return Err("周常超时退出进入无关页面");
            }
            V::Retreat => {
                if self.observe(view) {
                    self.result = Some(WeeklyTimeoutResult::Retreat);
                }
                return Ok(None);
            }
            V::Exit if self.confirm.is_some() => {
                self.exit_seen = true;
                if self.observe(view) {
                    self.result = Some(WeeklyTimeoutResult::Exit);
                }
                return Ok(None);
            }
            V::Hud if self.open.is_none() => A::OpenMenu,
            V::Hud if self.leave.is_none() => {
                self.reset_observation();
                return Ok(None);
            }
            V::Menu if self.open.is_some() && self.leave.is_none() => {
                self.menu_seen = true;
                A::Leave
            }
            V::Menu if self.leave.is_some() && self.confirm.is_none() => {
                self.reset_observation();
                return Ok(None);
            }
            V::ConfirmExit if self.leave.is_some() && self.confirm.is_none() => {
                self.confirm_seen = true;
                A::Confirm
            }
            V::ConfirmExit if self.confirm.is_some() => {
                self.reset_observation();
                return Ok(None);
            }
            _ => {
                self.reset_observation();
                return Err("周常超时退出没有本次菜单、退出或确认意图");
            }
        };
        Ok(self.observe(view).then_some(action))
    }
    fn view(action: WeeklyTimeoutAction) -> WeeklyTimeoutView {
        match action {
            WeeklyTimeoutAction::OpenMenu => WeeklyTimeoutView::Hud,
            WeeklyTimeoutAction::Leave => WeeklyTimeoutView::Menu,
            WeeklyTimeoutAction::Confirm => WeeklyTimeoutView::ConfirmExit,
        }
    }
    fn delivery(&mut self, action: WeeklyTimeoutAction) -> &mut Option<Delivery> {
        match action {
            WeeklyTimeoutAction::OpenMenu => &mut self.open,
            WeeklyTimeoutAction::Leave => &mut self.leave,
            WeeklyTimeoutAction::Confirm => &mut self.confirm,
        }
    }
    pub fn prepare(&mut self, action: WeeklyTimeoutAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.completed()
            || self.stable != 2
            || self.candidate != Some(Self::view(action))
            || self.delivery(action).is_some()
        {
            return Err("周常超时退出缺少本次连续动作证明");
        }
        *self.delivery(action) = Some(Delivery::Intent);
        self.reset_observation();
        Ok(())
    }
    pub fn allows(&self, action: WeeklyTimeoutAction, view: WeeklyTimeoutView) -> bool {
        use WeeklyTimeoutAction as A;
        self.validate().is_ok()
            && !self.completed()
            && Self::view(action) == view
            && match action {
                A::OpenMenu => self.open == Some(Delivery::Intent) && !self.menu_seen,
                A::Leave => self.leave == Some(Delivery::Intent) && !self.confirm_seen,
                A::Confirm => self.confirm == Some(Delivery::Intent) && !self.exit_seen,
            }
    }
    pub fn posted(&mut self, action: WeeklyTimeoutAction) -> Result<(), &'static str> {
        if *self.delivery(action) != Some(Delivery::Intent) {
            return Err("周常超时退出回执不匹配");
        }
        *self.delivery(action) = Some(Delivery::Posted);
        Ok(())
    }
    pub fn not_posted(&mut self, action: WeeklyTimeoutAction) -> Result<(), &'static str> {
        use WeeklyTimeoutAction as A;
        if *self.delivery(action) != Some(Delivery::Intent)
            || self.candidate.is_some()
            || match action {
                A::OpenMenu => self.menu_seen,
                A::Leave => self.confirm_seen,
                A::Confirm => self.exit_seen,
            }
        {
            return Err("已投递或已推进的周常超时退出不能撤销");
        }
        *self.delivery(action) = None;
        self.reset_observation();
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.reset_observation();
        Ok(())
    }
}

/// 文字读取原图，控件亮色另行核验，避免颜色过滤裁掉抗锯齿笔画。
pub fn pipeline() -> serde_json::Value {
    use serde_json::json;
    let mut p = json!({});
    for (name, roi) in [
        ("Tabs", [1420, 12, 430, 80]),
        ("Leave", [1616, 1006, 216, 46]),
        ("Question", [550, 350, 820, 240]),
        ("Confirm", [1038, 600, 186, 54]),
        ("Cancel", [710, 584, 240, 100]),
    ] {
        p[name] = json!({"recognition":"OcrText", "model":"ppocrv5", "threshold":0.6, "roi":roi});
    }
    p["Enabled"] = json!({"recognition":"ColorMatch","roi":[0,0,1,1],"method":4,"lower":[200,200,200],"upper":[255,255,255],"count":20,"connected":false});
    p
}
