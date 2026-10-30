//! 快捷手册资源栏 → 电池详情 → 合成页。只证明导航，不消费或登记次数。
use crate::ether_synthesis::{SynthesisRequest, SynthesisStock};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisEntryView {
    Unknown,
    Blocked,
    Handbook { battery: bool },
    BatteryDetail { recipe: bool },
    Page { stock: SynthesisStock },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisEntryAction {
    OpenBattery,
    SelectRecipe,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Receipt {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesisEntryFlow {
    request: SynthesisRequest,
    opened: Option<Receipt>,
    selected: Option<Receipt>,
    ready: Option<SynthesisStock>,
}
impl SynthesisEntryFlow {
    pub fn new(request: SynthesisRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            opened: None,
            selected: None,
            ready: None,
        })
    }
    pub fn request(&self) -> &SynthesisRequest {
        &self.request
    }
    pub fn ready(&self) -> Option<SynthesisStock> {
        self.ready
    }
    pub fn awaiting_page(&self) -> bool {
        self.selected.is_some() && self.ready.is_none()
    }
    pub fn opening_started(&self) -> bool {
        self.opened.is_some()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if let Some(stock) = self.ready {
            stock.validate()?;
        }
        if (self.selected.is_some() && self.opened.is_none())
            || (self.ready.is_some() && self.selected.is_none())
        {
            return Err("合成导航检查点缺少本次入口意图");
        }
        Ok(())
    }
    pub fn next(
        &mut self,
        view: SynthesisEntryView,
    ) -> Result<Option<SynthesisEntryAction>, &'static str> {
        self.validate()?;
        if self.ready.is_some() {
            return Ok(None);
        }
        match view {
            SynthesisEntryView::Unknown => Ok(None),
            SynthesisEntryView::Blocked => Err("电池合成入口出现无法确认身份的画面"),
            SynthesisEntryView::Handbook { battery: true } if self.opened.is_none() => {
                Ok(Some(SynthesisEntryAction::OpenBattery))
            }
            SynthesisEntryView::Handbook { .. } => Ok(None),
            SynthesisEntryView::BatteryDetail { .. } if self.opened.is_none() => {
                Err("不能采用未从本次资源栏打开的电池详情")
            }
            SynthesisEntryView::BatteryDetail { recipe: true } if self.selected.is_none() => {
                Ok(Some(SynthesisEntryAction::SelectRecipe))
            }
            SynthesisEntryView::BatteryDetail { .. } => Ok(None),
            SynthesisEntryView::Page { .. } if self.selected.is_none() => {
                Err("未经过本次合成入口，不能交接消费流程")
            }
            SynthesisEntryView::Page { stock } => {
                stock.validate()?;
                self.ready = Some(stock);
                Ok(None)
            }
        }
    }
    fn receipt(&self, action: SynthesisEntryAction) -> Option<Receipt> {
        match action {
            SynthesisEntryAction::OpenBattery => self.opened,
            SynthesisEntryAction::SelectRecipe => self.selected,
        }
    }
    fn set(&mut self, action: SynthesisEntryAction, value: Option<Receipt>) {
        match action {
            SynthesisEntryAction::OpenBattery => self.opened = value,
            SynthesisEntryAction::SelectRecipe => self.selected = value,
        }
    }
    pub fn prepare(&mut self, action: SynthesisEntryAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.ready.is_some()
            || self.receipt(action).is_some()
            || (action == SynthesisEntryAction::SelectRecipe && self.opened.is_none())
        {
            return Err("合成导航没有可保存的新动作");
        }
        self.set(action, Some(Receipt::Intent));
        Ok(())
    }
    pub fn allows(&self, action: SynthesisEntryAction, view: SynthesisEntryView) -> bool {
        self.validate().is_ok()
            && self.ready.is_none()
            && self.receipt(action) == Some(Receipt::Intent)
            && matches!(
                (action, view),
                (
                    SynthesisEntryAction::OpenBattery,
                    SynthesisEntryView::Handbook { battery: true }
                ) | (
                    SynthesisEntryAction::SelectRecipe,
                    SynthesisEntryView::BatteryDetail { recipe: true }
                )
            )
    }
    pub fn posted(&mut self, action: SynthesisEntryAction) -> Result<(), &'static str> {
        if self.receipt(action) != Some(Receipt::Intent) {
            return Err("合成导航没有保存动作意图");
        }
        self.set(action, Some(Receipt::Posted));
        Ok(())
    }
    pub fn not_posted(&mut self, action: SynthesisEntryAction) -> Result<(), &'static str> {
        if self.receipt(action) != Some(Receipt::Intent) {
            return Err("已投递合成导航动作不能撤销");
        }
        self.set(action, None);
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.ready = None;
        Ok(())
    }
}
