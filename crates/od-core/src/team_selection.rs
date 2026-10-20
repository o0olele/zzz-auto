//! 战前选择预备编队。清除原选择和全表复核都必须覆盖二十槽位。
use crate::intel_board::valid_rect;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SelectionCard {
    pub index: u8,
    pub members: u8,
    pub order: Option<u8>,
    pub name_rect: [i32; 4],
    pub recognized_name: String,
}

/// None 表示明确的未选中；无法识别或出现矛盾时返回错误。
pub fn marker(texts: &[String]) -> Result<Option<u8>, &'static str> {
    let words: Vec<_> = texts
        .iter()
        .map(|text| {
            text.chars()
                .filter(|c| !c.is_whitespace() && *c != '+')
                .collect::<String>()
                .to_uppercase()
        })
        .collect();
    let unselected = words.iter().any(|text| text == "SELECT");
    let team = words.iter().any(|text| {
        text == "TEAM" || text.starts_with("TEAM0") || text == "TEAM1" || text == "TEAM2"
    });
    let orders: BTreeSet<u8> = words
        .iter()
        .filter_map(|text| {
            let number = text.strip_prefix("TEAM").unwrap_or(text);
            match number {
                "01" | "1" => Some(1),
                "02" | "2" => Some(2),
                _ => None,
            }
        })
        .collect();
    match (unselected, team, orders.len()) {
        (true, false, 0) => Ok(None),
        (false, true, 1) => Ok(orders.first().copied()),
        _ => Err("编队选中标记缺失或冲突"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SelectionPhase {
    ResetClear,
    Clear,
    ResetChoose,
    Choose,
    ResetVerify,
    Verify,
    Ready,
    Confirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SelectionAction {
    Up,
    Down,
    Toggle { index: u8, order: Option<u8> },
    Confirm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TeamSelection {
    pub targets: Vec<u8>,
    pub phase: SelectionPhase,
    pub top_scrolls: u8,
    pub page: u8,
    pub seen: Vec<(u8, Option<u8>)>,
    pub chosen: Vec<u8>,
    pub cleared: bool,
    pub pending: Option<SelectionAction>,
}

impl TeamSelection {
    pub fn new(targets: Vec<u8>) -> Result<Self, &'static str> {
        let result = Self {
            targets,
            phase: SelectionPhase::ResetClear,
            top_scrolls: 0,
            page: 0,
            seen: vec![],
            chosen: vec![],
            cleared: false,
            pending: None,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=2).contains(&self.targets.len()) || self.targets.iter().any(|&id| id >= 20)
            || self.targets.iter().collect::<BTreeSet<_>>().len() != self.targets.len()
            || self.top_scrolls > 4 || self.page > 4 || self.seen.len() > 20
            || self.seen.iter().any(|&(index, order)| index >= 20 || order.is_some_and(|order| !(1..=2).contains(&order)))
            || self.seen.iter().map(|(id, _)| id).collect::<BTreeSet<_>>().len() != self.seen.len()
            || !self.targets.starts_with(&self.chosen)
            || self.pending.is_some_and(|p| matches!(p, SelectionAction::Toggle { index, order } if index >= 20 || order.is_some_and(|n| !(1..=2).contains(&n)))) {
            return Err("预备编队选择检查点无效");
        }
        if matches!(
            self.phase,
            SelectionPhase::Ready | SelectionPhase::Confirmed
        ) && (!self.cleared
            || self.chosen != self.targets
            || self.page != 4
            || self.top_scrolls != 4
            || !self.all_verified())
        {
            return Err("预备编队选择缺少全表或顺序证明");
        }
        if self.phase == SelectionPhase::Confirmed && self.pending.is_some() {
            return Err("预备编队已经确认，不能保留待投递输入");
        }
        let pending_valid = match self.pending {
            None => true,
            Some(SelectionAction::Up) => {
                matches!(
                    self.phase,
                    SelectionPhase::ResetClear
                        | SelectionPhase::ResetChoose
                        | SelectionPhase::ResetVerify
                ) && self.top_scrolls < 4
            }
            Some(SelectionAction::Down) => {
                matches!(
                    self.phase,
                    SelectionPhase::Clear | SelectionPhase::Choose | SelectionPhase::Verify
                ) && self.page < 4
            }
            Some(SelectionAction::Toggle { order: None, .. }) => {
                self.phase == SelectionPhase::Clear
            }
            Some(SelectionAction::Toggle {
                index,
                order: Some(order),
            }) => {
                self.phase == SelectionPhase::Choose
                    && self.targets.get(self.chosen.len()) == Some(&index)
                    && self.expected(index) == Some(order)
            }
            Some(SelectionAction::Confirm) => self.phase == SelectionPhase::Ready,
        };
        if !pending_valid {
            return Err("编队待投递输入不属于当前选择阶段");
        }
        Ok(())
    }
    fn expected(&self, index: u8) -> Option<u8> {
        self.targets
            .iter()
            .position(|&id| id == index)
            .map(|position| position as u8 + 1)
    }
    fn all_verified(&self) -> bool {
        self.seen.len() == 20
            && self
                .seen
                .iter()
                .all(|&(index, order)| self.expected(index) == order)
    }
    fn reset(&mut self, phase: SelectionPhase) {
        self.phase = phase;
        self.top_scrolls = 0;
        self.page = 0;
        self.seen.clear();
        self.pending = None;
    }
    pub fn restart(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.phase == SelectionPhase::Confirmed {
            return Err("已经确认的选队不能自动重做");
        }
        self.chosen.clear();
        self.cleared = false;
        self.reset(SelectionPhase::ResetClear);
        Ok(())
    }
    fn request(&mut self, action: SelectionAction) -> Option<SelectionAction> {
        self.pending = Some(action);
        Some(action)
    }
    /// 返回 None 表示等待新的画面；绝不在选中标记尚未改变时再次切换。
    pub fn next(
        &mut self,
        cards: &[SelectionCard],
        confirm_enabled: bool,
    ) -> Result<Option<SelectionAction>, &'static str> {
        let mut next = self.clone();
        let action = next.next_inner(cards, confirm_enabled)?;
        *self = next;
        Ok(action)
    }
    fn next_inner(
        &mut self,
        cards: &[SelectionCard],
        confirm_enabled: bool,
    ) -> Result<Option<SelectionAction>, &'static str> {
        self.validate()?;
        if cards.is_empty()
            || cards.len() > 6
            || cards.iter().any(|card| {
                card.index >= 20
                    || card.members > 3
                    || (card.order.is_some() && card.members == 0)
                    || card.order.is_some_and(|n| !(1..=2).contains(&n))
                    || !valid_rect(card.name_rect)
                    || card.recognized_name.is_empty()
                    || card.recognized_name.chars().count() > 80
            })
            || cards
                .iter()
                .map(|card| card.index)
                .collect::<BTreeSet<_>>()
                .len()
                != cards.len()
        {
            return Err("预备编队读页不完整或包含重复槽位");
        }
        if let Some(pending) = self.pending {
            match pending {
                SelectionAction::Toggle { index, order } => {
                    let Some(card) = cards
                        .iter()
                        .find(|card| card.index == index && card.order == order)
                    else {
                        return Ok(None);
                    };
                    if order.is_some() {
                        if self.phase != SelectionPhase::Choose
                            || card.members == 0
                            || self.expected(index) != order
                            || self.targets.get(self.chosen.len()) != Some(&index)
                        {
                            return Err("编队选择回执与目标顺序不符");
                        }
                        self.chosen.push(index);
                        self.reset(if self.chosen == self.targets {
                            SelectionPhase::ResetVerify
                        } else {
                            SelectionPhase::ResetChoose
                        });
                    } else {
                        self.pending = None;
                    }
                }
                _ => return Ok(None),
            }
        }
        loop {
            match self.phase {
                SelectionPhase::ResetClear
                | SelectionPhase::ResetChoose
                | SelectionPhase::ResetVerify => {
                    if self.top_scrolls < 4 {
                        return Ok(self.request(SelectionAction::Up));
                    }
                    self.phase = match self.phase {
                        SelectionPhase::ResetClear => SelectionPhase::Clear,
                        SelectionPhase::ResetChoose => SelectionPhase::Choose,
                        _ => SelectionPhase::Verify,
                    };
                }
                SelectionPhase::Clear => {
                    if let Some(card) = cards.iter().find(|card| card.order.is_some()) {
                        return Ok(self.request(SelectionAction::Toggle {
                            index: card.index,
                            order: None,
                        }));
                    }
                    self.record(cards)?;
                    if self.page < 4 {
                        return Ok(self.request(SelectionAction::Down));
                    }
                    if self.seen.len() != 20 {
                        return Err("清除选择未覆盖二十个编队，不能继续");
                    }
                    self.cleared = true;
                    self.reset(SelectionPhase::ResetChoose);
                }
                SelectionPhase::Choose => {
                    if !self.cleared {
                        return Err("选择前尚未清除原编队");
                    }
                    for card in cards.iter().filter(|card| card.order.is_some()) {
                        if self
                            .chosen
                            .iter()
                            .position(|&id| id == card.index)
                            .map(|i| i as u8 + 1)
                            != card.order
                        {
                            return Err("选择过程中出现额外或顺序不符的编队");
                        }
                    }
                    let target = *self
                        .targets
                        .get(self.chosen.len())
                        .ok_or("没有待选择的目标编队")?;
                    if let Some(card) = cards.iter().find(|card| card.index == target) {
                        if card.members == 0 {
                            return Err("不能选择没有代理人的预备编队");
                        }
                        return Ok(self.request(SelectionAction::Toggle {
                            index: target,
                            order: self.expected(target),
                        }));
                    }
                    if self.page == 4 {
                        return Err("完整翻页仍未找到目标编队");
                    }
                    return Ok(self.request(SelectionAction::Down));
                }
                SelectionPhase::Verify => {
                    if cards
                        .iter()
                        .any(|card| self.expected(card.index) != card.order)
                    {
                        return Err("预备编队复核发现额外选择、目标缺失或顺序变化");
                    }
                    self.record(cards)?;
                    if self.page < 4 {
                        return Ok(self.request(SelectionAction::Down));
                    }
                    if !self.all_verified() {
                        return Err("选队复核未覆盖二十个槽位");
                    }
                    self.phase = SelectionPhase::Ready;
                }
                SelectionPhase::Ready => {
                    return Ok(confirm_enabled.then(|| {
                        self.pending = Some(SelectionAction::Confirm);
                        SelectionAction::Confirm
                    }));
                }
                SelectionPhase::Confirmed => return Ok(None),
            }
        }
    }
    fn record(&mut self, cards: &[SelectionCard]) -> Result<(), &'static str> {
        for card in cards {
            if let Some((_, previous)) = self.seen.iter().find(|(index, _)| *index == card.index) {
                if previous != &card.order {
                    return Err("编队重叠页选中状态冲突");
                }
            } else {
                self.seen.push((card.index, card.order));
            }
        }
        Ok(())
    }
    pub fn acknowledge_scroll(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        match self.pending {
            Some(SelectionAction::Up) if self.top_scrolls < 4 => self.top_scrolls += 1,
            Some(SelectionAction::Down) if self.page < 4 => self.page += 1,
            _ => return Err("没有可确认的编队滚动"),
        }
        self.pending = None;
        Ok(())
    }
    pub fn observe_deployment(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.phase != SelectionPhase::Ready || self.pending != Some(SelectionAction::Confirm) {
            return Err("没有选队确认投递证明，不能将配队页当作完成");
        }
        self.pending = None;
        self.phase = SelectionPhase::Confirmed;
        self.validate()
    }
}
