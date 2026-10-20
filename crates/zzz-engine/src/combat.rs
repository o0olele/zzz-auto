use anyhow::{Context, Result, ensure};
use maa_adapter::{
    NativeSession,
    combat_vision::{CombatObservation, CombatRecognizer},
};
use od_core::{
    RunState,
    combat::{CombatConfig, CombatProgress},
    combat_inputs::InputLedger,
    conditional::{Action, ButtonWay, Decision, Scheduler, Script, StateUpdate},
    pipeline::{Interrupt, PipelineControl, PipelineRequest},
};
use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub fn script_catalog(
    bundle: &maa_adapter::combat_vision::CombatBundle,
    data: &std::path::Path,
) -> Result<od_core::combat::CombatScriptCatalog> {
    use sha2::Digest;
    let (imported, battle, dodge) = od_core::combat_scripts::catalog(data)?;
    let mut digest = sha2::Sha256::new();
    digest.update(std::fs::read(bundle.root.join("combat-manifest.json"))?);
    digest.update(imported.as_deref().unwrap_or("").as_bytes());
    Ok(od_core::combat::CombatScriptCatalog {
        revision: hex::encode(digest.finalize()),
        imported,
        battle: bundle
            .scripts
            .iter()
            .cloned()
            .chain(battle)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        dodge: bundle
            .dodge_scripts
            .iter()
            .cloned()
            .chain(dodge)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
    })
}

enum Step {
    Action(Action),
    Button(String, bool),
    Delay(f64),
    ClearInputs,
    ReleaseTurn,
}

pub struct CombatRun {
    pub id: Uuid,
    pub progress: CombatProgress,
    pub message: String,
    control: PipelineControl,
    scheduler: Scheduler,
    elapsed: f64,
    sequence: Option<u64>,
    random: u64,
}
impl CombatRun {
    pub fn new(id: Uuid, config: CombatConfig, script: Script) -> Result<Self> {
        config.validate().map_err(anyhow::Error::msg)?;
        config.validate_script(&script)?;
        let random = u64::from_le_bytes(id.as_bytes()[..8].try_into()?).max(1);
        Ok(Self {
            id,
            progress: CombatProgress::new(config),
            message: "战斗准备中".into(),
            control: PipelineControl::default(),
            scheduler: Scheduler::new(script),
            elapsed: 0.0,
            sequence: None,
            random,
        })
    }
    pub fn control(&self) -> PipelineRequest {
        self.control.requester()
    }
    pub fn stopping(&self) -> bool {
        self.control.requested() == Some(Interrupt::Stop)
    }
    pub fn resume(&mut self) -> Result<()> {
        ensure!(
            self.progress.state == RunState::Paused
                && self.progress.inputs_released
                && self.control.resume(),
            "战斗尚未确认暂停或已请求停止"
        );
        self.scheduler.resume();
        self.sequence = None;
        self.progress.state = RunState::Running;
        Ok(())
    }
    fn intervals(&mut self) -> Result<BTreeMap<String, f64>> {
        let mut result = BTreeMap::new();
        for (name, default) in [
            ("check_dodge_interval", 0.02),
            ("check_agent_interval", 0.5),
            ("check_chain_interval", 1.0),
            ("check_quick_interval", 0.5),
            ("target_lock_interval", 1.0),
            ("auto_lock_interval", 1.0),
            ("auto_turn_interval", 2.0),
        ] {
            let [lo, hi] = self.scheduler.script.interval_range(name, default)?;
            self.random ^= self.random << 13;
            self.random ^= self.random >> 7;
            self.random ^= self.random << 17;
            result.insert(
                name.into(),
                lo + (hi - lo) * (self.random >> 11) as f64 / ((1u64 << 53) as f64),
            );
        }
        Ok(result)
    }
    pub fn execute(
        &mut self,
        native: &mut NativeSession,
        recognizer: &mut CombatRecognizer,
        allow_audio: bool,
        mut publish: impl FnMut(CombatProgress),
    ) -> Result<()> {
        let started = Instant::now();
        let result = self.execute_inner(native, recognizer, allow_audio, &mut publish);
        self.elapsed += started.elapsed().as_secs_f64();
        let released = native.release_inputs();
        self.progress.inputs_released = released.is_ok();
        self.scheduler.pause();
        self.sequence = None;
        if let Err(error) = result.and(released) {
            self.control.finish();
            self.progress.state = RunState::Failed;
            self.message = format!("战斗失败：{error:#}");
            publish(self.progress.clone());
            return Err(error);
        }
        match self.control.requested() {
            Some(Interrupt::Pause) if self.control.acknowledge_pause() => {
                self.progress.state = RunState::Paused;
                self.message = "战斗已暂停，输入已释放；继续后重新识别".into();
            }
            _ => {
                self.control.finish();
                self.progress.state = RunState::Stopped;
                self.message = "战斗已停止，输入已释放".into();
            }
        }
        publish(self.progress.clone());
        Ok(())
    }
    fn execute_inner(
        &mut self,
        native: &mut NativeSession,
        recognizer: &mut CombatRecognizer,
        allow_audio: bool,
        publish: &mut impl FnMut(CombatProgress),
    ) -> Result<()> {
        recognizer.reset();
        for prefix in [
            "前台-",
            "后台-1-",
            "后台-2-",
            "快速支援-",
            "连携技-1-",
            "连携技-2-",
            "切换角色-",
        ] {
            let names: Vec<_> = recognizer
                .bundle
                .spec
                .agents
                .iter()
                .map(|agent| format!("{prefix}{}", agent.name))
                .chain((prefix.starts_with("连携技-")).then(|| format!("{prefix}邦布")))
                .collect();
            self.scheduler.states.add_mutex(&names);
            let kinds: std::collections::BTreeSet<_> = recognizer
                .bundle
                .spec
                .agents
                .iter()
                .map(|agent| format!("{prefix}{}", agent.kind))
                .collect();
            self.scheduler
                .states
                .add_mutex(&kinds.into_iter().collect::<Vec<_>>());
        }
        for agent in &recognizer.bundle.spec.agents {
            let name = &agent.name;
            self.scheduler.states.add_exclusions(
                format!("前台-{name}"),
                [
                    format!("后台-{name}"),
                    format!("后台-1-{name}"),
                    format!("后台-2-{name}"),
                ],
            );
            self.scheduler
                .states
                .add_exclusions(format!("后台-{name}"), [format!("前台-{name}")]);
            for position in 1..=2 {
                self.scheduler.states.add_exclusions(
                    format!("后台-{position}-{name}"),
                    [
                        format!("前台-{name}"),
                        format!("后台-{}-{name}", 3 - position),
                    ],
                );
            }
        }
        let recognition_dodge_only = self.scheduler.script.states.iter().all(|name| {
            name.starts_with("闪避识别-")
                || name.starts_with("自定义-")
                || name.starts_with("按键-")
                || name == "按键可用-普通攻击"
        });
        let flash_enabled = self
            .scheduler
            .script
            .states
            .iter()
            .any(|name| name == "闪避识别-红光" || name == "闪避识别-黄光");
        let cancel = self.control.cancellation();
        let delay = Instant::now();
        while delay.elapsed() < Duration::from_millis(self.progress.config.start_delay_ms as u64) {
            if self.control.requested().is_some() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut audio = if self.progress.config.audio {
            ensure!(allow_audio, "离线回放不能启动真实输出回录");
            let template = od_core::audio_dodge::read_wav(&std::fs::read(
                recognizer.bundle.root.join("audio/template_1.wav"),
            )?)?;
            Some((
                od_platform::audio::AudioLoopback::start()?,
                od_core::audio_dodge::AudioDodge::new(
                    &template,
                    self.progress.config.audio_threshold_milli as f64 / 1000.0,
                )?,
            ))
        } else {
            None
        };
        let started = Instant::now();
        let mut ledger = InputLedger::new(self.progress.config.mode.is_gamepad());
        let mut queue = VecDeque::new();
        let mut until = None;
        let mut next_frame = 0.0;
        let mut recognition_seconds = 0.0_f64;
        let mut last_fresh = None;
        let mut last_publish = 0.0;
        let mut last_lock = 0.0;
        let mut last_turn = 0.0;
        let mut last_audio = -1.0;
        let mut last_ultimate = -2.0;
        let mut pending_states = Vec::new();
        let mut observation = CombatObservation::default();
        self.progress.inputs_released = false;
        publish(self.progress.clone());
        loop {
            if self.control.requested().is_some() {
                return Ok(());
            }
            let mut now = self.elapsed + started.elapsed().as_secs_f64();
            ensure!(
                now * 1000.0 < self.progress.config.timeout_ms as f64,
                "战斗运行达到超时上限"
            );
            // 已排入的定时松键优先于可能阻塞的截图和识别。
            let release_due = matches!(queue.front(), Some(Step::Delay(_)))
                && matches!(
                    queue.get(1),
                    Some(Step::Button(_, false) | Step::ReleaseTurn)
                )
                || (matches!(
                    queue.front(),
                    Some(Step::Button(_, false) | Step::ReleaseTurn)
                ) && until
                    .is_none_or(|deadline| deadline - now <= recognition_seconds.max(0.005)));
            if now >= next_frame && !release_due {
                let recognition_started = Instant::now();
                let frame_at = now;
                let Some(frame) = native.combat_frame(&cancel)? else {
                    return Ok(());
                };
                self.progress.frames += 1;
                let mut intervals = self.intervals()?;
                intervals.insert("flash_enabled".into(), f64::from(flash_enabled));
                let fresh = recognizer.observe(
                    native,
                    &frame,
                    frame_at,
                    &intervals,
                    recognition_dodge_only,
                    &cancel,
                )?;
                if self.control.requested().is_some() {
                    return Ok(());
                }
                self.progress.frame_age_ms = frame.age_ms();
                now = self.elapsed + started.elapsed().as_secs_f64();
                recognition_seconds = recognition_started.elapsed().as_secs_f64();
                next_frame = now + self.progress.config.frame_interval_ms as f64 / 1000.0;
                if frame.age_ms() > self.progress.config.max_frame_age_ms {
                    self.progress.stale_frames += 1;
                    native.release_inputs()?;
                    ledger.clear();
                    queue.clear();
                    until = None;
                    pending_states.clear();
                    self.scheduler.pause();
                    self.scheduler.resume();
                    self.sequence = None;
                    last_fresh = None;
                    self.progress.in_battle = false;
                } else {
                    if self.progress.config.auto_ultimate
                        && now - last_ultimate >= 2.0
                        && fresh
                            .states
                            .iter()
                            .any(|s| s.name == "按键可用-终结技" && !s.clear)
                    {
                        pending_states.push(StateUpdate {
                            name: "自定义-终结技被强制释放".into(),
                            at: frame_at,
                            value: None,
                            add: None,
                            time_add: None,
                            clear: false,
                        });
                        last_ultimate = now;
                    }
                    pending_states.extend(fresh.states.iter().cloned());
                    self.progress.in_battle = fresh.in_battle;
                    if !fresh.team.is_empty() {
                        self.progress.team = fresh.team.clone();
                    }
                    last_fresh = (fresh.in_battle
                        || fresh.states.iter().any(|s| {
                            (s.name == "连携技-准备"
                                || s.name.starts_with("连携技-1-")
                                || s.name.starts_with("连携技-2-"))
                                && !s.clear
                        }))
                    .then_some(frame_at);
                    observation = fresh;
                }
            }
            if now - last_audio >= 0.02
                && let Some((capture, detector)) = &mut audio
            {
                last_audio = now;
                let (samples, dropped) = capture.drain()?;
                if dropped > 0 {
                    detector.reset();
                }
                detector.append(&samples)?;
                let (correlation, hit) = detector.check(now)?;
                self.progress.audio_correlation = Some(correlation);
                if hit && self.progress.in_battle {
                    pending_states.push(StateUpdate {
                        name: "闪避识别-声音".into(),
                        at: now,
                        value: None,
                        add: None,
                        time_add: None,
                        clear: false,
                    });
                }
            }
            let fresh = last_fresh.is_some_and(|at| {
                (now - at) * 1000.0 <= self.progress.config.max_frame_age_ms as f64
            });
            if !fresh {
                if self.sequence.is_some() || !queue.is_empty() || !ledger.is_empty() {
                    native.release_inputs()?;
                    ledger.clear();
                    queue.clear();
                    until = None;
                }
                self.scheduler.pause();
                self.scheduler.resume();
                self.sequence = None;
                pending_states.clear();
            } else {
                let decision = self
                    .scheduler
                    .tick(now, &std::mem::take(&mut pending_states))?;
                let replacing = matches!(decision, Decision::Replace(_));
                match decision {
                    Decision::Start(execution) | Decision::Replace(execution) => {
                        if replacing {
                            self.progress.preemptions += 1;
                            native.release_inputs()?;
                            ledger.clear();
                        }
                        queue.clear();
                        until = None;
                        self.progress.handler = if execution.label.is_empty() {
                            execution.source
                        } else {
                            execution.label
                        };
                        self.sequence = Some(execution.generation);
                        queue.extend(execution.actions.into_iter().map(Step::Action));
                    }
                    Decision::Interrupt => {
                        native.release_inputs()?;
                        ledger.clear();
                        queue.clear();
                        until = None;
                        self.sequence = None;
                        self.progress.preemptions += 1;
                    }
                    Decision::Idle => {}
                }
                if until.is_some_and(|deadline| now >= deadline) {
                    until = None;
                }
                if queue.is_empty() && until.is_none() {
                    if let Some(generation) = self.sequence.take() {
                        self.scheduler.complete(generation)?;
                    }
                    if self.progress.in_battle {
                        let intervals = self.intervals()?;
                        if intervals["auto_lock_interval"] > 0.0
                            && now - last_lock >= intervals["auto_lock_interval"]
                        {
                            queue.push_back(Step::Action(Action::Button {
                                name: "锁定敌人".into(),
                                way: ButtonWay::Tap,
                                press: None,
                            }));
                            last_lock = now;
                        }
                        if intervals["auto_turn_interval"] > 0.0
                            && now - last_turn >= intervals["auto_turn_interval"]
                        {
                            queue.push_back(Step::Action(Action::Turn { distance: 100.0 }));
                            last_turn = now;
                        }
                    }
                }
                if until.is_none()
                    && let Some(step) = queue.pop_front()
                {
                    match step {
                        Step::Delay(seconds) => until = Some(now + seconds),
                        Step::ClearInputs => {
                            native.release_inputs()?;
                            ledger.clear();
                        }
                        Step::ReleaseTurn => {
                            for (action, down) in ledger.set("__turn", Vec::new(), false) {
                                if !native.combat_input(action, down, &cancel)? {
                                    return Ok(());
                                }
                            }
                        }
                        Step::Button(name, pressed) => {
                            for (action, down) in ledger.set(
                                &name,
                                self.progress
                                    .config
                                    .binding(&name)
                                    .map_err(anyhow::Error::msg)?,
                                pressed,
                            ) {
                                if !native.combat_input(action, down, &cancel)? {
                                    return Ok(());
                                }
                            }
                            pending_states.push(StateUpdate {
                                name: format!(
                                    "按键-{name}-{}",
                                    if pressed { "按下" } else { "松开" }
                                ),
                                at: self.elapsed + started.elapsed().as_secs_f64(),
                                value: None,
                                add: None,
                                time_add: None,
                                clear: false,
                            });
                            if pressed {
                                pending_states.push(StateUpdate {
                                    name: format!("按键-{name}"),
                                    at: now,
                                    value: None,
                                    add: None,
                                    time_add: None,
                                    clear: false,
                                });
                                let team = &mut self.progress.team;
                                if team.len() > 1 {
                                    if name == "切换角色-下一个" {
                                        team.rotate_left(1);
                                    } else if name == "切换角色-上一个" {
                                        team.rotate_right(1);
                                    } else if name == "连携技-左" || name == "连携技-右" {
                                        let index = usize::from(name == "连携技-右");
                                        if let Some(Some(target)) = observation.chain.get(index)
                                            && let Some(position) =
                                                team.iter().position(|n| n == target)
                                        {
                                            team.rotate_left(position);
                                        }
                                    }
                                    if name.starts_with("切换角色-") || name.starts_with("连携技-")
                                    {
                                        recognizer.refresh_agents();
                                        for (position, name) in team.iter().enumerate() {
                                            if let Some(agent) = recognizer
                                                .bundle
                                                .spec
                                                .agents
                                                .iter()
                                                .find(|a| &a.name == name)
                                            {
                                                let prefix = if position == 0 {
                                                    "前台-".into()
                                                } else {
                                                    format!("后台-{position}-")
                                                };
                                                for suffix in [&agent.name, &agent.kind] {
                                                    pending_states.push(StateUpdate {
                                                        name: format!("{prefix}{suffix}"),
                                                        at: now,
                                                        value: None,
                                                        add: None,
                                                        time_add: None,
                                                        clear: false,
                                                    });
                                                }
                                                if position == 0 {
                                                    pending_states.push(StateUpdate {
                                                        name: format!("切换角色-{}", agent.name),
                                                        at: now,
                                                        value: None,
                                                        add: None,
                                                        time_add: None,
                                                        clear: false,
                                                    });
                                                    pending_states.push(StateUpdate {
                                                        name: format!("切换角色-{}", agent.kind),
                                                        at: now,
                                                        value: None,
                                                        add: None,
                                                        time_add: None,
                                                        clear: false,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Step::Action(action) => {
                            self.progress.actions += 1;
                            self.progress.action =
                                format!("{action:?}").chars().take(512).collect();
                            match action {
                                Action::Wait { seconds } => until = Some(now + seconds),
                                Action::Button { name, way, press } => {
                                    let switching =
                                        name.starts_with("切换角色-") && way != ButtonWay::Up;
                                    let mut steps = Vec::new();
                                    if way == ButtonWay::Up {
                                        steps.push(Step::Button(name.clone(), false));
                                    } else {
                                        steps.push(Step::Button(name.clone(), true));
                                        if way == ButtonWay::Tap || press.is_some() {
                                            steps.push(Step::Delay(press.unwrap_or(
                                                self.progress.config.tap_ms as f64 / 1000.0,
                                            )));
                                            steps.push(Step::Button(name.clone(), false));
                                        }
                                    }
                                    for step in steps.into_iter().rev() {
                                        queue.push_front(step);
                                    }
                                    if switching {
                                        queue.push_front(Step::ClearInputs);
                                    }
                                }
                                Action::SetState {
                                    names,
                                    seconds,
                                    seconds_add,
                                    value,
                                    add,
                                } => {
                                    for name in names {
                                        pending_states.push(StateUpdate {
                                            name,
                                            at: now + seconds,
                                            value,
                                            add,
                                            time_add: Some(seconds_add),
                                            clear: false,
                                        });
                                    }
                                }
                                Action::ClearState { names } => {
                                    for name in names {
                                        pending_states.push(StateUpdate {
                                            name,
                                            at: now,
                                            value: None,
                                            add: None,
                                            time_add: None,
                                            clear: true,
                                        });
                                    }
                                }
                                Action::SwitchAgent { name } => {
                                    let index = self
                                        .progress
                                        .team
                                        .iter()
                                        .position(|n| n == &name)
                                        .context(format!("切换目标 {name} 不在已识别队伍中"))?;
                                    if index > 0 {
                                        queue.push_front(Step::Action(Action::Button {
                                            name: if index == 1 {
                                                "切换角色-下一个"
                                            } else {
                                                "切换角色-上一个"
                                            }
                                            .into(),
                                            way: ButtonWay::Tap,
                                            press: None,
                                        }));
                                    }
                                }
                                Action::QuickAssist => {
                                    if let Some(name) = &observation.quick_assist {
                                        queue.push_front(Step::Action(Action::SwitchAgent {
                                            name: name.clone(),
                                        }));
                                    }
                                }
                                Action::Turn { distance } => {
                                    let distance = distance
                                        * self.progress.config.turn_sensitivity_milli as f64
                                        / 1000.0;
                                    ensure!(
                                        distance.is_finite() && distance.abs() <= 10000.0,
                                        "转向位移超出范围"
                                    );
                                    if self.progress.config.mode.is_gamepad() {
                                        let action = od_core::InputProbeAction::Stick {
                                            stick: od_core::InputSide::Right,
                                            x: if distance < 0.0 {
                                                -self.progress.config.gamepad_turn_speed
                                            } else {
                                                self.progress.config.gamepad_turn_speed
                                            },
                                            y: 0,
                                        };
                                        if distance != 0.0 {
                                            for (action, down) in
                                                ledger.set("__turn", vec![action], true)
                                            {
                                                if !native.combat_input(action, down, &cancel)? {
                                                    return Ok(());
                                                }
                                            }
                                            queue.push_front(Step::ReleaseTurn);
                                            queue.push_front(Step::Delay(distance.abs() / 1000.0));
                                        }
                                    } else if distance.round() as i32 != 0 {
                                        native.combat_turn(distance.round() as i32, &cancel)?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if now - last_publish >= 0.1 {
                self.progress.states = self
                    .scheduler
                    .states
                    .values()
                    .iter()
                    .take(128)
                    .map(|(n, v)| (n.clone(), v.at))
                    .collect();
                publish(self.progress.clone());
                last_publish = now;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
