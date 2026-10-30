//! 迷失之地出战守卫：新帧、明确按钮、矩阵完整身份及输入释放。
use crate::{
    NativeSession, RecognitionImage,
    suibian_adventure::{Reader, words},
};
use anyhow::{Result, ensure};
use od_core::{
    InputProbeAction, MouseButton,
    lost_void_deploy::{
        DeployAction as A, DeployPage as P, DeploymentParty, LostVoidDeployFlow,
        LostVoidDeployView as V,
    },
    lost_void_matrix::LostVoidMatrixFlow,
    lost_void_scene::LostVoidScreen as S,
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployInputEnd {
    Posted,
    NotPosted,
    Changed,
    TimedOut,
}
struct Sight {
    view: V,
    point: Option<[i32; 2]>,
}
impl NativeSession {
    pub fn require_lost_void_preparation_roots(
        &self,
        scene: &std::path::Path,
        daily: Option<&std::path::Path>,
        combat: &std::path::Path,
    ) -> Result<()> {
        ensure!(
            self.lost_void_scene_bundle.as_deref() == Some(scene)
                && self.combat_bundle.as_deref() == Some(combat)
                && daily.is_none_or(|d| self.daily_bundle.as_deref() == Some(d)),
            "迷失之地当前控制器的资源与首次输入前冻结的路径不一致"
        );
        Ok(())
    }
    fn lost_void_deploy_sight(
        &self,
        frame: &RecognitionImage,
        flow: &LostVoidDeployFlow,
        matrix: Option<&LostVoidMatrixFlow>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<Sight>> {
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let r = Reader {
            session: self,
            frame,
            cancel,
            started,
            timeout,
            pipeline: od_core::lost_void_scene::pipeline(),
        };
        let read = |s: &str| words(&r.read(&format!("LostVoid.{s}"), None)?);
        let unique = |ws: &[crate::suibian_adventure::Word], text: &str| {
            let found: Vec<_> = ws.iter().filter(|w| w.text == text).collect();
            (found.len() == 1).then(|| found[0].center())
        };
        let sight = |page, point| {
            Some(Sight {
                view: V {
                    page,
                    matrix: None,
                    member_count: None,
                },
                point,
            })
        };
        let capacity = read("DeployCapacity")?;
        if unique(&capacity, "驱动盘数量已达到可拥有上限").is_some() {
            return Ok(sight(P::InventoryFull, None));
        }
        let modal = read("DeployModal")?;
        let confirm = read("DeployConfirm")?;
        if ["恢复电量", "快捷使用", "获得"]
            .iter()
            .any(|s| unique(&modal, s).is_some())
        {
            return Ok(sight(P::Blocked, None));
        }
        // 与已有出战守卫相同的完整文案别名。实际 OCR 的两种裁剪会分别输出并 / 並。
        let level_button = |ws: &[crate::suibian_adventure::Word]| {
            let found: Vec<_> = ws
                .iter()
                .filter(|w| matches!(w.text.as_str(), "确定并出战" | "确定並出战"))
                .collect();
            (found.len() == 1).then(|| found[0].center())
        };
        if od_core::lost_void_deploy::short_party_notice(
            &modal.iter().map(|w| w.text.clone()).collect::<Vec<_>>(),
        ) {
            if level_button(&confirm).is_none()
                && level_button(&modal).is_none()
                && let Some((p, q)) = unique(&confirm, "确认").zip(unique(&modal, "确认"))
                && (p[0] - q[0]).abs() <= 20
                && (p[1] - q[1]).abs() <= 20
            {
                return Ok(sight(P::ShortParty, Some(p)));
            }
            return Ok(sight(P::Blocked, None));
        }
        if let Some((p, q)) = level_button(&confirm).zip(level_button(&modal))
            && (p[0] - q[0]).abs() <= 20
            && (p[1] - q[1]).abs() <= 20
        {
            return Ok(sight(P::LowLevel, Some(p)));
        }
        if confirm
            .iter()
            .any(|w| ["确认", "确定", "取消"].contains(&w.text.as_str()))
            || modal
                .iter()
                .any(|w| ["确认", "取消", "确定并出战", "确定並出战"].contains(&w.text.as_str()))
        {
            return Ok(sight(P::Blocked, None));
        }
        match &flow.request().party {
            DeploymentParty::Classic { .. } => {
                let deploy = read("DeployDeploy")?;
                let team = read("DeployTeam")?;
                if let Some(p) = unique(&deploy, "出战")
                    && unique(&team, "预备编队").is_some()
                {
                    let count = od_core::lost_void_deploy::observed_party_count(
                        &read("DeployPartyCount")?
                            .into_iter()
                            .map(|w| w.text)
                            .collect::<Vec<_>>(),
                    );
                    return Ok(Some(Sight {
                        view: V {
                            page: P::Classic,
                            matrix: None,
                            member_count: count,
                        },
                        point: Some(p),
                    }));
                }
            }
            DeploymentParty::Matrix { receipt } => {
                let m = matrix.ok_or_else(|| anyhow::anyhow!("矩阵出战缺少本次编队检查点"))?;
                ensure!(m.receipt() == Some(receipt), "矩阵出战编队检查点已变化");
                // 先确认本页固定开始按钮，再完整复验成员；输入守卫不重复扫描全场景。
                if unique(&read("MatrixStart")?, "开始挑战").is_some()
                    && let Some(v) = self.verify_lost_void_matrix_prepared(
                        frame,
                        m,
                        cancel,
                        timeout.saturating_sub(started.elapsed()),
                    )?
                    && v.screen == S::MatrixTeam
                    && !v.tutorial
                {
                    let p = unique(&read("MatrixStartEnabled")?, "开始挑战");
                    return Ok(Some(Sight {
                        view: V {
                            page: P::Matrix,
                            matrix: Some(v),
                            member_count: None,
                        },
                        point: p,
                    }));
                }
            }
        }
        let scene =
            self.lost_void_sight(frame, cancel, timeout.saturating_sub(started.elapsed()))?;
        let Some(scene) = scene else {
            return Ok(None);
        };
        match scene.observation.screen {
            S::World | S::Gear | S::Event | S::CommonChoice => {
                return Ok(sight(P::Inside(scene.observation.screen), None));
            }
            S::BattleFail | S::BattleResult => return Ok(sight(P::Failed, None)),
            S::Modal | S::TransportConfirm | S::GearConfirm => return Ok(sight(P::Blocked, None)),
            _ => {}
        }
        Ok(sight(P::Unknown, None))
    }
    pub fn inspect_lost_void_deploy(
        &self,
        frame: &RecognitionImage,
        flow: &LostVoidDeployFlow,
        matrix: Option<&LostVoidMatrixFlow>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        flow.validate().map_err(anyhow::Error::msg)?;
        self.require_lost_void_scene()?;
        Ok(self
            .lost_void_deploy_sight(frame, flow, matrix, cancel, timeout)?
            .map(|s| s.view))
    }
    pub fn read_lost_void_deploy(
        &self,
        flow: &LostVoidDeployFlow,
        matrix: Option<&LostVoidMatrixFlow>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<V>> {
        let started = Instant::now();
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let view = self.inspect_lost_void_deploy(
            &frame.image,
            flow,
            matrix,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        if frame.age() > Duration::from_millis(1500)
            || cancel.load(Ordering::Acquire)
            || started.elapsed() >= timeout
        {
            return Ok(None);
        }
        Ok(view)
    }
    pub fn act_lost_void_deploy(
        &mut self,
        flow: &LostVoidDeployFlow,
        action: A,
        matrix: Option<&LostVoidMatrixFlow>,
        cancel: &AtomicBool,
        timeout: Duration,
        game: bool,
    ) -> Result<DeployInputEnd> {
        let started = Instant::now();
        let result = (|| -> Result<DeployInputEnd> {
            flow.validate().map_err(anyhow::Error::msg)?;
            self.check_ether_synthesis_target(game)?;
            self.require_lost_void_scene()?;
            if cancel.load(Ordering::Acquire) {
                return Ok(DeployInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(DeployInputEnd::TimedOut);
            }
            if game {
                self.prepare_input_target(
                    od_core::mail::MailConfig::recovery_inputs()[1],
                    cancel,
                    &mut None,
                )?;
            }
            let Some(frame) = self.navigation_frame(cancel)? else {
                return Ok(DeployInputEnd::NotPosted);
            };
            let s = self.lost_void_deploy_sight(
                &frame.image,
                flow,
                matrix,
                cancel,
                timeout.saturating_sub(started.elapsed()),
            )?;
            if cancel.load(Ordering::Acquire) {
                return Ok(DeployInputEnd::NotPosted);
            }
            if started.elapsed() >= timeout {
                return Ok(DeployInputEnd::TimedOut);
            }
            if frame.age() > Duration::from_millis(1500) {
                return Ok(DeployInputEnd::Changed);
            }
            let Some(s) = s.filter(|s| flow.prepared_allows(action, &s.view)) else {
                return Ok(DeployInputEnd::Changed);
            };
            let Some([x, y]) = s.point else {
                return Ok(DeployInputEnd::Changed);
            };
            let input = InputProbeAction::MouseButton {
                button: MouseButton::Left,
                x,
                y,
            };
            if !self.combat_input(input, true, cancel)? {
                return Ok(DeployInputEnd::NotPosted);
            }
            let held = Instant::now();
            while held.elapsed() < Duration::from_millis(100)
                && !cancel.load(Ordering::Acquire)
                && started.elapsed() < timeout
            {
                std::thread::sleep(Duration::from_millis(2));
            }
            self.combat_input(input, false, &AtomicBool::new(false))?;
            Ok(DeployInputEnd::Posted)
        })();
        let released = self.release_inputs();
        result.and_then(|end| released.map(|_| end))
    }
}
