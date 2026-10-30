use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateValue {
    pub at: f64,
    pub value: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateUpdate {
    pub name: String,
    pub at: f64,
    #[serde(default)]
    pub value: Option<i64>,
    #[serde(default)]
    pub add: Option<i64>,
    #[serde(default)]
    pub time_add: Option<f64>,
    #[serde(default)]
    pub clear: bool,
}

#[derive(Debug, Clone, Default)]
pub struct StateBook {
    values: BTreeMap<String, StateValue>,
    mutex: BTreeMap<String, BTreeSet<String>>,
}

impl StateBook {
    pub fn values(&self) -> &BTreeMap<String, StateValue> {
        &self.values
    }
    pub fn reset(&mut self) {
        self.values.clear();
    }
    pub(super) fn reset_observation(&mut self) {
        self.values.retain(|name, _| name.starts_with("自定义-"));
    }
    pub fn add_mutex(&mut self, names: &[String]) {
        for name in names {
            self.mutex
                .entry(name.clone())
                .or_default()
                .extend(names.iter().filter(|n| *n != name).cloned());
        }
    }
    /// 部分旧状态是单向互斥，例如“后台-角色”可与后台位置状态同时存在。
    pub fn add_exclusions(&mut self, name: String, others: impl IntoIterator<Item = String>) {
        self.mutex.entry(name).or_default().extend(others);
    }
    pub fn apply(&mut self, update: &StateUpdate) -> Result<()> {
        ensure!(
            !update.name.is_empty() && update.name.len() <= 256 && update.at.is_finite(),
            "状态名或时间无效"
        );
        ensure!(
            update.time_add.is_none_or(f64::is_finite),
            "状态时间偏移无效"
        );
        if update.clear {
            self.values.remove(&update.name);
            return Ok(());
        }
        let previous = self.values.get(&update.name);
        let at = match update.time_add.filter(|v| *v != 0.0) {
            Some(offset) => match previous {
                Some(value) => value.at - offset,
                None => return Ok(()),
            },
            None => update.at,
        };
        let value = update
            .value
            .unwrap_or_else(|| previous.map_or(0, |v| v.value))
            .checked_add(update.add.unwrap_or(0))
            .ok_or_else(|| anyhow::anyhow!("状态数值溢出"))?;
        ensure!(at.is_finite(), "状态时间溢出");
        ensure!(
            self.values.contains_key(&update.name) || self.values.len() < 8192,
            "状态数量超过上限"
        );
        if let Some(others) = self.mutex.get(&update.name) {
            for name in others {
                self.values.remove(name);
            }
        }
        self.values
            .insert(update.name.clone(), StateValue { at, value });
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    True,
    State {
        name: String,
        min: f64,
        max: f64,
        value: Option<(i64, i64)>,
    },
    Not(Box<Expression>),
    And(Box<Expression>, Box<Expression>),
    Or(Box<Expression>, Box<Expression>),
}

impl Expression {
    pub fn parse(source: &str) -> Result<Self> {
        ensure!(source.len() <= 16_384, "状态表达式过长");
        if source.trim().is_empty() {
            return Ok(Self::True);
        }
        let mut parser = Parser {
            source,
            pos: 0,
            depth: 0,
            nodes: 0,
        };
        let value = parser.binary()?;
        parser.space();
        ensure!(
            parser.pos == source.len(),
            "状态表达式位置 {} 有多余内容",
            parser.pos + 1
        );
        Ok(value)
    }
    pub fn evaluate(&self, states: &StateBook, now: f64) -> bool {
        if !now.is_finite() {
            return false;
        }
        match self {
            Self::True => true,
            Self::State {
                name,
                min,
                max,
                value,
            } => states.values.get(name).is_some_and(|record| {
                let age = now - record.at;
                age >= *min
                    && age <= *max
                    && value.is_none_or(|(lo, hi)| record.value >= lo && record.value <= hi)
            }),
            Self::Not(child) => !child.evaluate(states, now),
            Self::And(left, right) => left.evaluate(states, now) && right.evaluate(states, now),
            Self::Or(left, right) => left.evaluate(states, now) || right.evaluate(states, now),
        }
    }
    pub fn states(&self, into: &mut BTreeSet<String>) {
        match self {
            Self::State { name, .. } => {
                into.insert(name.clone());
            }
            Self::Not(child) => child.states(into),
            Self::And(l, r) | Self::Or(l, r) => {
                l.states(into);
                r.states(into);
            }
            Self::True => {}
        }
    }
}

struct Parser<'a> {
    source: &'a str,
    pos: usize,
    depth: usize,
    nodes: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while let Some(c) = self.source[self.pos..]
            .chars()
            .next()
            .filter(|c| c.is_whitespace())
        {
            self.pos += c.len_utf8();
        }
    }
    fn eat(&mut self, c: char) -> bool {
        self.space();
        if self.source[self.pos..].starts_with(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }
    fn binary(&mut self) -> Result<Expression> {
        let mut left = self.unary()?;
        // 旧脚本的 & 和 | 同优先级，按出现顺序结合。
        loop {
            let and = if self.eat('&') {
                true
            } else if self.eat('|') {
                false
            } else {
                break;
            };
            let right = self.unary()?;
            left = if and {
                Expression::And(Box::new(left), Box::new(right))
            } else {
                Expression::Or(Box::new(left), Box::new(right))
            };
        }
        Ok(left)
    }
    fn unary(&mut self) -> Result<Expression> {
        self.nodes += 1;
        ensure!(self.nodes <= 256, "状态表达式节点过多");
        self.depth += 1;
        ensure!(self.depth <= 64, "状态表达式嵌套过深");
        let value = if self.eat('!') {
            Expression::Not(Box::new(self.unary()?))
        } else if self.eat('(') {
            let expression = self.binary()?;
            ensure!(self.eat(')'), "状态表达式位置 {} 缺少右括号", self.pos + 1);
            expression
        } else {
            ensure!(
                self.eat('['),
                "状态表达式位置 {} 需要状态或括号",
                self.pos + 1
            );
            let fields = self.fields(']')?;
            ensure!(
                fields.len() == 1 || fields.len() == 3,
                "状态需为 [名称] 或 [名称,最小秒,最大秒]"
            );
            let name = fields[0].to_string();
            ensure!(!name.is_empty() && name.len() <= 256, "状态名为空或过长");
            let (min, max) = if fields.len() == 1 {
                (0.0, 1.0)
            } else {
                (fields[1].parse::<f64>()?, fields[2].parse::<f64>()?)
            };
            ensure!(
                min.is_finite() && max.is_finite() && min <= max,
                "状态时间范围无效"
            );
            let value = if self.eat('{') {
                let values = self.fields('}')?;
                ensure!(
                    (1..=2).contains(&values.len()),
                    "状态数值需为 {{值}} 或 {{最小值,最大值}}"
                );
                let lo = values[0].parse::<i64>()?;
                let hi = if values.len() == 2 {
                    values[1].parse()?
                } else {
                    lo
                };
                ensure!(lo <= hi, "状态数值范围反向");
                Some((lo, hi))
            } else {
                None
            };
            Expression::State {
                name,
                min,
                max,
                value,
            }
        };
        self.depth -= 1;
        Ok(value)
    }
    fn fields(&mut self, end: char) -> Result<Vec<&str>> {
        let Some(offset) = self.source[self.pos..].find(end) else {
            bail!("状态表达式缺少 {end}");
        };
        let result = self.source[self.pos..self.pos + offset]
            .split(',')
            .map(str::trim)
            .collect();
        self.pos += offset + end.len_utf8();
        Ok(result)
    }
}
