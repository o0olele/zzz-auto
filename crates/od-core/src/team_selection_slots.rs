//! 按槽位选队的按钮和身份回执；复用原顺序选择与完整复核。
use crate::{
    intel_board::valid_rect,
    team_selection::{SelectionAction, SelectionCard, SelectionPhase, TeamSelection},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SlotMarker {
    Unselected,
    Selected { order: Option<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SlotIdentity {
    pub index: u8,
    pub name: Option<String>,
    pub members: u8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotSelectionCard {
    pub identity: SlotIdentity,
    pub marker: SlotMarker,
    pub button: [i32; 4],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SlotToggle {
    pub index: u8,
    pub order: Option<u8>,
    pub button: [i32; 4],
}

pub fn valid_identity(card: &SlotIdentity) -> bool {
    card.index < 20
        && card.members <= 3
        && card.name.as_ref().is_none_or(|name| {
            !name.is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control)
        })
}

pub fn sources() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../resources/daily/team-selection-source-lock.json"
    ))
    .expect("固定选队来源")
}

impl TeamSelection {
    pub fn next_slots(
        &mut self,
        cards: &[SlotSelectionCard],
        confirm_enabled: bool,
    ) -> Result<Option<SelectionAction>, &'static str> {
        let mut next = self.clone();
        let action = next.next_slots_inner(cards, confirm_enabled)?;
        *self = next;
        Ok(action)
    }
    fn next_slots_inner(
        &mut self,
        cards: &[SlotSelectionCard],
        confirm_enabled: bool,
    ) -> Result<Option<SelectionAction>, &'static str> {
        self.validate()?;
        // 回顶只依赖调用方已核对选择页；当前页可能不足六卡，不能先猜其下标。
        if self.top_scrolls < 4
            && matches!(
                self.phase,
                SelectionPhase::ResetClear
                    | SelectionPhase::ResetChoose
                    | SelectionPhase::ResetVerify
            )
        {
            if self.pending.is_some() {
                return Ok(None);
            }
            self.pending = Some(SelectionAction::Up);
            return Ok(Some(SelectionAction::Up));
        }
        let expected: std::collections::BTreeSet<_> = (0..6)
            .filter_map(|cell| crate::team_slots::index(self.page, cell))
            .collect();
        if cards.len() != expected.len()
            || cards
                .iter()
                .map(|c| c.identity.index)
                .collect::<std::collections::BTreeSet<_>>()
                != expected || cards.iter().any(|c| {
            !valid_identity(&c.identity)
                || !valid_rect(c.button)
                || !button_in_slot(self.page, c.identity.index, c.button)
                || matches!(c.marker,SlotMarker::Selected{order:Some(n)} if !(1..=2).contains(&n))
                || (c.identity.members == 0 && c.marker != SlotMarker::Unselected)
        }) {
            return Err("选队槽位、人数或按钮回执无效");
        }
        if let Some(SelectionAction::Toggle { index, order }) = self.pending {
            let receipt = self.slot_toggle.as_ref().ok_or("槽位切换缺少原按钮回执")?;
            if receipt.index != index || receipt.order != order {
                return Err("槽位切换回执与待输入不符");
            }
            let card = cards
                .iter()
                .find(|c| c.identity.index == index)
                .ok_or("原选队按钮不在当前页")?;
            let [x, y, w, h] = card.button;
            let [px, py, pw, ph] = receipt.button;
            if (x + w / 2 - px - pw / 2).abs() > 160 || (y + h / 2 - py - ph / 2).abs() > 120 {
                return Ok(None);
            }
        }
        // 仅在完成回顶后收集/核对身份，回顶中的页号不作为画面位置证明。
        if self.top_scrolls == 4
            && !matches!(
                self.phase,
                SelectionPhase::ResetClear
                    | SelectionPhase::ResetChoose
                    | SelectionPhase::ResetVerify
            )
        {
            self.remember_slot_cards(cards)?;
        }
        let mut legacy = Vec::new();
        for card in cards {
            let index = card.identity.index;
            let order = match card.marker {
                SlotMarker::Unselected => None,
                SlotMarker::Selected { order: Some(order) } => Some(order),
                SlotMarker::Selected { order: None }
                    if matches!(
                        self.phase,
                        SelectionPhase::ResetClear | SelectionPhase::Clear
                    ) =>
                {
                    Some(1)
                }
                SlotMarker::Selected { order: None } => {
                    let known = self
                        .chosen
                        .iter()
                        .position(|id| *id == index)
                        .map(|n| n as u8 + 1)
                        .or(match self.pending {
                            Some(SelectionAction::Toggle {
                                index: id,
                                order: Some(n),
                            }) if id == index => Some(n),
                            _ => None,
                        });
                    Some(known.ok_or("没有本次顺序投递证明的额外选中编队")?)
                }
            };
            legacy.push(SelectionCard {
                index,
                members: card.identity.members,
                order,
                name_rect: card.button,
                recognized_name: card
                    .identity
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("槽位{}", index + 1)),
            });
        }
        let action = self.next_inner(&legacy, confirm_enabled)?;
        if self.top_scrolls == 4
            && matches!(
                self.phase,
                SelectionPhase::Clear
                    | SelectionPhase::Choose
                    | SelectionPhase::Verify
                    | SelectionPhase::Ready
            )
        {
            self.remember_slot_cards(cards)?;
        }
        if let Some(SelectionAction::Toggle { index, order }) = action {
            let card = cards
                .iter()
                .find(|c| c.identity.index == index)
                .ok_or("目标槽位缺少实际按钮")?;
            self.slot_toggle = Some(SlotToggle {
                index,
                order,
                button: card.button,
            });
        } else if !matches!(self.pending, Some(SelectionAction::Toggle { .. })) {
            self.slot_toggle = None;
        }
        self.validate()?;
        Ok(action)
    }
    fn remember_slot_cards(&mut self, cards: &[SlotSelectionCard]) -> Result<(), &'static str> {
        for card in cards {
            if let Some(previous) = self
                .slot_cards
                .iter_mut()
                .find(|p| p.index == card.identity.index)
            {
                if previous.members != card.identity.members
                    || matches!((&previous.name,&card.identity.name),(Some(a),Some(b)) if a!=b)
                {
                    return Err("编队重叠页或复核身份变化");
                }
                if previous.name.is_none() {
                    previous.name = card.identity.name.clone();
                }
            } else {
                if !matches!(self.phase, SelectionPhase::Clear) {
                    return Err("选队缺少清除阶段的槽位身份");
                }
                self.slot_cards.push(card.identity.clone());
            }
        }
        self.slot_cards.sort_by_key(|c| c.index);
        Ok(())
    }
}

pub fn button_in_slot(page: u8, index: u8, [x, y, w, h]: [i32; 4]) -> bool {
    let Some(cell) = index
        .checked_sub(page.saturating_mul(4))
        .map(usize::from)
        .filter(|cell| *cell < 6)
    else {
        return false;
    };
    let [left, top, width, height] = crate::team_slots::CARDS[cell];
    x >= left
        && x + w <= left + width
        && y >= top
        && y + h <= top + height
        && (y + h / 2 - top - height / 2).abs() <= 85
}
