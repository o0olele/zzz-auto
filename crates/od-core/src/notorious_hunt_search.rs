//! 周常横向副本列表查找。方向来自固定目录与当前可见名称，不从 OCR 缺失猜测。
use crate::{charge_plan::catalog, charge_transport::canonical_name};
use serde::{Deserialize, Serialize};

pub const MAX_SEARCH_DRAGS: usize = 12;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HuntDrag {
    Left,
    Right,
}
impl HuntDrag {
    pub fn delta(self) -> i32 {
        match self {
            Self::Left => -500,
            Self::Right => 500,
        }
    }
}
fn canonical_visible(visible: &[String]) -> Option<Vec<String>> {
    if visible.is_empty() || visible.len() > 16 {
        return None;
    }
    let mut result = Vec::new();
    for name in visible {
        let canonical = canonical_name("恶名狩猎", name)?;
        if canonical == "代理人方案培养" || result.iter().any(|n| n == canonical) {
            return None;
        }
        result.push(canonical.to_owned());
    }
    result.sort();
    Some(result)
}
pub fn direction(expected: &str, visible: &[String]) -> Option<HuntDrag> {
    let names = canonical_visible(visible)?;
    let category = catalog().iter().find(|c| c.category_name == "恶名狩猎")?;
    let target = canonical_name("恶名狩猎", expected)?;
    if target == "代理人方案培养" {
        return None;
    }
    let index = category
        .mission_type_list
        .iter()
        .position(|m| m.mission_type_name == target)?;
    let positions: Vec<_> = names
        .iter()
        .map(|n| {
            category
                .mission_type_list
                .iter()
                .position(|m| m.mission_type_name == *n)
        })
        .collect::<Option<_>>()?;
    // 与参考版的反向目录遍历一致：可见项位于目标之后时向左拖 500px。
    if positions.iter().all(|n| *n > index) {
        Some(HuntDrag::Left)
    } else if positions.iter().all(|n| *n < index) {
        Some(HuntDrag::Right)
    } else {
        None
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchStep {
    visible: Vec<String>,
    direction: HuntDrag,
    posted: bool,
    changed: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyListSearch {
    steps: Vec<SearchStep>,
    candidate: Option<Vec<String>>,
    stable: u8,
    stalled: u8,
}
impl WeeklyListSearch {
    pub fn validate(&self, expected: &str) -> Result<(), &'static str> {
        if self.steps.len() > MAX_SEARCH_DRAGS
            || self.stable > 2
            || self.stalled > 3
            || (self.candidate.is_some() != (self.stable > 0))
            || self.candidate.as_ref().is_some_and(|v| {
                canonical_visible(v).as_ref() != Some(v) || direction(expected, v).is_none()
            })
        {
            return Err("周常列表查找观察无效");
        }
        for (i, s) in self.steps.iter().enumerate() {
            if canonical_visible(&s.visible).as_ref() != Some(&s.visible)
                || direction(expected, &s.visible) != Some(s.direction)
                || (!s.changed && i + 1 != self.steps.len())
                || self.steps[..i].iter().any(|old| old.visible == s.visible)
            {
                return Err("周常列表查找历史或方向无效");
            }
        }
        Ok(())
    }
    pub fn pending(&self) -> bool {
        self.steps.last().is_some_and(|s| !s.changed)
    }
    pub fn exhausted(&self) -> bool {
        self.stalled == 3
    }
    pub fn reset_observation(&mut self) {
        self.candidate = None;
        self.stable = 0;
        self.stalled = 0;
    }
    /// None 表示没有一致可用的方向。重复旧页面、往返循环和上限均停止查找。
    pub fn observe(
        &mut self,
        expected: &str,
        visible: &[String],
    ) -> Result<Option<HuntDrag>, &'static str> {
        self.validate(expected)?;
        let reading = canonical_visible(visible).filter(|v| direction(expected, v).is_some());
        let Some(reading) = reading else {
            self.uncertain();
            return Ok(None);
        };
        if self.steps.iter().any(|s| s.visible == reading) || self.steps.len() == MAX_SEARCH_DRAGS {
            self.uncertain();
            return Ok(None);
        }
        if let Some(s) = self.steps.last_mut() {
            s.changed = true;
        }
        self.stalled = 0;
        self.stable = if self.candidate.as_ref() == Some(&reading) {
            (self.stable + 1).min(2)
        } else {
            1
        };
        self.candidate = Some(reading.clone());
        Ok((self.stable == 2)
            .then(|| direction(expected, &reading))
            .flatten())
    }
    pub fn uncertain(&mut self) {
        self.candidate = None;
        self.stable = 0;
        self.stalled = (self.stalled + 1).min(3);
    }
    pub fn target_seen(&mut self) {
        if let Some(s) = self.steps.last_mut() {
            s.changed = true;
        }
        self.reset_observation();
    }
    pub fn prepare(&mut self, expected: &str, drag: HuntDrag) -> Result<(), &'static str> {
        self.validate(expected)?;
        let visible = self.candidate.as_ref().ok_or("周常拖动没有当前列表观察")?;
        if self.pending()
            || self.stable != 2
            || self.steps.len() == MAX_SEARCH_DRAGS
            || direction(expected, visible) != Some(drag)
        {
            return Err("周常拖动没有连续方向证明或已达到上限");
        }
        self.steps.push(SearchStep {
            visible: visible.clone(),
            direction: drag,
            posted: false,
            changed: false,
        });
        self.reset_observation();
        Ok(())
    }
    pub fn allows(&self, expected: &str, drag: HuntDrag, visible: &[String]) -> bool {
        self.validate(expected).is_ok()
            && self.steps.last().is_some_and(|s| {
                !s.posted
                    && !s.changed
                    && s.direction == drag
                    && canonical_visible(visible).as_ref() == Some(&s.visible)
            })
    }
    pub fn posted(&mut self, drag: HuntDrag) -> Result<(), &'static str> {
        let s = self.steps.last_mut().ok_or("周常拖动意图缺失")?;
        if s.posted || s.changed || s.direction != drag {
            return Err("周常拖动回执不匹配");
        }
        s.posted = true;
        Ok(())
    }
    pub fn not_posted(&mut self, drag: HuntDrag) -> Result<(), &'static str> {
        let s = self.steps.last().ok_or("周常拖动意图缺失")?;
        if s.posted || s.changed || s.direction != drag {
            return Err("不能撤销已投递周常拖动");
        }
        self.steps.pop();
        self.reset_observation();
        Ok(())
    }
}
