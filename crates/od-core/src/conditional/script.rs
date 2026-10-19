use super::Expression;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::{collections::BTreeSet, path::{Path, PathBuf}};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ButtonWay { Tap, Down, Up }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Action {
    Wait { seconds: f64 },
    Button { name: String, way: ButtonWay, press: Option<f64> },
    SwitchAgent { name: String },
    QuickAssist,
    SetState { names: Vec<String>, seconds: f64, seconds_add: f64, value: Option<i64>, add: Option<i64> },
    ClearState { names: Vec<String> },
    Turn { distance: f64 },
}

#[derive(Debug, Clone)]
pub struct Handler {
    pub source: String,
    pub condition: Expression,
    pub interrupt: Option<Expression>,
    pub label: String,
    pub children: Vec<Handler>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub triggers: Vec<String>,
    pub priority: Option<i32>,
    pub interval: f64,
    pub handlers: Vec<Handler>,
}

#[derive(Debug, Clone)]
pub struct Script {
    pub name: String,
    pub source: String,
    pub scenes: Vec<Scene>,
    pub states: BTreeSet<String>,
    pub options: Mapping,
}

pub trait TemplateResolver {
    fn resolve(&self, kind: &str, name: &str) -> Result<(String, String)>;
}

pub struct FileTemplates { pub root: PathBuf }
impl FileTemplates {
    pub fn read(root: &Path, kind: &str, name: &str, merged: bool) -> Result<(String, String)> {
        ensure!(!name.is_empty() && name.len() <= 240 && !name.contains(['/', '\\', ':']) && name != "." && name != "..", "模板名称非法: {name}");
        let folder = root.join(kind);
        let extensions: &[&str] = if merged { &["merged.yml", "yml", "sample.yml"] } else { &["yml", "sample.yml"] };
        for extension in extensions {
            let path = folder.join(format!("{name}.{extension}"));
            if path.is_file() {
                let root = root.canonicalize()?;
                let resolved = path.canonicalize()?;
                ensure!(resolved.starts_with(root), "模板位于配置目录之外");
                ensure!(resolved.metadata()?.len() <= 4 * 1024 * 1024, "脚本超过 4 MiB");
                return Ok((resolved.display().to_string(), std::fs::read_to_string(&resolved)?));
            }
        }
        bail!("找不到 {kind}/{name}")
    }
}
impl TemplateResolver for FileTemplates {
    fn resolve(&self, kind: &str, name: &str) -> Result<(String, String)> { Self::read(&self.root, kind, name, false) }
}

impl Script {
    pub fn load(root: &Path, name: &str) -> Result<Self> {
        Self::load_entry(root,"auto_battle",name,true)
    }
    pub fn load_entry(root:&Path,category:&str,name:&str,merged:bool)->Result<Self> {
        ensure!(["auto_battle","dodge"].contains(&category),"脚本分类无效");
        let (file, source) = FileTemplates::read(root, category, name, merged)?;
        Self::parse(name, &file, &source, &FileTemplates { root: root.into() })
    }
    pub fn parse(name: &str, file: &str, source: &str, resolver: &dyn TemplateResolver) -> Result<Self> {
        ensure!(source.len() <= 4 * 1024 * 1024, "{file}: 脚本超过 4 MiB");
        let mut root: Value = serde_yaml::from_str(source).with_context(|| format!("{file}: YAML"))?;
        root.apply_merge().with_context(||format!("{file}: YAML 合并键"))?;
        let map = object(&root, file)?;
        let mut executable = map.clone();
        // 历史脚本用根级 t-* 保存 YAML 锚点，执行内容仍来自 scenes。
        let anchors=anchor_keys(source);
        executable.retain(|key, _| !key.as_str().is_some_and(|s| s.starts_with("t-")||anchors.contains(s)));
        keys(&executable, &["name", "scenes", "author", "homepage", "thanks", "version", "introduction", "team_list", "check_dodge_interval", "check_agent_interval", "check_special_attack_interval", "check_ultimate_interval", "check_chain_interval", "check_quick_interval", "check_end_interval", "target_lock_interval", "abnormal_status_interval", "auto_lock_interval", "auto_turn_interval"], file)?;
        for (key, value) in &executable {
            if key.as_str().is_some_and(|s| s.ends_with("_interval")) { parse_interval(value, &format!("{file}.{}", key.as_str().unwrap()))?; }
        }
        let mut loader = Loader { resolver, stack: BTreeSet::new(), count: 0 };
        let mut scenes = Vec::new();
        let mut triggers = BTreeSet::new();
        let mut normal = false;
        let mut states = BTreeSet::new();
        for (index, data) in list(map, "scenes", file)?.iter().enumerate() {
            let location = format!("{file}: scenes[{index}]");
            let scene = object(data, &location)?;
            keys(scene, &["triggers", "priority", "interval", "handlers"], &location)?;
            let names = strings(scene, "triggers", &location)?;
            if names.is_empty() { ensure!(!normal, "{location}: 无触发器场景重复"); normal = true; }
            for name in &names { ensure!(triggers.insert(name.clone()), "{location}.triggers: 重复触发器 {name}"); states.insert(name.clone()); }
            let interval = number(scene, "interval", 0.5, &location)?;
            ensure!((0.0..=3600.0).contains(&interval), "{location}.interval: 超出 0..3600");
            let priority = scene.get("priority").filter(|v| !v.is_null()).map(|v| integer(v, &format!("{location}.priority")).and_then(|n| Ok(i32::try_from(n)?))).transpose()?;
            let handlers = loader.handlers(list(scene, "handlers", &location)?, &location, 0)?;
            ensure!(!handlers.is_empty(), "{location}.handlers: 不能为空");
            collect_states(&handlers, &mut states);
            scenes.push(Scene { triggers: names, priority, interval, handlers });
        }
        ensure!(!scenes.is_empty() && scenes.len() <= 256, "{file}.scenes: 需要 1..256 个场景");
        Ok(Self { name: name.into(), source: file.into(), scenes, states, options: map.clone() })
    }
    pub fn interval(&self, name: &str, default: f64) -> Result<f64> { number(&self.options, name, default, &self.source) }
    pub fn interval_range(&self, name: &str, default: f64) -> Result<[f64; 2]> {
        self.options.get(name).map(|value| parse_interval(value, &format!("{}.{name}", self.source))).unwrap_or(Ok([default, default]))
    }
}

fn parse_interval(value: &Value, loc: &str) -> Result<[f64; 2]> {
    let range = if let Some(n) = value.as_f64() { [n, n] } else {
        let values = value.as_sequence().filter(|v| v.len() == 2).context(format!("{loc}: 间隔需为数字或两个数字的范围"))?;
        [values[0].as_f64().context(format!("{loc}: 间隔下限无效"))?, values[1].as_f64().context(format!("{loc}: 间隔上限无效"))?]
    };
    ensure!(range.iter().all(|n| n.is_finite() && (-1.0..=3600.0).contains(n)) && range[0] <= range[1], "{loc}: 间隔范围无效");
    Ok(range)
}

fn collect_states(handlers: &[Handler], states: &mut BTreeSet<String>) {
    for handler in handlers {
        handler.condition.states(states);
        if let Some(interrupt) = &handler.interrupt { interrupt.states(states); }
        for action in &handler.actions {
            if let Action::SetState { names, .. } | Action::ClearState { names } = action { states.extend(names.iter().cloned()); }
        }
        collect_states(&handler.children, states);
    }
}

struct Loader<'a> { resolver: &'a dyn TemplateResolver, stack: BTreeSet<String>, count: usize }
impl Loader<'_> {
    fn template(&mut self, kind: &str, name: &str) -> Result<(String, Value,BTreeSet<String>)> {
        let id = format!("{kind}/{name}");
        ensure!(self.stack.len() < 64 && self.stack.insert(id), "模板循环或嵌套过深: {kind}/{name}");
        let (file, text) = self.resolver.resolve(kind, name)?;
        ensure!(text.len() <= 4 * 1024 * 1024, "{file}: 模板过大");
        let mut value:Value = serde_yaml::from_str(&text).with_context(|| format!("{file}: YAML"))?;
        value.apply_merge().with_context(||format!("{file}: YAML 合并键"))?;
        Ok((file, value,anchor_keys(&text)))
    }
    fn handlers(&mut self, values: &[Value], parent: &str, depth: usize) -> Result<Vec<Handler>> {
        ensure!(depth < 64, "{parent}: handler 嵌套过深");
        let mut result = Vec::new();
        for (i, value) in values.iter().enumerate() {
            self.count += 1;
            ensure!(self.count <= 100_000, "{parent}: 模板展开超过上限");
            let loc = format!("{parent}.handlers[{i}]");
            let map = object(value, &loc)?;
            keys(map, &["state_template", "debug_name", "states", "interrupt_states", "operations", "sub_handlers"], &loc)?;
            if let Some(name) = text(map, "state_template", &loc)? {
                ensure!(map.len() == 1, "{loc}: state_template 不能混用其他字段");
                let (file, value,anchors) = self.template("auto_battle_state_handler", name).with_context(|| loc.clone())?;
                let template = object(&value, &file)?;
                let mut executable=template.clone();executable.retain(|key,_|!key.as_str().is_some_and(|s|anchors.contains(s)));
                keys(&executable, &["handlers","template_name","description"], &file)?;
                result.extend(self.handlers(list(template, "handlers", &file)?, &file, depth + 1)?);
                self.stack.remove(&format!("auto_battle_state_handler/{name}"));
                continue;
            }
            let condition = Expression::parse(text(map, "states", &loc)?.unwrap_or("")).with_context(|| format!("{loc}.states"))?;
            let interrupt = text(map, "interrupt_states", &loc)?.map(Expression::parse).transpose().with_context(|| format!("{loc}.interrupt_states"))?;
            let children = self.handlers(optional_list(map, "sub_handlers", &loc)?, &format!("{loc}.sub_handlers"), depth + 1)?;
            let actions = self.actions(optional_list(map, "operations", &loc)?, &loc, 0)?;
            ensure!(children.is_empty() || actions.is_empty(), "{loc}: sub_handlers 与 operations 不能共存");
            result.push(Handler { source: loc, condition, interrupt, label: text(map, "debug_name", parent)?.unwrap_or("").into(), children, actions });
        }
        Ok(result)
    }
    fn actions(&mut self, values: &[Value], parent: &str, depth: usize) -> Result<Vec<Action>> {
        ensure!(depth < 64, "{parent}: 动作模板嵌套过深");
        let mut result = Vec::new();
        for (i, value) in values.iter().enumerate() {
            self.count += 1;
            ensure!(self.count <= 100_000, "{parent}: 模板展开超过上限");
            let loc = format!("{parent}.operations[{i}]");
            let map = object(value, &loc)?;
            keys(map, &["op_name", "operation_template", "data", "pre_delay", "post_delay", "way", "press", "repeat", "seconds", "state", "state_list", "seconds_add", "value", "add", "agent_name", "distance"], &loc)?;
            if let Some(name) = text(map, "operation_template", &loc)? {
                ensure!(map.len() == 1, "{loc}: operation_template 不能混用其他字段");
                let (file, value,anchors) = self.template("auto_battle_operation", name).with_context(|| loc.clone())?;
                let template = object(&value, &file)?;
                let mut executable=template.clone();executable.retain(|key,_|!key.as_str().is_some_and(|s|anchors.contains(s)));
                keys(&executable, &["operations","template_name","description"], &file)?;
                result.extend(self.actions(list(template, "operations", &file)?, &file, depth + 1)?);
                self.stack.remove(&format!("auto_battle_operation/{name}"));
                continue;
            }
            let name = text(map, "op_name", &loc)?.context(format!("{loc}.op_name: 缺失"))?;
            let data: Vec<String> = optional_list(map, "data", &loc)?.iter().map(|v| match v { Value::String(s) => Ok(s.clone()), Value::Number(n) => Ok(n.to_string()), _ => bail!("{loc}.data: 需要字符串或数字") }).collect::<Result<_>>()?;
            let pre = duration(map, "pre_delay", 0.0, &loc)?;
            let post = duration(map, "post_delay", 0.0, &loc)?;
            let repeat = map.get("repeat").map(|v| integer(v, &loc)).transpose()?.unwrap_or(1);
            ensure!((1..=1000).contains(&repeat), "{loc}.repeat: 需要 1..1000");
            let legacy = |index: usize, fallback: f64| -> Result<f64> { data.get(index).map(|s| s.parse::<f64>().with_context(|| format!("{loc}.data[{index}]"))).unwrap_or(Ok(fallback)) };
            let names = || -> Result<Vec<String>> {
                let mut names = strings(map, "state_list", &loc)?;
                if names.is_empty() { if let Some(name) = text(map, "state", &loc)?.or_else(|| data.first().map(String::as_str)) { names.push(name.into()); } }
                ensure!(!names.is_empty(), "{loc}.state: 需要状态名称");
                ensure!(names.iter().all(|n| !n.is_empty() && n.len() <= 256), "{loc}.state: 状态名为空或过长");
                Ok(names)
            };
            let action = match name {
                "等待秒数" => Action::Wait { seconds: duration(map, "seconds", legacy(0, 0.0)?, &loc)? },
                "设置状态" => Action::SetState {
                    names: names()?, seconds: number(map, "seconds", legacy(1, 0.0)?, &loc)?,
                    seconds_add: number(map, "seconds_add", 0.0, &loc)?,
                    value: map.get("value").map(|v| integer(v, &loc)).transpose()?.or(data.get(2).map(|s| s.parse::<i64>()).transpose()?),
                    add: map.get("add").map(|v| integer(v, &loc)).transpose()?,
                },
                "清除状态" => Action::ClearState { names: names()? },
                "切换角色" | "按键-切换角色" => Action::SwitchAgent { name: text(map, "agent_name", &loc)?.or_else(|| data.first().map(String::as_str)).context(format!("{loc}.agent_name: 缺失"))?.into() },
                "按键-快速支援" => Action::QuickAssist,
                "转向" => Action::Turn { distance: number(map, "distance", legacy(0, 100.0)?, &loc)? },
                _ => {
                    let (base, suffix) = if let Some(s) = name.strip_suffix("-按下") { (s, Some(ButtonWay::Down)) } else if let Some(s) = name.strip_suffix("-松开") { (s, Some(ButtonWay::Up)) } else { (name, None) };
                    ensure!(["按键-闪避", "按键-切换角色-下一个", "按键-切换角色-上一个", "按键-切换后援", "按键-普通攻击", "按键-特殊攻击", "按键-终结技", "按键-连携技-左", "按键-连携技-右", "按键-连携技-取消", "按键-移动-前", "按键-移动-后", "按键-移动-左", "按键-移动-右", "按键-锁定敌人"].contains(&base), "{loc}.op_name: 未知指令 {name}");
                    let way = match text(map, "way", &loc)? { Some("按下") => ButtonWay::Down, Some("松开") => ButtonWay::Up, Some("点按") | None => suffix.unwrap_or(ButtonWay::Tap), Some(other) => bail!("{loc}.way: 未知方式 {other}") };
                    let press = if map.contains_key("press") { Some(duration(map, "press", 0.0, &loc)?) } else if way == ButtonWay::Down && !data.is_empty() { Some(legacy(0, 0.0)?) } else { None };
                    ensure!(press.is_none_or(|p| p.is_finite() && (0.0..=3600.0).contains(&p)), "{loc}.press: 无效时长");
                    Action::Button { name: base.strip_prefix("按键-").unwrap().into(), way, press }
                }
            };
            self.count += repeat as usize * (1 + usize::from(pre > 0.0) + usize::from(post > 0.0));
            ensure!(self.count<=100_000, "{loc}: 动作展开总数超过上限");
            for _ in 0..repeat {
                if pre > 0.0 { result.push(Action::Wait { seconds: pre }); }
                result.push(action.clone());
                if post > 0.0 { result.push(Action::Wait { seconds: post }); }
            }
            ensure!(result.len() <= 100_000, "{loc}: 展开动作数量超过上限");
        }
        Ok(result)
    }
}

fn object<'a>(value: &'a Value, loc: &str) -> Result<&'a Mapping> { value.as_mapping().context(format!("{loc}: 需要对象")) }
fn anchor_keys(source:&str)->BTreeSet<String> {
    source.lines().filter(|line|line.chars().next().is_some_and(|c|!c.is_whitespace())).filter_map(|line|{
        let (key,value)=line.split_once(':')?;
        value.trim_start().starts_with('&').then(||key.trim().trim_matches(['\'','"']).to_string())
    }).collect()
}
fn keys(map: &Mapping, allowed: &[&str], loc: &str) -> Result<()> {
    for key in map.keys() { ensure!(key.as_str().is_some_and(|s| allowed.contains(&s)), "{loc}: 未知字段 {key:?}"); }
    Ok(())
}
fn text<'a>(map: &'a Mapping, key: &str, loc: &str) -> Result<Option<&'a str>> { map.get(key).filter(|v| !v.is_null()).map(|v| v.as_str().context(format!("{loc}.{key}: 需要字符串"))).transpose() }
fn list<'a>(map: &'a Mapping, key: &str, loc: &str) -> Result<&'a [Value]> { map.get(key).and_then(Value::as_sequence).map(Vec::as_slice).context(format!("{loc}.{key}: 需要数组")) }
fn optional_list<'a>(map: &'a Mapping, key: &str, loc: &str) -> Result<&'a [Value]> { if map.contains_key(key) { list(map, key, loc) } else { Ok(&[]) } }
fn strings(map: &Mapping, key: &str, loc: &str) -> Result<Vec<String>> { optional_list(map, key, loc)?.iter().map(|v| { let s = v.as_str().context(format!("{loc}.{key}: 需要字符串"))?; ensure!(!s.is_empty() && s.len() <= 256, "{loc}.{key}: 名称为空或过长"); Ok(s.into()) }).collect() }
fn number(map: &Mapping, key: &str, default: f64, loc: &str) -> Result<f64> {
    let n = map.get(key).map(|v| v.as_f64().context(format!("{loc}.{key}: 需要数字"))).transpose()?.unwrap_or(default);
    ensure!(n.is_finite() && n.abs() <= 86_400.0, "{loc}.{key}: 需要有界有限数字"); Ok(n)
}
fn duration(map: &Mapping, key: &str, default: f64, loc: &str) -> Result<f64> { let n = number(map, key, default, loc)?; ensure!((0.0..=3600.0).contains(&n), "{loc}.{key}: 时长超出 0..3600"); Ok(n) }
fn integer(value: &Value, loc: &str) -> Result<i64> { value.as_i64().context(format!("{loc}: 需要整数")) }
