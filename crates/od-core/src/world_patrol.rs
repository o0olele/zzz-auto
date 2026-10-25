//! 锄大地冻结配置、已有路线和循环检查点。地图坐标沿用旧版缩放后的像素。
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PatrolPoint {
    pub x: i32,
    pub y: i32,
}
impl PatrolPoint {
    pub fn distance(self, other: Self) -> f64 {
        (f64::from(self.x) - f64::from(other.x)).hypot(f64::from(self.y) - f64::from(other.y))
    }
    pub fn heading(self, target: Self) -> f64 {
        (f64::from(self.y) - f64::from(target.y))
            .atan2(f64::from(target.x) - f64::from(self.x))
            .to_degrees()
            .rem_euclid(360.0)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PatrolIcon {
    pub template_id: String,
    pub icon_name: String,
    pub lm_pos: PatrolPoint,
    pub tp_pos: PatrolPoint,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PatrolArea {
    pub id: String,
    pub entry_id: String,
    pub name: String,
    pub parent_name: Option<String>,
    pub hollow: bool,
    pub width: u32,
    pub height: u32,
    pub icons: Vec<PatrolIcon>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PatrolRoute {
    pub id: String,
    pub area_id: String,
    pub tp_name: String,
    pub points: Vec<PatrolPoint>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PatrolListType {
    #[default]
    Whitelist,
    Blacklist,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PatrolRouteList {
    pub name: String,
    pub list_type: PatrolListType,
    pub route_items: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PatrolCatalog {
    pub areas: Vec<PatrolArea>,
    pub routes: Vec<PatrolRoute>,
    pub lists: Vec<PatrolRouteList>,
}
impl PatrolCatalog {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.routes.is_empty()
            || self.routes.len() > 1024
            || self.areas.len() > 128
            || self.lists.len() > 256
        {
            return Err("锄大地路线数量无效");
        }
        let mut ids = HashSet::new();
        for area in &self.areas {
            if !crate::config::valid_identifier(&area.id)
                || !crate::config::valid_identifier(&area.entry_id)
                || !ids.insert(&area.id)
                || !(210..=16384).contains(&area.width)
                || !(210..=16384).contains(&area.height)
                || area.icons.len() > 4096
                || !valid_name(&area.name)
            {
                return Err("锄大地区域无效");
            }
            for icon in &area.icons {
                if !crate::config::valid_identifier(&icon.template_id)
                    || !within(icon.lm_pos, area)
                    || !within(icon.tp_pos, area)
                {
                    return Err("锄大地图标坐标无效");
                }
            }
        }
        let mut routes = HashSet::new();
        for route in &self.routes {
            let area = self
                .areas
                .iter()
                .find(|a| a.id == route.area_id)
                .ok_or("路线区域不存在")?;
            if !crate::config::valid_identifier(&route.id)
                || !routes.insert(&route.id)
                || !valid_name(&route.tp_name)
                || route.points.is_empty()
                || route.points.len() > 4096
                || route.points.iter().any(|&p| !within(p, area))
                || area
                    .icons
                    .iter()
                    .filter(|i| i.icon_name == route.tp_name && i.template_id == "map_icon_01")
                    .count()
                    != 1
            {
                return Err("锄大地路线或起点无效");
            }
        }
        let mut lists = HashSet::new();
        for list in &self.lists {
            if !valid_name(&list.name)
                || !lists.insert(&list.name)
                || list.route_items.len() > 1024
                || list.route_items.iter().any(|id| !routes.contains(id))
                || list.route_items.iter().collect::<HashSet<_>>().len() != list.route_items.len()
            {
                return Err("锄大地名单含未知或重复路线");
            }
        }
        Ok(())
    }
    pub fn selected(&self, config: &WorldPatrolConfig) -> Result<Vec<PatrolRoute>, &'static str> {
        self.validate()?;
        config.validate()?;
        let list = if config.route_list.is_empty() {
            None
        } else {
            Some(
                self.lists
                    .iter()
                    .find(|l| l.name == config.route_list)
                    .ok_or("锄大地路线名单不存在")?,
            )
        };
        let routes: Vec<_> = self
            .routes
            .iter()
            .filter(|r| {
                list.is_none_or(|l| {
                    l.route_items.contains(&r.id) == (l.list_type == PatrolListType::Whitelist)
                })
            })
            .cloned()
            .collect();
        if routes.is_empty() {
            return Err("锄大地所选名单没有路线");
        }
        Ok(routes)
    }
    pub fn start(&self, route: &PatrolRoute) -> Result<PatrolPoint, &'static str> {
        self.areas
            .iter()
            .find(|a| a.id == route.area_id)
            .and_then(|a| {
                a.icons
                    .iter()
                    .find(|i| i.template_id == "map_icon_01" && i.icon_name == route.tp_name)
            })
            .map(|i| i.tp_pos)
            .ok_or("锄大地起点缺失")
    }
}
fn within(p: PatrolPoint, a: &PatrolArea) -> bool {
    p.x >= 0 && p.y >= 0 && p.x < a.width as i32 && p.y < a.height as i32
}
fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.trim() == s
        && s.len() <= 240
        && !s.chars().any(char::is_control)
        && !s.contains(['/', '\\', ':'])
        && !matches!(s, "." | "..")
}
pub fn catalog() -> Result<PatrolCatalog, &'static str> {
    let catalog: PatrolCatalog =
        serde_json::from_str(include_str!("../../../resources/world-patrol/catalog.json"))
            .map_err(|_| "内置锄大地路线无法读取")?;
    catalog.validate()?;
    Ok(catalog)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PatrolRetryAction {
    #[default]
    SkipOnStuckAgain,
    RetryOnStuckAgain,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct WorldPatrolConfig {
    pub auto_battle: String,
    pub route_list: String,
    pub daily_loop_count: u16,
    pub loop_interval_seconds: u32,
    pub route_retry_times: u8,
    pub route_retry_action: PatrolRetryAction,
    pub ui_disappear_seconds: u16,
    pub timeout_ms: u32,
}
impl Default for WorldPatrolConfig {
    fn default() -> Self {
        Self {
            auto_battle: "全配队通用".into(),
            route_list: String::new(),
            daily_loop_count: 1,
            loop_interval_seconds: 1800,
            route_retry_times: 1,
            route_retry_action: Default::default(),
            ui_disappear_seconds: 10,
            timeout_ms: 14_400_000,
        }
    }
}
impl WorldPatrolConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_name(&self.auto_battle)
            || (!self.route_list.is_empty() && !valid_name(&self.route_list))
            || !(1..=48).contains(&self.daily_loop_count)
            || self.loop_interval_seconds > 86400
            || self.route_retry_times > 10
            || !(1..=999).contains(&self.ui_disappear_seconds)
            || !(60_000..=86_400_000).contains(&self.timeout_ms)
        {
            return Err("锄大地脚本、循环、重试或超时设置无效");
        }
        Ok(())
    }
}
pub fn import(value: &serde_json::Value) -> Result<WorldPatrolConfig, &'static str> {
    let mut value = value.as_object().ok_or("旧锄大地配置必须是对象")?.clone();
    // 重启并登录游戏尚无可复用执行器，不能把旧版的重开策略伪装成已迁移。
    if let Some(action) = value.remove("ui_disappear_action")
        && action != "silent_fail"
    {
        return Err("锄大地自动重开游戏策略尚未迁移，请先在旧配置中选择静默失败");
    }
    let names = [
        "auto_battle",
        "route_list",
        "daily_loop_count",
        "loop_interval_seconds",
        "route_retry_times",
        "route_retry_action",
        "ui_disappear_seconds",
        "timeout_ms",
    ];
    value.retain(|key, _| names.contains(&key.as_str()));
    let settings: WorldPatrolConfig =
        serde_json::from_value(value.into()).map_err(|_| "旧锄大地配置字段类型或取值无效")?;
    catalog()?.selected(&settings)?;
    Ok(settings)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PatrolProgress {
    pub config: WorldPatrolConfig,
    pub routes: Vec<PatrolRoute>,
    pub completed_rounds: u16,
    pub route_index: usize,
    pub point_index: usize,
    pub attempt: u8,
    pub finished: Vec<String>,
    pub failed: Vec<String>,
    #[ts(type = "number")]
    pub round_started_ms: u64,
    #[ts(type = "number | null")]
    pub next_round_ms: Option<u64>,
}
impl PatrolProgress {
    pub fn new(config: WorldPatrolConfig) -> Result<Self, &'static str> {
        let routes = catalog()?.selected(&config)?;
        Ok(Self {
            config,
            routes,
            completed_rounds: 0,
            route_index: 0,
            point_index: 0,
            attempt: 0,
            finished: vec![],
            failed: vec![],
            round_started_ms: 0,
            next_round_ms: None,
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.routes != catalog()?.selected(&self.config)?
            || self.completed_rounds > self.config.daily_loop_count
            || self.route_index > self.routes.len()
            || self.attempt > self.config.route_retry_times
            || self.finished.len() + self.failed.len() != self.route_index
            || self
                .routes
                .iter()
                .take(self.route_index)
                .any(|r| !self.finished.contains(&r.id) && !self.failed.contains(&r.id))
            || (self.route_index == self.routes.len() && self.point_index != 0)
            || self.next_round_ms.is_some_and(|at| {
                self.completed_rounds == 0
                    || self.completed_rounds >= self.config.daily_loop_count
                    || at < self.round_started_ms
                    || self.route_index != self.routes.len()
                    || !self.failed.is_empty()
            })
            || self
                .routes
                .get(self.route_index)
                .is_some_and(|r| self.point_index > r.points.len())
            || self
                .finished
                .iter()
                .chain(&self.failed)
                .any(|id| !self.routes.iter().any(|r| &r.id == id))
            || self
                .finished
                .iter()
                .chain(&self.failed)
                .collect::<HashSet<_>>()
                .len()
                != self.finished.len() + self.failed.len()
        {
            return Err("锄大地检查点无效");
        }
        Ok(())
    }
    pub fn complete_route(&mut self, success: bool) -> Result<(), &'static str> {
        self.validate()?;
        let route = self
            .routes
            .get(self.route_index)
            .ok_or("没有当前锄大地路线")?;
        if success && self.point_index != route.points.len() {
            return Err("锄大地路线还没有到达终点");
        }
        if success {
            self.finished.push(route.id.clone());
        } else {
            self.failed.push(route.id.clone());
        }
        self.route_index += 1;
        self.point_index = 0;
        self.attempt = 0;
        Ok(())
    }
    pub fn finish_round(&mut self, active_ms: u64) -> Result<(), &'static str> {
        self.validate()?;
        if self.route_index != self.routes.len()
            || self.finished.len() != self.routes.len()
            || !self.failed.is_empty()
            || active_ms < self.round_started_ms
            || self.completed_rounds >= self.config.daily_loop_count
        {
            return Err("本轮路线未全部成功，不能增加完成轮数");
        }
        self.completed_rounds += 1;
        self.next_round_ms = (self.completed_rounds < self.config.daily_loop_count).then(|| {
            active_ms.max(
                self.round_started_ms
                    .saturating_add(u64::from(self.config.loop_interval_seconds) * 1000),
            )
        });
        Ok(())
    }
    pub fn next_round(&mut self, active_ms: u64) -> Result<(), &'static str> {
        self.validate()?;
        if self
            .next_round_ms
            .is_none_or(|deadline| active_ms < deadline)
        {
            return Err("尚未到下一轮时间");
        }
        self.route_index = 0;
        self.point_index = 0;
        self.attempt = 0;
        self.finished.clear();
        self.failed.clear();
        self.next_round_ms = None;
        self.round_started_ms = active_ms;
        Ok(())
    }
    pub fn completed(&self) -> bool {
        self.validate().is_ok()
            && self.completed_rounds == self.config.daily_loop_count
            && self.failed.is_empty()
            && self.route_index == self.routes.len()
            && self.finished.len() == self.routes.len()
    }
}
