use anyhow::{Context, Result};
use od_core::{
    predefined_teams,
    team_selection_slots::{SlotIdentity, SlotMarker, SlotSelectionCard},
    team_slots,
};
use serde_json::Value;

fn inside([x, y, w, h]: [i32; 4], [l, t, r, b]: [i32; 4]) -> bool {
    x >= l && y >= t && x + w <= r && y + h <= b
}
fn centered([x, y, w, h]: [i32; 4], [l, t, r, b]: [i32; 4]) -> bool {
    (l..r).contains(&(x + w / 2)) && (t..b).contains(&(y + h / 2))
}
fn label(text: &str) -> Option<SlotMarker> {
    let text = predefined_teams::normalize_name(text).to_uppercase();
    match text.trim_start_matches(['+', '＋']) {
        "SELECT" => Some(SlotMarker::Unselected),
        "TEAM" | "SELECTED" => Some(SlotMarker::Selected { order: None }),
        "TEAM01" | "TEAM1" => Some(SlotMarker::Selected { order: Some(1) }),
        "TEAM02" | "TEAM2" => Some(SlotMarker::Selected { order: Some(2) }),
        _ => None,
    }
}

pub(super) fn read_cards(
    page: u8,
    readings: &[Value],
) -> Result<(Vec<SlotSelectionCard>, Vec<u8>)> {
    let texts: Vec<(String, [i32; 4])> = readings
        .iter()
        .map(|r| {
            Ok((
                r["text"].as_str().context("选择文字无效")?.into(),
                serde_json::from_value(r["box"].clone())?,
            ))
        })
        .collect::<Result<_>>()?;
    let mut cards = Vec::new();
    let mut unreadable = Vec::new();
    for (cell, bounds @ [left, top, width, height]) in team_slots::CARDS.into_iter().enumerate() {
        let Some(index) = team_slots::index(page, cell) else {
            continue;
        };
        let counts: std::collections::BTreeSet<_> = texts
            .iter()
            .filter(|(_, rect)| {
                centered(
                    *rect,
                    [left + width - 180, top, left + width - 45, top + 70],
                )
            })
            .filter_map(|(t, _)| predefined_teams::member_count(t))
            .collect();
        let Some(&members) = counts.first().filter(|_| counts.len() == 1) else {
            unreadable.push(index);
            continue;
        };
        let column = if cell % 2 == 0 {
            [774, 180, 940, 941]
        } else {
            [1598, 180, 1757, 941]
        };
        let center = top + height / 2;
        let mut buttons: Vec<_> = texts
            .iter()
            .filter(|(_, rect @ [_, y, _, h])| {
                inside(*rect, column) && (y + h / 2 - center).abs() <= 85
            })
            .filter_map(|(text, rect)| label(text).map(|marker| (*rect, marker)))
            .collect();
        buttons.sort_by_key(|([_, y, _, h], _)| (y + h / 2 - center).abs());
        let Some(&(button, mut marker)) = buttons.first() else {
            unreadable.push(index);
            continue;
        };
        if buttons.iter().any(|(_, other)| {
            matches!(
                (marker, *other),
                (SlotMarker::Unselected, SlotMarker::Selected { .. })
                    | (SlotMarker::Selected { .. }, SlotMarker::Unselected)
            )
        }) {
            unreadable.push(index);
            continue;
        }
        if let SlotMarker::Selected { order } = marker {
            let [bx, by, bw, bh] = button;
            let mut orders: std::collections::BTreeSet<_> = buttons
                .iter()
                .filter_map(|(_, m)| match m {
                    SlotMarker::Selected { order } => *order,
                    _ => None,
                })
                .collect();
            if let Some(order) = order {
                orders.insert(order);
            }
            let numbers: Vec<_> = texts
                .iter()
                .filter(|(_, rect @ [x, y, w, h])| {
                    inside(*rect, column)
                        && (x + w / 2 - bx - bw / 2).abs() <= 100
                        && (y + h / 2 - by - bh / 2).abs() <= 75
                })
                .filter_map(|(text, _)| {
                    let normalized = predefined_teams::normalize_name(text);
                    (!normalized.is_empty()
                        && normalized.len() <= 2
                        && normalized.chars().all(|c| c.is_ascii_digit()))
                    .then_some(normalized)
                })
                .collect();
            let mut invalid = false;
            for number in numbers {
                match number.as_str() {
                    "01" | "1" => {
                        orders.insert(1);
                    }
                    "02" | "2" => {
                        orders.insert(2);
                    }
                    _ => invalid = true,
                }
            }
            if invalid || orders.len() > 1 {
                unreadable.push(index);
                continue;
            }
            marker = SlotMarker::Selected {
                order: orders.first().copied(),
            };
        }
        cards.push(SlotSelectionCard {
            identity: SlotIdentity {
                index,
                name: team_slots::name(bounds, &texts),
                members,
            },
            marker,
            button,
        });
    }
    Ok((cards, unreadable))
}
