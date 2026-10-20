use super::{Action, Expression, Handler, Script, StateBook, StateUpdate};
use anyhow::{Result, ensure};

#[derive(Debug, Clone)]
pub struct Execution {
    pub generation: u64,
    pub scene: usize,
    pub priority: Option<i32>,
    pub label: String,
    pub source: String,
    pub actions: Vec<Action>,
    pub interrupt: Option<Expression>,
}

#[derive(Debug, Clone)]
pub enum Decision {
    Idle,
    Start(Execution),
    /// 调用者必须先释放旧输入，确认后才可执行新序列。
    Replace(Execution),
    Interrupt,
}

pub struct Scheduler {
    pub script: Script,
    pub states: StateBook,
    active: Option<Execution>,
    last_scene: Vec<Option<f64>>,
    generation: u64,
    paused: bool,
    last_tick: f64,
}

impl Scheduler {
    pub fn new(script: Script) -> Self {
        Self {
            last_scene: vec![None; script.scenes.len()],
            script,
            states: StateBook::default(),
            active: None,
            generation: 0,
            paused: false,
            last_tick: 0.0,
        }
    }
    pub fn active(&self) -> Option<&Execution> {
        self.active.as_ref()
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn pause(&mut self) {
        self.paused = true;
        self.active = None;
        self.generation += 1;
        self.states.reset();
    }
    pub fn resume(&mut self) {
        self.paused = false;
        self.last_scene.fill(None);
        self.states.reset();
    }
    pub fn complete(&mut self, generation: u64) -> Result<()> {
        ensure!(
            self.active
                .as_ref()
                .is_some_and(|a| a.generation == generation),
            "动作代际已失效"
        );
        self.active = None;
        Ok(())
    }
    pub fn tick(&mut self, now: f64, updates: &[StateUpdate]) -> Result<Decision> {
        ensure!(
            now.is_finite() && now >= self.last_tick,
            "调度时钟必须单调递增"
        );
        ensure!(updates.len() <= 8192, "状态批次过大");
        self.last_tick = now;
        if self.paused {
            return Ok(Decision::Idle);
        }
        let mut triggers = std::collections::BTreeSet::new();
        for update in updates {
            self.states.apply(update)?;
            if !update.clear {
                triggers.insert(update.name.as_str());
            }
        }
        let interrupted = self.active.as_ref().is_some_and(|a| {
            a.interrupt
                .as_ref()
                .is_some_and(|e| e.evaluate(&self.states, now))
        });
        let mut best: Option<Execution> = None;
        for (index, scene) in self.script.scenes.iter().enumerate() {
            if self.last_scene[index].is_some_and(|last| now - last < scene.interval) {
                continue;
            }
            if scene.triggers.is_empty() {
                if self.active.is_some() && !interrupted {
                    continue;
                }
            } else {
                if !scene.triggers.iter().any(|s| triggers.contains(s.as_str())) {
                    continue;
                }
                if let Some(active) = &self.active {
                    if active.scene == index {
                        continue;
                    }
                    if !interrupted
                        && active
                            .priority
                            .is_some_and(|p| scene.priority.is_none_or(|n| n <= p))
                    {
                        continue;
                    }
                }
            }
            if let Some((handler, interrupt)) = find(&scene.handlers, &self.states, now, None) {
                if best.as_ref().is_some_and(|b| {
                    b.priority
                        .is_some_and(|p| scene.priority.is_none_or(|n| n <= p))
                }) {
                    continue;
                }
                best = Some(Execution {
                    generation: self.generation + 1,
                    scene: index,
                    priority: scene.priority,
                    label: handler.label.clone(),
                    source: handler.source.clone(),
                    actions: handler.actions.clone(),
                    interrupt,
                });
            }
        }
        if let Some(execution) = best {
            let replace = self.active.is_some();
            self.generation += 1;
            self.last_scene[execution.scene] = Some(now);
            self.active = Some(execution.clone());
            return Ok(if replace {
                Decision::Replace(execution)
            } else {
                Decision::Start(execution)
            });
        }
        if interrupted {
            self.active = None;
            self.generation += 1;
            return Ok(Decision::Interrupt);
        }
        Ok(Decision::Idle)
    }
}

fn find<'a>(
    handlers: &'a [Handler],
    states: &StateBook,
    now: f64,
    parent: Option<Expression>,
) -> Option<(&'a Handler, Option<Expression>)> {
    for handler in handlers {
        if !handler.condition.evaluate(states, now) {
            continue;
        }
        let interrupt = match (&parent, &handler.interrupt) {
            (Some(p), Some(c)) => Some(Expression::Or(Box::new(p.clone()), Box::new(c.clone()))),
            (Some(p), None) => Some(p.clone()),
            (None, value) => value.clone(),
        };
        if interrupt.as_ref().is_some_and(|e| e.evaluate(states, now)) {
            continue;
        }
        if !handler.children.is_empty() {
            if let Some(found) = find(&handler.children, states, now, interrupt) {
                return Some(found);
            }
        } else {
            return Some((handler, interrupt));
        }
    }
    None
}
