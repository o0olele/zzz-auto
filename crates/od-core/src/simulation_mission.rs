//! 模拟室类型与关卡选择。投递回执不提供选中证明，父流程不能据此出战。
use crate::{
    charge_plan::{ChargePlanItem, catalog},
    simulation_cards::CardsCharge,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionRequest {
    pub mission_type: String,
    pub mission_name: String,
    pub timeout_ms: u32,
}
impl MissionRequest {
    pub fn names(&self) -> Result<Vec<&str>, &'static str> {
        if !(1000..=900_000).contains(&self.timeout_ms) || self.mission_type == "代理人方案培养"
        {
            return Err("模拟室关卡类型或超时无效；代理人方案培养尚未接入");
        }
        let mission = catalog()
            .iter()
            .find(|c| c.category_name == "实战模拟室")
            .unwrap()
            .mission_type_list
            .iter()
            .find(|m| m.mission_type_name == self.mission_type)
            .ok_or("模拟室类型不在固定目录中")?;
        let names: Vec<_> = mission
            .mission_list
            .iter()
            .map(|m| m.mission_name.as_str())
            .collect();
        if !names.contains(&self.mission_name.as_str()) {
            return Err("关卡不属于本次冻结的模拟室类型");
        }
        Ok(names)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.names().map(|_| ())
    }
    pub fn from_plan(plan: &ChargePlanItem, timeout_ms: u32) -> Result<Self, &'static str> {
        plan.validate()?;
        if plan.category_name != "实战模拟室" {
            return Err("此选择器仅支持实战模拟室");
        }
        let request = Self {
            mission_type: plan.mission_type_name.clone(),
            mission_name: plan.mission_name.clone().ok_or("缺少关卡名称")?,
            timeout_ms,
        };
        request.validate()?;
        Ok(request)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionLabel {
    pub text: String,
    pub rect: [i32; 4],
}
impl MissionLabel {
    pub fn validate(&self) -> Result<(), &'static str> {
        let [x, y, w, h] = self.rect;
        if self.text.is_empty()
            || self.text.chars().count() > 128
            || x < 128
            || y < 724
            || w <= 0
            || h <= 0
            || x.checked_add(w).is_none_or(|r| r > 1796)
            || y.checked_add(h).is_none_or(|b| b > 882)
        {
            return Err("关卡文字框超出固定列表区域");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionView {
    Unknown,
    Blocked,
    Categories {
        names: Vec<String>,
    },
    Challenge {
        labels: Vec<MissionLabel>,
        charge: CardsCharge,
        back: bool,
    },
    Other,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionAction {
    Back,
    ChooseType,
    PanLeft(u8),
    PanRight(u8),
    ChooseMission,
}

/// 严格完整名称与目录顺序，拒绝命破/击破等近似词和混入的其他类型。
pub fn locate(
    request: &MissionRequest,
    labels: &[MissionLabel],
) -> Result<Option<MissionAction>, &'static str> {
    let names = request.names()?;
    if labels.len() > 32 {
        return Err("关卡列表文字过多");
    }
    let all_names: BTreeSet<_> = catalog()
        .iter()
        .find(|c| c.category_name == "实战模拟室")
        .unwrap()
        .mission_type_list
        .iter()
        .flat_map(|t| t.mission_list.iter().map(|m| m.mission_name.as_str()))
        .collect();
    let mut visible = Vec::new();
    for label in labels {
        label.validate()?;
        if let Some(index) = names.iter().position(|n| *n == label.text) {
            visible.push((index, label));
        } else if all_names.contains(label.text.as_str()) {
            return Err("可见关卡属于其他模拟室类型");
        }
    }
    visible.sort_by_key(|(_, l)| l.rect[0]);
    if visible
        .windows(2)
        .any(|p| p[0].0 >= p[1].0 || p[0].1.rect[0] + p[0].1.rect[2] > p[1].1.rect[0])
    {
        return Err("关卡名称重复、重叠或顺序不一致");
    }
    let target = names
        .iter()
        .position(|n| *n == request.mission_name)
        .unwrap();
    if let Some((_, label)) = visible.iter().find(|(i, _)| *i == target) {
        if label.rect[1] + 50 >= 882 {
            return Err("关卡点击偏移超出列表区域");
        }
        return Ok(Some(MissionAction::ChooseMission));
    }
    Ok(match (visible.first(), visible.last()) {
        (Some((first, _)), _) if target < *first => Some(MissionAction::PanRight(0)),
        (_, Some((last, _))) if target > *last => Some(MissionAction::PanLeft(0)),
        _ => None,
    })
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionReceipt {
    pub mission_type: String,
    pub mission_name: String,
    pub charge: CardsCharge,
    pub selection_verified: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Inspect,
    Back,
    Type,
    Search,
    Readback,
    Submitted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionFlow {
    request: MissionRequest,
    phase: Phase,
    pans: u8,
    pending: Option<MissionAction>,
    receipt: Option<MissionReceipt>,
}
impl MissionFlow {
    pub fn new(request: MissionRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            phase: Phase::Inspect,
            pans: 0,
            pending: None,
            receipt: None,
        })
    }
    pub fn request(&self) -> &MissionRequest {
        &self.request
    }
    pub fn submitted(&self) -> bool {
        self.phase == Phase::Submitted
    }
    pub fn receipt(&self) -> Option<&MissionReceipt> {
        self.receipt.as_ref()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.pans > 10
            || self.submitted() != self.receipt.is_some()
            || self.pending.is_some_and(|a| !self.phase_allows(a))
        {
            return Err("关卡选择检查点不一致");
        }
        if let Some(r) = &self.receipt {
            r.charge.validate()?;
            if r.selection_verified
                || r.mission_type != self.request.mission_type
                || r.mission_name != self.request.mission_name
            {
                return Err("投递回执不能替代或改写关卡选中证明");
            }
        }
        Ok(())
    }
    fn phase_allows(&self, a: MissionAction) -> bool {
        matches!(
            (self.phase, a),
            (Phase::Back, MissionAction::Back)
                | (Phase::Type, MissionAction::ChooseType)
                | (Phase::Search, MissionAction::ChooseMission)
        ) || matches!((self.phase,a),(Phase::Search,MissionAction::PanLeft(n)|MissionAction::PanRight(n)) if n == self.pans && n < 10)
    }
    fn page_allows(&self, a: MissionAction, view: &MissionView) -> bool {
        match (a, view) {
            (MissionAction::Back, MissionView::Challenge { back: true, .. }) => true,
            (MissionAction::ChooseType, MissionView::Categories { names }) => {
                names
                    .iter()
                    .filter(|n| **n == self.request.mission_type)
                    .count()
                    == 1
            }
            (_, MissionView::Challenge { labels, charge, .. }) if charge.validate().is_ok() => {
                let expected = locate(&self.request, labels).ok().flatten();
                match (a, expected) {
                    (MissionAction::ChooseMission, Some(MissionAction::ChooseMission)) => true,
                    (MissionAction::PanLeft(n), Some(MissionAction::PanLeft(_)))
                    | (MissionAction::PanRight(n), Some(MissionAction::PanRight(_))) => {
                        n == self.pans && n < 10
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }
    pub fn next(&mut self, view: &MissionView) -> Result<Option<MissionAction>, &'static str> {
        self.validate()?;
        if self.submitted() {
            return Ok(None);
        }
        if *view == MissionView::Other {
            return Err("关卡选择已离开类型或副本详情页");
        }
        if self.phase == Phase::Inspect {
            self.phase = match view {
                MissionView::Categories { .. } => Phase::Type,
                MissionView::Challenge { .. } => Phase::Back,
                _ => return Ok(None),
            };
        }
        if self.pending == Some(MissionAction::Back)
            && matches!(view, MissionView::Categories { .. })
        {
            self.pending = None;
            self.phase = Phase::Type;
        }
        if self.pending == Some(MissionAction::ChooseType)
            && matches!(view, MissionView::Challenge { .. })
        {
            self.pending = None;
            self.phase = Phase::Search;
        }
        if self.pending.is_some() {
            return Ok(None);
        }
        if self.phase == Phase::Readback {
            if let MissionView::Challenge { labels, charge, .. } = view {
                charge.validate()?;
                if locate(&self.request, labels)? == Some(MissionAction::ChooseMission) {
                    self.receipt = Some(MissionReceipt {
                        mission_type: self.request.mission_type.clone(),
                        mission_name: self.request.mission_name.clone(),
                        charge: *charge,
                        selection_verified: false,
                    });
                    self.phase = Phase::Submitted;
                }
            }
            return Ok(None);
        }
        let action = match self.phase {
            Phase::Back => Some(MissionAction::Back),
            Phase::Type => Some(MissionAction::ChooseType),
            Phase::Search => {
                if let MissionView::Challenge { labels, charge, .. } = view {
                    charge.validate()?;
                    match locate(&self.request, labels)? {
                        Some(MissionAction::PanLeft(_)) if self.pans < 10 => {
                            Some(MissionAction::PanLeft(self.pans))
                        }
                        Some(MissionAction::PanRight(_)) if self.pans < 10 => {
                            Some(MissionAction::PanRight(self.pans))
                        }
                        Some(MissionAction::PanLeft(_) | MissionAction::PanRight(_)) => {
                            return Err("关卡查找达到十次翻页上限");
                        }
                        other => other,
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        Ok(action.filter(|a| self.page_allows(*a, view)))
    }
    pub fn prepare(&mut self, a: MissionAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending.is_some() || !self.phase_allows(a) {
            return Err("关卡动作不是当前步骤");
        }
        self.pending = Some(a);
        Ok(())
    }
    pub fn allows(&self, a: MissionAction, v: &MissionView) -> bool {
        self.validate().is_ok() && self.pending == Some(a) && self.page_allows(a, v)
    }
    pub fn posted(&mut self, a: MissionAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.pending != Some(a) {
            return Err("关卡投递回执不匹配");
        }
        match a {
            MissionAction::Back | MissionAction::ChooseType => return Ok(()),
            MissionAction::ChooseMission => self.phase = Phase::Readback,
            MissionAction::PanLeft(_) | MissionAction::PanRight(_) => self.pans += 1,
        }
        self.pending = None;
        self.validate()
    }
    pub fn not_posted(&mut self, a: MissionAction) -> Result<(), &'static str> {
        if self.pending != Some(a) {
            return Err("关卡未投递回执不匹配");
        }
        self.pending = None;
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        self.phase = if self.submitted()
            || self.phase == Phase::Readback
            || self.pending == Some(MissionAction::ChooseMission)
        {
            Phase::Readback
        } else {
            Phase::Inspect
        };
        self.pending = None;
        self.receipt = None;
        self.validate()
    }
}
