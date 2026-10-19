//! 仅解析参考资料中的字面量与 Agent / AgentStateDef 声明，不执行 Python。
use anyhow::{Context, Result, bail, ensure};
use od_core::combat_vision::{Agent, AgentStateDef};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
enum Literal { Json(Value), Symbol(String), Call(String, Vec<Literal>, BTreeMap<String, Literal>) }
struct Parser<'a> { input: &'a str, pos: usize, depth: usize }
impl Parser<'_> {
    fn space(&mut self) {
        loop {
            while self.input.as_bytes().get(self.pos).is_some_and(u8::is_ascii_whitespace) { self.pos += 1; }
            if self.input.as_bytes().get(self.pos) == Some(&b'#') {
                self.pos += self.input[self.pos..].find('\n').unwrap_or(self.input.len() - self.pos);
            } else { break; }
        }
    }
    fn eat(&mut self, c: u8) -> bool { self.space(); if self.input.as_bytes().get(self.pos) == Some(&c) { self.pos += 1; true } else { false } }
    fn identifier(&mut self) -> Result<String> {
        self.space(); let start = self.pos;
        while self.input.as_bytes().get(self.pos).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'.') { self.pos += 1; }
        ensure!(start != self.pos, "字面量位置 {} 需要标识符", self.pos);
        Ok(self.input[start..self.pos].into())
    }
    fn parse(&mut self) -> Result<Literal> {
        self.depth += 1; ensure!(self.depth <= 32, "资料表达式嵌套过深"); self.space();
        let Some(&first) = self.input.as_bytes().get(self.pos) else { bail!("资料表达式截断"); };
        let result = if first == b'\'' || first == b'"' {
            self.pos += 1; let mut result = String::new();
            loop {
                let c = self.input[self.pos..].chars().next().context("字符串未结束")?; self.pos += c.len_utf8();
                if c as u32 == first as u32 { break; }
                if c == '\\' { let c = self.input[self.pos..].chars().next().context("字符串转义未结束")?; self.pos += c.len_utf8(); result.push(match c { 'n' => '\n', 'r' => '\r', 't' => '\t', '\\' | '\'' | '"' => c, _ => bail!("不支持的资料字符串转义") }); } else { result.push(c); }
            }
            Literal::Json(Value::String(result))
        } else if first == b'[' || first == b'(' {
            self.pos += 1; let end = if first == b'[' { b']' } else { b')' }; let mut values = Vec::new();
            while !self.eat(end) { values.push(self.parse()?); if !self.eat(b',') { ensure!(self.eat(end), "资料数组缺失结束符"); break; } }
            Literal::Json(Value::Array(values.into_iter().map(as_json).collect::<Result<_>>()?))
        } else if first.is_ascii_digit() || first == b'-' {
            let start = self.pos; self.pos += 1;
            while self.input.as_bytes().get(self.pos).is_some_and(|c| c.is_ascii_digit() || [b'.', b'e', b'E', b'+', b'-'].contains(c)) { self.pos += 1; }
            Literal::Json(serde_json::from_str(&self.input[start..self.pos])?)
        } else {
            let name = self.identifier()?;
            if self.eat(b'(') {
                let mut args = Vec::new(); let mut kwargs = BTreeMap::new();
                while !self.eat(b')') {
                    let saved = self.pos;
                    let key = self.identifier().ok();
                    if let Some(key) = key.filter(|_| self.eat(b'=')) { ensure!(!kwargs.contains_key(&key), "重复资料字段 {key}"); kwargs.insert(key, self.parse()?); }
                    else { self.pos = saved; args.push(self.parse()?); }
                    if !self.eat(b',') { ensure!(self.eat(b')'), "资料调用缺失结束符"); break; }
                }
                Literal::Call(name, args, kwargs)
            } else { match name.as_str() { "None" => Literal::Json(Value::Null), "True" => Literal::Json(Value::Bool(true)), "False" => Literal::Json(Value::Bool(false)), _ => Literal::Symbol(name) } }
        };
        self.depth -= 1; Ok(result)
    }
}
fn as_json(value: Literal) -> Result<Value> {
    match value {
        Literal::Json(v) => Ok(v),
        Literal::Symbol(name) => Ok(Value::String(name)),
        Literal::Call(name, args, fields) if name == "AgentStateDef" => {
            ensure!((2..=3).contains(&args.len()), "AgentStateDef 位置参数数量不匹配");
            let mut map = serde_json::Map::new();
            for (key, value) in ["state_name", "check_way", "template_id"].into_iter().zip(args) { map.insert(key.into(), as_json(value)?); }
            for (key, value) in fields { ensure!(!map.contains_key(&key), "AgentStateDef 字段重复 {key}"); map.insert(key, as_json(value)?); }
            if map["check_way"] == "AgentStateCheckWay.COLOR_RANGE_EXIST" && !map.contains_key("min_value_trigger_state") { map.insert("min_value_trigger_state".into(), json!(1)); }
            let mut state: AgentStateDef = serde_json::from_value(Value::Object(map))?;
            state.check_way = state.check_way.strip_prefix("AgentStateCheckWay.").context("未知状态识别方式")?.into();
            serde_json::to_value(state).map_err(Into::into)
        },
        _ => bail!("不支持的资料调用"),
    }
}

pub fn parse_agents(source: &str) -> Result<(Vec<Agent>, BTreeMap<String, AgentStateDef>)> {
    let agent_source = source.split("class AgentEnum(Enum):").nth(1).context("缺失 AgentEnum")?;
    let common_source = source.split("class CommonAgentStateEnum(Enum):").nth(1).and_then(|s| s.split("class Agent:").next()).context("缺失公共状态")?;
    let pattern = regex::Regex::new(r"(?m)^    ([A-Z][A-Z0-9_]*)\s*=\s*(Agent|AgentStateDef)\(")?;
    let mut common = BTreeMap::new();
    for capture in pattern.captures_iter(common_source) {
        let start = capture.get(2).unwrap().start();
        let value = Parser { input: &common_source[start..], pos: 0, depth: 0 }.parse()?;
        common.insert(capture[1].to_string(), serde_json::from_value(as_json(value)?)?);
    }
    let mut agents = Vec::new();
    let kinds = BTreeMap::from([("ATTACK", "强攻"), ("STUN", "击破"), ("SUPPORT", "支援"), ("DEFENSE", "防护"), ("ANOMALY", "异常"), ("RUPTURE", "命破"), ("ARMORER", "锋御")]);
    for capture in pattern.captures_iter(agent_source) {
        let start = capture.get(2).unwrap().start();
        let value = Parser { input: &agent_source[start..], pos: 0, depth: 0 }.parse().with_context(|| capture[1].to_string())?;
        let Literal::Call(name, args, mut fields) = value else { bail!("代理人不是声明"); };
        ensure!(name == "Agent" && args.len() == 6 && fields.keys().all(|k| k == "state_list"), "不支持的代理人声明 {}", &capture[1]);
        let values: Vec<_> = args.into_iter().map(as_json).collect::<Result<_>>()?;
        let kind = values[3].as_str().and_then(|s| s.strip_prefix("AgentTypeEnum.")).and_then(|s| kinds.get(s)).context("未知代理人类型")?;
        agents.push(Agent { id: serde_json::from_value(values[0].clone())?, name: serde_json::from_value(values[1].clone())?, kind: kind.to_string(), templates: serde_json::from_value(values[5].clone())?, states: fields.remove("state_list").map(as_json).transpose()?.map(serde_json::from_value).transpose()?.unwrap_or_default() });
    }
    ensure!(agents.len() >= 16 && common.len() >= 15, "角色资料解析不完整");
    Ok((agents, common))
}
