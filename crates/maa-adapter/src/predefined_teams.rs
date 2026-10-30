use crate::{NativeSession, RecognitionImage};
use anyhow::{Context, Result, ensure};
use od_core::predefined_teams::{self, AvatarMatch, TeamConfig, TeamMembers};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamPageMode {
    Unknown,
    Editing,
    Selection,
}

#[derive(Debug)]
pub struct TeamPage {
    pub mode: TeamPageMode,
    pub members: Vec<TeamMembers>,
    pub unreadable: Vec<String>,
    pub by_slot: bool,
    pub unreadable_slots: Vec<u8>,
}

impl NativeSession {
    pub fn read_team_page(
        &self,
        config: &TeamConfig,
        game: bool,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<TeamPage>> {
        self.read_team_page_at(config, 0, game, cancel, timeout)
    }

    pub fn read_team_page_at(
        &self,
        config: &TeamConfig,
        page_index: u8,
        game: bool,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<TeamPage>> {
        ensure!(
            self.game_window.is_some() == game
                && self.input_mode == Some(crate::InputMode::Foreground),
            "编队检查控制器模式不匹配"
        );
        let bundle = crate::validate_basic_daily_app_bundle(
            self.daily_bundle.as_deref().unwrap_or(&self.resource_root),
            "predefined_team_checker",
        )?;
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(bundle.join("basic-daily-manifest.json"))?)?;
        let page = if manifest["schema_version"]
            .as_u64()
            .is_some_and(|schema| schema >= 9)
        {
            self.inspect_predefined_team_slots(&frame.image, config, page_index, cancel, timeout)?
        } else {
            self.inspect_predefined_teams_with_timeout(&frame.image, config, cancel, timeout)?
        };
        Ok((!cancel.load(Ordering::Acquire)).then_some(page))
    }
    /// 只读识别，同一张固定画面上的名称、人数及头像共同构成结果。
    /// 调用方负责加载经校验的日常资源包，并在配置保存前核对运行身份及修订。
    pub fn inspect_predefined_teams(
        &self,
        frame: &RecognitionImage,
        config: &TeamConfig,
        cancel: &AtomicBool,
    ) -> Result<TeamPage> {
        self.inspect_predefined_teams_with_timeout(frame, config, cancel, Duration::from_secs(60))
    }

    pub fn inspect_predefined_teams_with_timeout(
        &self,
        frame: &RecognitionImage,
        config: &TeamConfig,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<TeamPage> {
        let started = Instant::now();
        let ready = || -> Result<bool> {
            if cancel.load(Ordering::Acquire) {
                return Ok(false);
            }
            ensure!(started.elapsed() < timeout, "编队画面识别超时");
            Ok(true)
        };
        config.validate().map_err(anyhow::Error::msg)?;
        ensure!(!self.tasker.running(), "编队识别前须等待 Pipeline 结束");
        let mut page = TeamPage {
            mode: TeamPageMode::Unknown,
            members: vec![],
            unreadable: vec![],
            by_slot: false,
            unreadable_slots: vec![],
        };
        if !ready()? {
            return Ok(page);
        }
        let ocr =
            self.recognize_prepared(frame, "OCR", &json!({"model":"ppocrv5", "threshold":0.6}))?;
        let texts = ocr.detail["detail"]["filtered"]
            .as_array()
            .context("编队 OCR 缺少结果")?;
        if !ready()? {
            return Ok(page);
        }
        let back = self.recognize_prepared(
            frame,
            "TemplateMatch",
            &json!({
                "template":"daily/back.png","roi":[54,12,115,80],"green_mask":true,"threshold":0.7
            }),
        )?;
        if !ready()? || back.detail["hit"] != true {
            return Ok(page);
        }
        // Maa 不稳定地读出灰色的 1P / BANGBOO。用卡片右上角的核心技计数
        // 和固定双列区域确认网格；头像仍须在名称对应的三个位置逐一证明。
        let counters = texts
            .iter()
            .filter(|text| {
                let x = text["box"][0].as_i64().unwrap_or(-1);
                let y = text["box"][1].as_i64().unwrap_or(-1);
                ((760..=880).contains(&x) || (1580..=1700).contains(&x))
                    && (100..=800).contains(&y)
                    && text["text"]
                        .as_str()
                        .and_then(predefined_teams::member_count)
                        .is_some()
            })
            .count();
        if counters < 2 {
            return Ok(page);
        }
        page.mode = if texts.iter().any(|t| {
            let x = t["box"][0].as_i64().unwrap_or(-1);
            let y = t["box"][1].as_i64().unwrap_or(-1);
            let text = t["text"].as_str().unwrap_or("").to_uppercase();
            ((795..=940).contains(&x) || (1610..=1755).contains(&x))
                && (215..=960).contains(&y)
                && (text.contains("SELECT") || text.starts_with("TEAM"))
                || (1570..=1880).contains(&x) && y >= 985 && text == "预备出战"
        }) {
            TeamPageMode::Selection
        } else {
            TeamPageMode::Editing
        };
        if page.mode != TeamPageMode::Editing {
            return Ok(page);
        }
        let mut names = std::collections::BTreeMap::new();
        for text in texts {
            let Some(name) = text["text"].as_str() else {
                continue;
            };
            let Some(index) = predefined_teams::match_name(config, name) else {
                continue;
            };
            let rect: [i32; 4] =
                serde_json::from_value(text["box"].clone()).context("编队名称坐标无效")?;
            let [x, y, _, _] = rect;
            if !(140..=1100).contains(&x) || !(100..=800).contains(&y) {
                continue;
            }
            if let Some(previous) = names.insert(index, rect) {
                // OCR 有重复框时只接受同一位置，不把同名异处卡片合为一队。
                if (previous[0] - x).abs() > 30 || (previous[1] - y).abs() > 30 {
                    page.unreadable
                        .push(config.get(index).map_err(anyhow::Error::msg)?.name.clone());
                }
            }
        }
        let agents = crate::basic_daily::basic_daily_agents(
            self.daily_bundle.as_deref().unwrap_or(&self.resource_root),
        )?;
        for (index, [x, y, _, _]) in names {
            if !ready()? {
                page.members.clear();
                return Ok(page);
            }
            let name = &config.get(index).map_err(anyhow::Error::msg)?.name;
            if page.unreadable.contains(name) {
                continue;
            }
            let mut counts: std::collections::BTreeSet<_> = texts
                .iter()
                .filter(|t| {
                    let tx = t["box"][0].as_i64().unwrap_or(-1);
                    let ty = t["box"][1].as_i64().unwrap_or(-1);
                    (i64::from(x + 580)..=i64::from(x + 720)).contains(&tx)
                        && (i64::from(y - 20)..=i64::from(y + 45)).contains(&ty)
                })
                .filter_map(|t| t["text"].as_str())
                .filter_map(predefined_teams::member_count)
                .collect();
            if counts.is_empty() {
                let count = self.recognize_prepared(
                    frame,
                    "OCR",
                    &json!({
                        "model":"ppocrv5", "roi":[x+590,y-8,160,65], "threshold":0.6
                    }),
                )?;
                counts = count.detail["detail"]["filtered"]
                    .as_array()
                    .context("编队人数 OCR 缺少结果")?
                    .iter()
                    .filter_map(|t| t["text"].as_str())
                    .filter_map(predefined_teams::member_count)
                    .collect();
            }
            if counts.len() != 1 {
                page.unreadable.push(name.clone());
                continue;
            }
            let count = *counts.first().unwrap();
            if let Some(agent_ids) =
                self.recognize_team_agents(frame, x, y, count, &agents, &ready)?
            {
                page.members.push(TeamMembers {
                    index,
                    name: name.clone(),
                    observed_name: None,
                    member_count: count,
                    agent_ids,
                });
            } else {
                page.unreadable.push(name.clone());
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
    pub(super) fn recognize_team_agents(
        &self,
        frame: &RecognitionImage,
        x: i32,
        y: i32,
        count: u8,
        agents: &[od_core::random_play::ShopAgent],
        ready: &impl Fn() -> Result<bool>,
    ) -> Result<Option<[String; 3]>> {
        let mut matches = Vec::new();
        for slot in 0..3 {
            // 名称框与头像边缘有数像素差异，保留余量；限定到单个头像，
            // 既避免扫描整张卡片，也不会把邦布或邻队头像算作当前代理人。
            let roi = [x - 10 + 194 * slot, y + 70, 208, 150];
            for agent in agents {
                if !ready()? {
                    return Ok(None);
                }
                // 管理卡片在固定 1080p 下与头像素材同尺寸。先用宽松的相关性
                // 筛掉无关素材，再用旧版同样的 SIFT 参数验证，避免逐队重复
                // 提取全部头像素材的特征而耗尽应用超时。
                let templates: Vec<_> = agent
                    .templates
                    .iter()
                    .map(|template| format!("random-play/avatar_{template}.png"))
                    .collect();
                let candidate = self.recognize_prepared(
                    frame,
                    "TemplateMatch",
                    &json!({
                        "template":templates,
                        "roi":roi,"green_mask":true,"threshold":0.6
                    }),
                )?;
                if candidate.detail["hit"] != true {
                    continue;
                }
                if !ready()? {
                    return Ok(None);
                }
                let detected = self.recognize_prepared(frame, "FeatureMatch", &json!({
                    "template":templates,
                    "roi":roi,"detector":"SIFT","ratio":0.5,"count":4,"green_mask":true,"order_by":"Score"
                }))?;
                if detected.detail["hit"] != true {
                    continue;
                }
                for hit in detected.detail["detail"]["filtered"]
                    .as_array()
                    .context("头像特征匹配缺少结果")?
                {
                    matches.push(AvatarMatch {
                        agent_id: agent.id.clone(),
                        rect: serde_json::from_value(hit["box"].clone())
                            .context("头像匹配框无效")?,
                        score: hit["count"].as_f64().context("头像匹配缺少特征点数量")?,
                    });
                }
            }
        }
        Ok(predefined_teams::resolve_members(x, y, count, &matches))
    }
}
