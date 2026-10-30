use crate::{
    NativeSession, RecognitionImage,
    predefined_teams::{TeamPage, TeamPageMode},
};
use anyhow::{Context, Result, ensure};
use od_core::{
    predefined_teams::{self, TeamConfig, TeamMembers},
    team_slots,
};
use serde_json::json;
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

impl NativeSession {
    /// 页号只来自冻结扫描的已完成翻页，名称不参与确定编队下标。
    pub fn inspect_predefined_team_slots(
        &self,
        frame: &RecognitionImage,
        config: &TeamConfig,
        page_index: u8,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<TeamPage> {
        ensure!(page_index < 5, "编队页号超出扫描范围");
        config.validate().map_err(anyhow::Error::msg)?;
        ensure!(!self.tasker.running(), "编队识别前须等待 Pipeline 结束");
        let started = Instant::now();
        let ready = || -> Result<bool> {
            if cancel.load(Ordering::Acquire) {
                return Ok(false);
            }
            ensure!(started.elapsed() < timeout, "编队画面识别超时");
            Ok(true)
        };
        let mut page = TeamPage {
            mode: TeamPageMode::Unknown,
            members: vec![],
            unreadable: vec![],
            by_slot: true,
            unreadable_slots: vec![],
        };
        if !ready()? {
            return Ok(page);
        }
        let ocr =
            self.recognize_prepared(frame, "OCR", &json!({"model":"ppocrv5","threshold":0.6}))?;
        let texts: Vec<(String, [i32; 4])> = ocr.detail["detail"]["filtered"]
            .as_array()
            .context("编队 OCR 缺少结果")?
            .iter()
            .map(|r| {
                Ok((
                    r["text"].as_str().context("编队 OCR 文字无效")?.into(),
                    serde_json::from_value(r["box"].clone())?,
                ))
            })
            .collect::<Result<_>>()?;
        if !ready()? {
            return Ok(page);
        }
        let back = self.recognize_prepared(frame, "TemplateMatch", &json!({"template":"daily/back.png","roi":[54,12,115,80],"green_mask":true,"threshold":0.7}))?;
        if !ready()? || back.detail["hit"] != true {
            return Ok(page);
        }
        let in_rect = |[x, y, w, h]: [i32; 4], [l, t, r, b]: [i32; 4]| {
            (l..r).contains(&(x + w / 2)) && (t..b).contains(&(y + h / 2))
        };
        let counters = texts
            .iter()
            .filter(|(text, rect)| {
                (in_rect(*rect, [765, 100, 890, 800]) || in_rect(*rect, [1580, 100, 1710, 800]))
                    && predefined_teams::member_count(text).is_some()
            })
            .count();
        if counters < 2 {
            return Ok(page);
        }
        if texts.iter().any(|(text, rect)| {
            let text = predefined_teams::normalize_name(text).to_uppercase();
            ((in_rect(*rect, [774, 180, 940, 941]) || in_rect(*rect, [1598, 180, 1757, 941]))
                && (text.contains("SELECT") || text.starts_with("TEAM")))
                || (in_rect(*rect, [1570, 985, 1880, 1070]) && text == "预备出战")
        }) {
            page.mode = TeamPageMode::Selection;
            return Ok(page);
        }
        // 背景上仍可能出现卡片，中央弹窗须优先拦截。
        if texts.iter().any(|(text, rect)| {
            in_rect(*rect, [570, 290, 1310, 810])
                && matches!(
                    text.as_str(),
                    "确认" | "取消" | "保存" | "恢复电量" | "快捷使用"
                )
        }) {
            return Ok(page);
        }
        page.mode = TeamPageMode::Editing;
        let agents = crate::basic_daily::basic_daily_agents(
            self.daily_bundle.as_deref().unwrap_or(&self.resource_root),
        )?;
        for (card_index, card @ [left, top, width, _]) in team_slots::CARDS.into_iter().enumerate()
        {
            let Some(index) = team_slots::index(page_index, card_index) else {
                continue;
            };
            if !ready()? {
                page.members.clear();
                return Ok(page);
            }
            let count_area = [left + width - 180, top, left + width - 45, top + 65];
            let mut counts: BTreeSet<_> = texts
                .iter()
                .filter(|(_, rect)| in_rect(*rect, count_area))
                .filter_map(|(text, _)| predefined_teams::member_count(text))
                .collect();
            if counts.is_empty() {
                let result = self.recognize_prepared(frame,"OCR",&json!({"model":"ppocrv5","roi":[count_area[0],count_area[1],135,65],"threshold":0.6}))?;
                counts = result.detail["detail"]["filtered"]
                    .as_array()
                    .context("编队人数 OCR 缺少结果")?
                    .iter()
                    .filter_map(|r| r["text"].as_str())
                    .filter_map(predefined_teams::member_count)
                    .collect();
            }
            let Some(&count) = counts.first().filter(|_| counts.len() == 1) else {
                page.unreadable_slots.push(index);
                continue;
            };
            let observed_name = team_slots::name(card, &texts);
            let agent_ids =
                self.recognize_team_agents(frame, left + 15, top + 8, count, &agents, &ready)?;
            if let Some(agent_ids) = agent_ids {
                page.members.push(TeamMembers {
                    index,
                    name: config.get(index).map_err(anyhow::Error::msg)?.name.clone(),
                    observed_name,
                    member_count: count,
                    agent_ids,
                });
            } else {
                page.unreadable_slots.push(index);
            }
        }
        if !ready()? {
            page.members.clear();
            return Ok(page);
        }
        if !page.members.is_empty() {
            predefined_teams::apply_members(config, &page.members).map_err(anyhow::Error::msg)?;
        }
        Ok(page)
    }
}
