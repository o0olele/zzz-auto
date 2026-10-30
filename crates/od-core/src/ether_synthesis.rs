//! 单次以太电池合成。确认和领取各保存一次意图，数量增减与获得弹窗共同证明成功。
use crate::charge_plan::ChargePlanItem;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesisRequest {
    pub plan: ChargePlanItem,
    pub timeout_ms: u32,
}
impl SynthesisRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.plan.validate()?;
        if self.plan.category_name != "合成电池" || !(1000..=300_000).contains(&self.timeout_ms)
        {
            return Err("电池合成只接受合成电池计划及 1000..300000ms 时限");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesisStock {
    pub charge: u32,
    pub ether: u32,
}
impl SynthesisStock {
    pub fn validate(self) -> Result<(), &'static str> {
        if self.charge > 999 || self.ether >= 1_000_000 {
            return Err("合成电量或电池数量无效");
        }
        Ok(())
    }
    /// 兑换由完整确认和获得链证明，电池必须恰好增加一个。
    /// 电量可在暂停期间自然恢复至 240；超出自然上限的电量不自然增加。
    pub fn validate_after_one(self, after: Self) -> Result<(), &'static str> {
        self.validate()?;
        after.validate()?;
        let minimum = self.charge.checked_sub(60).ok_or("合成初始电量不足")?;
        if self.ether.checked_add(1) != Some(after.ether)
            || !(minimum..=minimum.max(240)).contains(&after.charge)
        {
            return Err("合成后电池数量或电量不符合单次兑换及自然恢复范围");
        }
        Ok(())
    }
    fn can_regenerate_to(self, after: Self) -> bool {
        self.ether == after.ether && (self.charge..=self.charge.max(240)).contains(&after.charge)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisView {
    Unknown,
    Blocked,
    /// 身份、合成数量为 1、所需电量为 60 均已在同帧确认。
    Page {
        stock: SynthesisStock,
        missing_materials: bool,
    },
    /// 完整确认句明确以太电池 ×1，素材为电量 60、储值电卡 1、丁尼 100。
    Confirmation,
    /// 获得标题、以太电池名称、数量 1 和确认按钮同时命中。
    Obtained,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisAction {
    Synthesize,
    Confirm,
    Collect,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Receipt {
    Intent,
    Posted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SynthesisResult {
    Succeeded {
        before: SynthesisStock,
        after: SynthesisStock,
    },
    Unavailable {
        stock: SynthesisStock,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SynthesisFlow {
    request: SynthesisRequest,
    before: Option<SynthesisStock>,
    synthesize: Option<Receipt>,
    confirm: Option<Receipt>,
    collect: Option<Receipt>,
    obtained: bool,
    result: Option<SynthesisResult>,
}
impl SynthesisFlow {
    pub fn new(request: SynthesisRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            before: None,
            synthesize: None,
            confirm: None,
            collect: None,
            obtained: false,
            result: None,
        })
    }
    pub fn request(&self) -> &SynthesisRequest {
        &self.request
    }
    pub fn result(&self) -> Option<SynthesisResult> {
        self.result
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if let Some(before) = self.before {
            before.validate()?;
        }
        if (self.synthesize.is_some() && self.before.is_none())
            || (self.confirm.is_some() && self.synthesize.is_none())
            || (self.obtained && self.confirm.is_none())
            || (self.collect.is_some() && !self.obtained)
            || (self.synthesize.is_some() && self.before.is_some_and(|b| b.charge < 60))
        {
            return Err("合成检查点缺少前置证明");
        }
        match self.result {
            Some(SynthesisResult::Succeeded { before, after })
                if self.before != Some(before)
                    || before.validate_after_one(after).is_err()
                    || !self.obtained
                    || self.collect.is_none() =>
            {
                Err("合成成功缺少获得、领取或数量变化证明")
            }
            Some(SynthesisResult::Unavailable { stock })
                if self.before != Some(stock) || self.synthesize.is_some() =>
            {
                Err("素材不足检查点与消费意图不一致")
            }
            _ => Ok(()),
        }
    }
    pub fn next(&mut self, view: SynthesisView) -> Result<Option<SynthesisAction>, &'static str> {
        self.validate()?;
        if self.result.is_some() {
            return Ok(None);
        }
        match view {
            SynthesisView::Unknown => Ok(None),
            SynthesisView::Blocked => Err("合成出现无法确认身份或数量的画面"),
            SynthesisView::Page {
                stock,
                missing_materials,
            } => {
                stock.validate()?;
                if self.synthesize.is_none() {
                    if self.before.is_some_and(|old| !old.can_regenerate_to(stock)) {
                        return Err("合成前资源变化，不能沿用旧意图");
                    }
                    self.before = Some(stock);
                    if missing_materials || stock.charge < 60 {
                        self.result = Some(SynthesisResult::Unavailable { stock });
                        return Ok(None);
                    }
                    Ok(Some(SynthesisAction::Synthesize))
                } else if self.collect.is_some() && self.obtained {
                    let before = self.before.ok_or("合成缺少初始数量")?;
                    before.validate_after_one(stock)?;
                    self.result = Some(SynthesisResult::Succeeded {
                        before,
                        after: stock,
                    });
                    Ok(None)
                } else {
                    // 弹窗消失或返回合成页不足以证明本次合成；也不能重复消费。
                    Ok(None)
                }
            }
            SynthesisView::Confirmation => {
                if self.synthesize.is_none() {
                    return Err("不能采用既有合成确认弹窗");
                }
                if self.collect.is_some() || self.obtained {
                    return Err("领取后意外返回合成确认");
                }
                Ok(self.confirm.is_none().then_some(SynthesisAction::Confirm))
            }
            SynthesisView::Obtained => {
                if self.confirm.is_none() {
                    return Err("不能采用既有获得弹窗");
                }
                self.obtained = true;
                Ok(self.collect.is_none().then_some(SynthesisAction::Collect))
            }
        }
    }
    fn receipt(&self, action: SynthesisAction) -> Option<Receipt> {
        match action {
            SynthesisAction::Synthesize => self.synthesize,
            SynthesisAction::Confirm => self.confirm,
            SynthesisAction::Collect => self.collect,
        }
    }
    fn set(&mut self, action: SynthesisAction, receipt: Option<Receipt>) {
        match action {
            SynthesisAction::Synthesize => self.synthesize = receipt,
            SynthesisAction::Confirm => self.confirm = receipt,
            SynthesisAction::Collect => self.collect = receipt,
        }
    }
    pub fn prepare(&mut self, action: SynthesisAction) -> Result<(), &'static str> {
        self.validate()?;
        let allowed = match action {
            SynthesisAction::Synthesize => self.before.is_some_and(|b| b.charge >= 60),
            SynthesisAction::Confirm => self.synthesize.is_some() && !self.obtained,
            SynthesisAction::Collect => self.obtained,
        };
        if self.result.is_some() || self.receipt(action).is_some() || !allowed {
            return Err("合成动作没有可保存的新意图");
        }
        self.set(action, Some(Receipt::Intent));
        Ok(())
    }
    pub fn allows(&self, action: SynthesisAction, view: SynthesisView) -> bool {
        if self.validate().is_err()
            || self.result.is_some()
            || self.receipt(action) != Some(Receipt::Intent)
        {
            return false;
        }
        match (action, view) {
            (
                SynthesisAction::Synthesize,
                SynthesisView::Page {
                    stock,
                    missing_materials: false,
                },
            ) => self.before == Some(stock) && stock.charge >= 60,
            (SynthesisAction::Confirm, SynthesisView::Confirmation) => true,
            (SynthesisAction::Collect, SynthesisView::Obtained) => true,
            _ => false,
        }
    }
    pub fn posted(&mut self, action: SynthesisAction) -> Result<(), &'static str> {
        if self.receipt(action) != Some(Receipt::Intent) {
            return Err("合成动作没有保存意图");
        }
        self.set(action, Some(Receipt::Posted));
        Ok(())
    }
    pub fn not_posted(&mut self, action: SynthesisAction) -> Result<(), &'static str> {
        if self.receipt(action) != Some(Receipt::Intent) {
            return Err("已投递合成动作不能撤销");
        }
        self.set(action, None);
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.result = None;
        Ok(())
    }
}
