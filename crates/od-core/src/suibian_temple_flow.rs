//! 随便观外层编排。完成每个子流程并确认返回入口与释放输入后才交给后继。
use crate::{suibian_temple::SuibianTempleConfig, suibian_temple_auto::TempleHostingProgress};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TempleOperation {
    AutoManage,
    AdventureClaimOnly,
    AdventureClaimAndDispatch,
    YumChaSin,
    AdventureDispatchOnly,
    Craft,
    Sales,
    GoodGoods,
    BooBox,
}

pub fn operations(config: &SuibianTempleConfig) -> Result<Vec<TempleOperation>, &'static str> {
    config.validate()?;
    use TempleOperation::*;
    let mut ops = if config.auto_manage_enabled {
        vec![AutoManage]
    } else {
        let mut ops = vec![if config.yum_cha_sin {
            AdventureClaimOnly
        } else {
            AdventureClaimAndDispatch
        }];
        if config.yum_cha_sin {
            ops.extend([YumChaSin, AdventureDispatchOnly]);
        }
        ops.extend([Craft, Sales]);
        ops
    };
    if config.good_goods_purchase_enabled {
        ops.push(GoodGoods);
    }
    if config.boo_box_purchase_enabled {
        ops.push(BooBox);
    }
    // 参考应用明确停用德丰大押。保留配置不意味着执行旧商店或新商店。
    Ok(ops)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TemplePhase {
    Entering,
    Ready,
    Child,
    Returning,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TempleFlow {
    #[serde(default)]
    pub boo_box: Option<crate::suibian_boo_box::TempleBooBox>,
    #[serde(default)]
    pub good_goods: Option<crate::suibian_good_goods::TempleGoodGoods>,
    #[serde(default)]
    pub sales: Option<crate::suibian_sales::TempleSales>,
    #[serde(default)]
    pub yum: Option<Box<crate::suibian_yum::TempleYum>>,
    #[serde(default)]
    pub craft: Option<crate::suibian_craft::TempleCraft>,
    #[serde(default)]
    pub adventures: Vec<crate::suibian_adventure::TempleAdventure>,
    pub config: SuibianTempleConfig,
    pub phase: TemplePhase,
    pub transport_attempted: bool,
    pub entry_verified: bool,
    pub current: Option<TempleOperation>,
    pub completed: Vec<TempleOperation>,
    pub hosting: Option<TempleHostingProgress>,
    pub returned_world: bool,
    pub inputs_released: bool,
}
impl TempleFlow {
    pub fn new(config: SuibianTempleConfig) -> Result<Self, &'static str> {
        config.validate()?;
        Ok(Self {
            boo_box: None,
            good_goods: None,
            sales: None,
            yum: None,
            craft: None,
            adventures: vec![],
            config,
            phase: TemplePhase::Entering,
            transport_attempted: false,
            entry_verified: false,
            current: None,
            completed: vec![],
            hosting: None,
            returned_world: false,
            inputs_released: true,
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let ops = operations(&self.config)?;
        let boo_expected = self.current == Some(TempleOperation::BooBox)
            || self.completed.contains(&TempleOperation::BooBox);
        if boo_expected != self.boo_box.is_some() {
            return Err("随便观缺少邦巢子流程证明");
        }
        if let Some(boo) = &self.boo_box {
            boo.validate()?;
            if boo.config != self.config
                || self.current != Some(TempleOperation::BooBox) && !boo.completed()
            {
                return Err("邦巢结果不属于本次设置或尚未返回入口");
            }
        }
        let good_expected = self.current == Some(TempleOperation::GoodGoods)
            || self.completed.contains(&TempleOperation::GoodGoods);
        if good_expected != self.good_goods.is_some() {
            return Err("随便观缺少好物铺子流程证明");
        }
        if let Some(good) = &self.good_goods {
            good.validate()?;
            if good.config != self.config
                || (self.current != Some(TempleOperation::GoodGoods) && !good.completed())
            {
                return Err("好物铺结果不属于本次设置或尚未安全返回");
            }
        }
        let sales_expected = self.current == Some(TempleOperation::Sales)
            || self.completed.contains(&TempleOperation::Sales);
        if sales_expected != self.sales.is_some() {
            return Err("随便观缺少售卖子流程证明");
        }
        if let Some(sales) = &self.sales {
            sales.validate()?;
            if sales.config != self.config
                || (self.current != Some(TempleOperation::Sales) && !sales.completed())
            {
                return Err("售卖结果不属于冻结配置或尚未返回");
            }
        }
        let yum_expected = self.current == Some(TempleOperation::YumChaSin)
            || self.completed.contains(&TempleOperation::YumChaSin);
        if yum_expected != self.yum.is_some() {
            return Err("随便观缺少饮茶仙子流程证明");
        }
        if let Some(yum) = &self.yum {
            yum.validate()?;
            if yum.config != self.config
                || yum.submit_only
                || (self.current != Some(TempleOperation::YumChaSin) && !yum.completed())
            {
                return Err("饮茶仙结果不属于本次冻结流程或尚未返回");
            }
        }

        let craft_expected = self.current == Some(TempleOperation::Craft)
            || self.completed.contains(&TempleOperation::Craft);
        if craft_expected != self.craft.is_some() {
            return Err("随便观缺少本次制造子流程证明");
        }
        if let Some(craft) = &self.craft {
            craft.validate()?;
            if craft.config != self.config
                || craft.mode != crate::suibian_craft::CraftMode::Workshop
                || craft.material.is_some()
                || (self.current != Some(TempleOperation::Craft)
                    && (!craft.completed() || !craft.returned_entry))
            {
                return Err("制造结果不属于本次随便观或尚未安全返回");
            }
        }

        let expected: Vec<_> = self
            .completed
            .iter()
            .copied()
            .chain(self.current)
            .filter_map(adventure_mode)
            .collect();
        if self.adventures.len() != expected.len() {
            return Err("随便观缺少有序游历子流程检查点");
        }
        for (i, (flow, mode)) in self.adventures.iter().zip(expected).enumerate() {
            flow.validate()?;
            let current =
                self.current.and_then(adventure_mode).is_some() && i + 1 == self.adventures.len();
            if flow.config != self.config
                || flow.mode != mode
                || (!current && !flow.completed())
                || flow.material_mission.is_some()
            {
                return Err("游历结果不属于本次随便观模式或缺少安全返回证明");
            }
        }
        if !ops.starts_with(&self.completed)
            || self.completed.len() > ops.len()
            || ((self.phase == TemplePhase::Child) != self.current.is_some())
            || self
                .current
                .is_some_and(|op| ops.get(self.completed.len()) != Some(&op))
            || (self.phase != TemplePhase::Entering && !self.entry_verified)
            || (matches!(self.phase, TemplePhase::Returning | TemplePhase::Done)
                && self.completed != ops)
            || ((self.phase == TemplePhase::Done) != self.returned_world)
            || (self.phase == TemplePhase::Done && !self.inputs_released)
            || (self.phase == TemplePhase::Entering
                && (self.entry_verified || !self.completed.is_empty()))
            || (matches!(self.phase, TemplePhase::Ready | TemplePhase::Returning)
                && !self.inputs_released)
        {
            return Err("随便观编排缺少按序子流程、入口或输入释放证明");
        }
        if let Some(hosting) = &self.hosting {
            hosting.validate()?;
            if !self.config.auto_manage_enabled
                || (self.current != Some(TempleOperation::AutoManage)
                    && !self.completed.contains(&TempleOperation::AutoManage))
                || (self.completed.contains(&TempleOperation::AutoManage) && !hosting.completed())
            {
                return Err("托管进度与冻结的随便观模式不符");
            }
        } else if self.current == Some(TempleOperation::AutoManage)
            || self.completed.contains(&TempleOperation::AutoManage)
        {
            return Err("随便观缺少托管子流程证明");
        }
        Ok(())
    }
    pub fn check_update(&self, next: &Self) -> Result<(), &'static str> {
        self.validate()?;
        next.validate()?;
        match (&self.boo_box, &next.boo_box) {
            (Some(o), Some(n)) => o.check_update(n)?,
            (Some(_), None) => return Err("不能删除邦巢检查点"),
            (None, Some(n))
                if self.phase != TemplePhase::Ready
                    || next.current != Some(TempleOperation::BooBox)
                    || *n != crate::suibian_boo_box::TempleBooBox::new(self.config.clone())? =>
            {
                return Err("邦巢须从本次冻结设置的新流程开始");
            }
            _ => {}
        }
        match (&self.good_goods, &next.good_goods) {
            (Some(o), Some(n)) => o.check_update(n)?,
            (Some(_), None) => return Err("不能删除好物铺检查点"),
            (None, Some(n))
                if self.phase != TemplePhase::Ready
                    || next.current != Some(TempleOperation::GoodGoods)
                    || *n
                        != crate::suibian_good_goods::TempleGoodGoods::new(
                            self.config.clone(),
                        )? =>
            {
                return Err("好物铺须从冻结设置的新流程开始");
            }
            _ => {}
        }
        match (&self.sales, &next.sales) {
            (Some(o), Some(n)) => o.check_update(n)?,
            (Some(_), None) => return Err("不能删除售卖检查点"),
            (None, Some(n))
                if self.phase != TemplePhase::Ready
                    || next.current != Some(TempleOperation::Sales)
                    || *n != crate::suibian_sales::TempleSales::new(self.config.clone())? =>
            {
                return Err("售卖须从冻结配置的新流程开始");
            }
            _ => {}
        }
        match (&self.yum, &next.yum) {
            (Some(o), Some(n)) => o.check_update(n)?,
            (Some(_), None) => return Err("不能删除饮茶仙检查点"),
            (None, Some(n))
                if self.phase != TemplePhase::Ready
                    || next.current != Some(TempleOperation::YumChaSin)
                    || **n != crate::suibian_yum::TempleYum::new(self.config.clone(), false)? =>
            {
                return Err("饮茶仙须从冻结配置的新子流程开始");
            }
            _ => {}
        }

        match (&self.craft, &next.craft) {
            (Some(old), Some(new)) => old.check_update(new)?,
            (Some(_), None) => return Err("不能删除制造检查点"),
            (None, Some(new))
                if self.phase != TemplePhase::Ready
                    || next.current != Some(TempleOperation::Craft)
                    || *new != crate::suibian_craft::TempleCraft::new(self.config.clone())? =>
            {
                return Err("制造须从本次冻结配置的新流程开始");
            }
            _ => {}
        }

        if next.adventures.len() < self.adventures.len()
            || next.adventures.len() > self.adventures.len() + 1
        {
            return Err("不能删除或跳过随便观游历子流程");
        }
        for (old, new) in self.adventures.iter().zip(&next.adventures) {
            old.check_update(new)?;
        }
        if next.adventures.len() > self.adventures.len() {
            let mode = next
                .current
                .and_then(adventure_mode)
                .ok_or("新增游历没有对应当前子流程")?;
            if self.phase != TemplePhase::Ready
                || next.phase != TemplePhase::Child
                || next.adventures.last()
                    != Some(&crate::suibian_adventure::TempleAdventure::new(
                        self.config.clone(),
                        mode,
                    )?)
            {
                return Err("游历须从本次冻结配置的新子流程开始");
            }
        }
        if self.config != next.config
            || !next.completed.starts_with(&self.completed)
            || next.completed.len() > self.completed.len() + 1
            || (self.transport_attempted && !next.transport_attempted)
            || (self.entry_verified && !next.entry_verified)
            || (self.returned_world && self != next)
        {
            return Err("不能改写随便观冻结设置、传送意图或完成证明");
        }
        if next.completed.len() > self.completed.len() {
            if self.phase != TemplePhase::Child
                || self.current != next.completed.last().copied()
                || next.phase != TemplePhase::Ready
                || !next.inputs_released
                || (self.current == Some(TempleOperation::AutoManage)
                    && !self
                        .hosting
                        .as_ref()
                        .is_some_and(TempleHostingProgress::completed))
            {
                return Err("新增子流程结果必须对应当前子流程及已有完成证明");
            }
        } else if self.current != next.current
            && !(self.phase == TemplePhase::Ready && next.phase == TemplePhase::Child)
        {
            return Err("不能跳过或替换随便观当前子流程");
        }
        if self.phase != next.phase
            && !matches!(
                (self.phase, next.phase),
                (TemplePhase::Entering, TemplePhase::Ready)
                    | (TemplePhase::Ready, TemplePhase::Child)
                    | (TemplePhase::Child, TemplePhase::Ready)
                    | (TemplePhase::Ready, TemplePhase::Returning)
                    | (TemplePhase::Returning, TemplePhase::Done)
            )
        {
            return Err("随便观阶段不能倒退或跳过交接");
        }
        match (&self.hosting, &next.hosting) {
            (Some(old), Some(new)) => old.check_update(new)?,
            (Some(_), None) => return Err("不能删除托管检查点"),
            (None, Some(new))
                if self.phase != TemplePhase::Ready
                    || next.current != Some(TempleOperation::AutoManage)
                    || *new != TempleHostingProgress::default() =>
            {
                return Err("托管须从新的子流程开始");
            }
            _ => {}
        }
        Ok(())
    }
    fn edit(
        &mut self,
        change: impl FnOnce(&mut Self) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let mut next = self.clone();
        change(&mut next)?;
        self.check_update(&next)?;
        *self = next;
        Ok(())
    }
    pub fn mark_transport(&mut self) -> Result<(), &'static str> {
        self.edit(|s| {
            if s.phase != TemplePhase::Entering || s.transport_attempted {
                return Err("传送只能登记一次");
            }
            s.transport_attempted = true;
            Ok(())
        })
    }
    pub fn observe_entry(&mut self, inputs_released: bool) -> Result<(), &'static str> {
        self.edit(|s| {
            if s.phase != TemplePhase::Entering || !inputs_released {
                return Err("尚未确认入口及释放输入");
            }
            s.entry_verified = true;
            s.inputs_released = true;
            s.phase = TemplePhase::Ready;
            Ok(())
        })
    }
    pub fn begin_next(&mut self) -> Result<Option<TempleOperation>, &'static str> {
        let mut next = None;
        self.edit(|s| {
            if s.phase != TemplePhase::Ready || !s.inputs_released {
                return Err("上一子流程尚未释放输入");
            }
            next = operations(&s.config)?.get(s.completed.len()).copied();
            s.current = next;
            s.phase = if next.is_some() {
                TemplePhase::Child
            } else {
                TemplePhase::Returning
            };
            if next == Some(TempleOperation::YumChaSin) {
                s.yum = Some(Box::new(crate::suibian_yum::TempleYum::new(
                    s.config.clone(),
                    false,
                )?));
            }
            if next == Some(TempleOperation::Craft) {
                s.craft = Some(crate::suibian_craft::TempleCraft::new(s.config.clone())?);
            }
            if next == Some(TempleOperation::Sales) {
                s.sales = Some(crate::suibian_sales::TempleSales::new(s.config.clone())?);
            }
            if next == Some(TempleOperation::GoodGoods) {
                s.good_goods = Some(crate::suibian_good_goods::TempleGoodGoods::new(
                    s.config.clone(),
                )?);
            }
            if next == Some(TempleOperation::BooBox) {
                s.boo_box = Some(crate::suibian_boo_box::TempleBooBox::new(s.config.clone())?);
            }
            if next == Some(TempleOperation::AutoManage) {
                s.hosting = Some(Default::default());
            }
            if let Some(mode) = next.and_then(adventure_mode) {
                s.adventures
                    .push(crate::suibian_adventure::TempleAdventure::new(
                        s.config.clone(),
                        mode,
                    )?);
            }
            Ok(())
        })?;
        Ok(next)
    }
    pub fn finish_child(
        &mut self,
        op: TempleOperation,
        entry_verified: bool,
        inputs_released: bool,
    ) -> Result<(), &'static str> {
        self.edit(|s| {
            if s.phase != TemplePhase::Child
                || s.current != Some(op)
                || !entry_verified
                || !inputs_released
            {
                return Err("子流程尚未按序完成并返回随便观入口或释放输入");
            }
            if op == TempleOperation::AutoManage
                && !s
                    .hosting
                    .as_ref()
                    .is_some_and(TempleHostingProgress::completed)
            {
                return Err("托管尚未确认重新开始及返回");
            }
            if adventure_mode(op).is_some()
                && s.adventures
                    .last()
                    .is_none_or(|flow| !flow.completed() || !flow.returned_entry)
            {
                return Err("游历尚未完成本模式的收获 / 派遣并返回入口");
            }
            if op == TempleOperation::Craft
                && s.craft
                    .as_ref()
                    .is_none_or(|f| !f.completed() || !f.returned_entry)
            {
                return Err("制造尚未完成并返回入口");
            }
            if op == TempleOperation::YumChaSin && s.yum.as_ref().is_none_or(|y| !y.completed()) {
                return Err("饮茶仙尚未完成订单处理并返回入口");
            }
            if op == TempleOperation::Sales && s.sales.as_ref().is_none_or(|v| !v.completed()) {
                return Err("售卖尚未完成并返回入口");
            }
            if op == TempleOperation::GoodGoods
                && s.good_goods.as_ref().is_none_or(|g| !g.completed())
            {
                return Err("好物铺尚未完成兑换或无工作检查并返回入口");
            }
            if op == TempleOperation::BooBox && s.boo_box.as_ref().is_none_or(|b| !b.completed()) {
                return Err("邦巢尚未完成聘用与刷新处理并返回入口");
            }
            s.completed.push(op);
            s.current = None;
            s.inputs_released = true;
            s.phase = TemplePhase::Ready;
            Ok(())
        })
    }
    pub fn finish_world(&mut self, inputs_released: bool) -> Result<(), &'static str> {
        self.edit(|s| {
            if s.phase != TemplePhase::Returning || !inputs_released {
                return Err("随便观尚未完成全部子流程并安全返回");
            }
            s.phase = TemplePhase::Done;
            s.returned_world = true;
            s.inputs_released = true;
            Ok(())
        })
    }
}
fn adventure_mode(op: TempleOperation) -> Option<crate::suibian_adventure::AdventureMode> {
    use crate::suibian_adventure::AdventureMode as M;
    match op {
        TempleOperation::AdventureClaimOnly => Some(M::ClaimOnly),
        TempleOperation::AdventureClaimAndDispatch => Some(M::ClaimAndDispatch),
        TempleOperation::AdventureDispatchOnly => Some(M::DispatchOnly),
        _ => None,
    }
}
