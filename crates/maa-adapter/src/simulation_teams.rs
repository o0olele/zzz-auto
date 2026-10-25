//! 出战页与选队列表共用同一帧；只打开列表，不投递出战。
use crate::{
    NativeSession, RecognitionImage,
    predefined_teams::TeamPageMode,
    simulation::CardsInputEnd,
    suibian_adventure::{Reader, point, words},
};
use anyhow::{Result, ensure};
use od_core::{
    predefined_teams::TeamConfig,
    simulation_teams::{TeamsFlow, TeamsView},
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

impl NativeSession {
    pub fn require_simulation_teams(&self) -> Result<()> {
        self.require_simulation()?;
        crate::validate_basic_daily_bundle(
            self.daily_bundle.as_deref().unwrap_or(&self.resource_root),
        )?;
        Ok(())
    }
    fn simulation_teams_sight(
        &self,
        frame: &RecognitionImage,
        teams: &TeamConfig,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<(TeamsView, Option<[i32; 2]>)> {
        let started = Instant::now();
        let r = Reader {
            session: self,
            frame,
            cancel,
            started,
            timeout,
            pipeline: od_core::simulation_pipeline::pipeline(),
        };
        let modal = words(&r.read("Simulation.Area.Modal", None)?)?;
        if modal.iter().any(|w| {
            [
                "恢复电量",
                "快捷使用",
                "获得",
                "确认",
                "取消",
                "确定并出战",
                "确定並出战",
            ]
            .contains(&w.text.as_str())
        }) {
            return Ok((TeamsView::Blocked, None));
        }
        let deploy = words(&r.read("Simulation.Area.Deploy", None)?)?;
        let team = words(&r.read("Simulation.Area.Team", None)?)?;
        let open = point(&team, "预备编队", [368, 1002, 563, 1057]);
        if point(&deploy, "出战", [1638, 1002, 1832, 1052]).is_some() && open.is_some() {
            return Ok((TeamsView::Deployment, open));
        }
        let Some(page) = self.inspect_team_selection(
            frame,
            teams,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?
        else {
            return Ok((TeamsView::Unknown, None));
        };
        Ok((
            match page.mode {
                TeamPageMode::Selection => TeamsView::Selection,
                TeamPageMode::Editing => TeamsView::Blocked,
                _ => TeamsView::Unknown,
            },
            None,
        ))
    }
    pub fn read_simulation_teams(
        &self,
        teams: &TeamConfig,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<TeamsView>> {
        let started = Instant::now();
        self.require_simulation_teams()?;
        teams.validate().map_err(anyhow::Error::msg)?;
        if cancel.load(Ordering::Acquire) || started.elapsed() >= timeout {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let sight = self.simulation_teams_sight(
            &frame.image,
            teams,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        );
        if cancel.load(Ordering::Acquire)
            || started.elapsed() >= timeout
            || frame.age() > Duration::from_millis(1500)
        {
            return Ok(None);
        }
        sight.map(|(view, _)| Some(view))
    }
    pub fn act_simulation_teams_open(
        &mut self,
        flow: &TeamsFlow,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<CardsInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<CardsInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            ensure!(flow.allows_open(), "打开编队列表前须保存意图");
            ensure!(
                self.game_window.is_some() == game
                    && self.input_mode == Some(crate::InputMode::Foreground),
                "模拟室选队控制器模式不匹配"
            );
            self.require_simulation_teams()?;
            if cancel.load(Ordering::Acquire) {
                return Ok(CardsInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(CardsInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(CardsInputEnd::NotPosted);
            };
            let sight = self.simulation_teams_sight(
                &frame.image,
                &flow.request().teams,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            );
            if cancel.load(Ordering::Acquire) {
                return Ok(CardsInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(CardsInputEnd::TimedOut);
            }
            let (view, open) = sight?;
            if view != TeamsView::Deployment || frame.age() > Duration::from_millis(1500) {
                return Ok(CardsInputEnd::Changed);
            }
            let Some([x, y]) = open else {
                return Ok(CardsInputEnd::Changed);
            };
            let input = od_core::InputProbeAction::MouseButton {
                button: od_core::MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(CardsInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(70)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.combat_input(input, false, &AtomicBool::new(false))?;
            Ok(CardsInputEnd::Posted)
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}
