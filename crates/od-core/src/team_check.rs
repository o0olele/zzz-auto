//! 预备编队检查的冻结配置和分页证据。新扫描按槽位同步名称与成员，脚本保持。
use crate::predefined_teams::{self, TeamConfig, TeamMembers};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;

pub const SCAN_PAGES: u8 = 5;
#[path = "team_check_pipeline.rs"]
mod definition;
pub use definition::pipeline;

pub fn observe(
    progress: &mut crate::basic_daily::DailyProgress,
    node: &str,
) -> Result<(), &'static str> {
    let scan = progress.team_check.as_mut().ok_or("缺少冻结编队")?;
    scan.validate()?;
    let next = match (progress.stage.as_str(), node) {
        ("Inspect", "World") => "OpenMenu",
        ("Inspect", "Menu") => "OpenMore",
        ("Inspect", "More") => "OpenTeams",
        ("Inspect", "Grid") if scan.opened => "ReadTeams",
        ("Inspect", "Grid") => "ResetReturn",
        ("OpenMenu" | "OpenMore", "Navigated") => "Inspect",
        ("OpenTeams", "Opened") => {
            scan.opened = true;
            "ResetTop"
        }
        ("ResetTop", "TopReset") if scan.top_scrolls < 4 => {
            scan.top_scrolls += 1;
            if scan.top_scrolls == 4 {
                "ReadTeams"
            } else {
                "ResetTop"
            }
        }
        ("Scroll", "Scrolled") => "ReadTeams",
        ("ReadTeams", "Read") if scan.pages_read == SCAN_PAGES => {
            progress.completion_proven = scan.scan_completed();
            "Return"
        }
        ("ReadTeams", "Read") if scan.pages_read > 0 => "Scroll",
        ("ResetReturn", "World") => {
            scan.restart()?;
            "OpenMenu"
        }
        ("Return", "World") => {
            progress.returned_world = true;
            "Save"
        }
        (_, "Selection") => return Err("预备编队检查不能在出战选择页执行"),
        _ => return Err("编队检查节点与当前步骤不符"),
    };
    progress.stage = next.into();
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TeamCheckProgress {
    pub config: TeamConfig,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub by_slot: bool,
    pub pages_read: u8,
    pub opened: bool,
    pub top_scrolls: u8,
    pub members: Vec<TeamMembers>,
    pub conflicted: Vec<u8>,
    pub unreadable: Vec<u8>,
    pub saved_revision: Option<u32>,
}

impl TeamCheckProgress {
    pub fn new(config: TeamConfig) -> Result<Self, &'static str> {
        config.validate()?;
        Ok(Self {
            config,
            by_slot: false,
            pages_read: 0,
            opened: false,
            top_scrolls: 0,
            members: vec![],
            conflicted: vec![],
            unreadable: vec![],
            saved_revision: None,
        })
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate()?;
        if (self.by_slot && self.pages_read == 0)
            || (!self.by_slot && self.members.iter().any(|m| m.observed_name.is_some()))
        {
            return Err("编队名称回执与扫描方式不符");
        }
        if self.pages_read > SCAN_PAGES
            || self.top_scrolls > 4
            || (self.pages_read > 0 && self.top_scrolls != 4)
            || (!self.opened
                && (self.top_scrolls != 0
                    || self.pages_read != 0
                    || !self.members.is_empty()
                    || !self.conflicted.is_empty()
                    || !self.unreadable.is_empty()))
            || (self.pages_read == 0
                && (!self.members.is_empty()
                    || !self.conflicted.is_empty()
                    || !self.unreadable.is_empty()))
        {
            return Err("编队分页检查点无效");
        }
        for indices in [&self.conflicted, &self.unreadable] {
            if indices.len() > 20
                || indices.iter().any(|&index| index >= 20)
                || indices.iter().collect::<BTreeSet<_>>().len() != indices.len()
            {
                return Err("编队未识别或冲突槽位无效");
            }
        }
        if self
            .members
            .iter()
            .any(|m| self.conflicted.contains(&m.index) || self.unreadable.contains(&m.index))
        {
            return Err("编队有效成员不能同时标记为未识别或冲突");
        }
        if self.by_slot {
            let expected: BTreeSet<_> = (0..(self.pages_read * 4 + 2).min(20)).collect();
            let actual: BTreeSet<_> = self
                .members
                .iter()
                .map(|m| m.index)
                .chain(self.unreadable.iter().copied())
                .collect();
            if !self.conflicted.is_empty() || actual != expected {
                return Err("编队槽位检查点缺少已读页面覆盖证明");
            }
        }
        if !self.members.is_empty() {
            predefined_teams::apply_members(&self.config, &self.members)?;
        }
        if self
            .saved_revision
            .is_some_and(|revision| revision == 0 || !self.scan_completed())
        {
            return Err("编队尚未完成扫描，不能记录配置回写");
        }
        Ok(())
    }

    pub fn scan_completed(&self) -> bool {
        self.opened
            && self.top_scrolls == 4
            && self.pages_read == SCAN_PAGES
            && !self.members.is_empty()
    }

    /// 每次读页只接受明确的管理页。调用方完成翻页后递增页号，重复页号不能
    /// 冒充翻页；固定五次读页、四次滚动，覆盖旧版二十槽位的扫描范围。
    pub fn record_page(
        &mut self,
        page: u8,
        members: Vec<TeamMembers>,
        unreadable: &[String],
    ) -> Result<(), &'static str> {
        self.validate()?;
        if self.by_slot || members.iter().any(|m| m.observed_name.is_some()) {
            return Err("旧版名称扫描不能混入槽位回执");
        }
        if !self.opened
            || self.top_scrolls != 4
            || page != self.pages_read
            || page >= SCAN_PAGES
            || self.saved_revision.is_some()
        {
            return Err("编队读页次序无效");
        }
        if !members.is_empty() {
            predefined_teams::apply_members(&self.config, &members)?;
        }
        if members
            .iter()
            .any(|member| unreadable.contains(&member.name))
        {
            return Err("同一画面的编队结果不能同时有效和不可识别");
        }
        let mut next = self.clone();
        for name in unreadable {
            let index = self
                .config
                .teams
                .iter()
                .position(|team| &team.name == name)
                .ok_or("未识别的编队不属于冻结配置")? as u8;
            if !next.members.iter().any(|member| member.index == index)
                && !next.conflicted.contains(&index)
                && !next.unreadable.contains(&index)
            {
                next.unreadable.push(index);
            }
        }
        for member in members {
            if next.conflicted.contains(&member.index) {
                continue;
            }
            next.unreadable.retain(|&index| index != member.index);
            if let Some(previous) = next.members.iter().find(|m| m.index == member.index) {
                if previous != &member {
                    next.members.retain(|m| m.index != member.index);
                    next.conflicted.push(member.index);
                }
            } else {
                next.members.push(member);
            }
        }
        next.members.sort_by_key(|m| m.index);
        next.conflicted.sort_unstable();
        next.unreadable.sort_unstable();
        next.pages_read += 1;
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// 暂停后重新从菜单打开第一页，避免把暂停期间更改过的队伍与旧页合并。
    pub fn restart(&mut self) -> Result<(), &'static str> {
        if self.saved_revision.is_some() {
            return Err("已经回写的编队不能重新扫描");
        }
        *self = Self::new(self.config.clone())?;
        Ok(())
    }

    /// 翻页后的前两槽须与上一页末两槽一致，避免未翻页或到达底部后误写下标。
    pub fn record_slot_page(
        &mut self,
        page: u8,
        members: Vec<TeamMembers>,
        unreadable: &[u8],
    ) -> Result<(), &'static str> {
        self.validate()?;
        if !self.opened
            || self.top_scrolls != 4
            || page != self.pages_read
            || page >= SCAN_PAGES
            || self.saved_revision.is_some()
            || (page > 0 && !self.by_slot)
        {
            return Err("编队槽位读页次序无效");
        }
        let expected: BTreeSet<_> = (0..6)
            .filter_map(|card| crate::team_slots::index(page, card))
            .collect();
        let supplied: Vec<_> = members
            .iter()
            .map(|m| m.index)
            .chain(unreadable.iter().copied())
            .collect();
        if supplied.len() != expected.len()
            || supplied.iter().copied().collect::<BTreeSet<_>>() != expected
        {
            return Err("编队槽位读页缺失或重复");
        }
        if !members.is_empty() {
            predefined_teams::apply_members(&self.config, &members)?;
        }
        if page > 0 {
            for index in page * 4..page * 4 + 2 {
                let old = self
                    .members
                    .iter()
                    .find(|m| m.index == index)
                    .ok_or("前页缺少重叠编队证明")?;
                if members.iter().find(|m| m.index == index) != Some(old) {
                    return Err("编队重叠卡片不一致，不能确认翻页位置");
                }
            }
        }
        let mut next = self.clone();
        next.by_slot = true;
        for member in members {
            if !next.members.iter().any(|m| m.index == member.index) {
                next.members.push(member);
            }
        }
        next.unreadable.extend_from_slice(unreadable);
        next.unreadable.sort_unstable();
        next.unreadable.dedup();
        next.members.sort_by_key(|m| m.index);
        next.pages_read += 1;
        next.validate()?;
        *self = next;
        Ok(())
    }

    pub fn merged(&self) -> Result<TeamConfig, &'static str> {
        self.validate()?;
        if !self.scan_completed() {
            return Err("编队扫描未完成或没有可靠结果");
        }
        predefined_teams::apply_members(&self.config, &self.members)
    }
}

/// 由持有运行控制权的引擎调用。配置更新仍走存储层的修订比较和备份，
/// 禁止在冻结账号、名称或成员被外部改写后覆盖新配置。
pub fn save_members(
    store: &crate::config::ConfigStore,
    instance_id: u32,
    expected_revision: u32,
    scan: &TeamCheckProgress,
) -> anyhow::Result<u32> {
    anyhow::ensure!(scan.saved_revision.is_none(), "编队已经保存");
    let settings = scan.merged().map_err(anyhow::Error::msg)?;
    let current = store.read()?;
    anyhow::ensure!(current.revision == expected_revision, "编队配置修订已变化");
    let instance = current
        .instances
        .iter()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| anyhow::anyhow!("编队账号已不存在"))?;
    anyhow::ensure!(
        instance.predefined_teams == scan.config,
        "编队冻结配置已变化"
    );
    Ok(store
        .update(
            expected_revision,
            crate::config::ConfigChange::SaveTeams {
                instance_id,
                settings,
            },
        )?
        .revision)
}
