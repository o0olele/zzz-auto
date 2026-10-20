use crate::{InputPipelineConfig, InputPipelineResult, NativeSession, SDK_VERSION};
use anyhow::{Result, ensure};
use maa_framework::{MaaStatus, job::Job};
pub use od_core::{PipelineEnd, PipelineProbeResult};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

// 由可信调用方声明全部路径可能持有的输入，不从可丢弃的展示事件推断。
#[derive(Debug, Clone, Default)]
pub struct PipelineInputPlan {
    pub keys: BTreeSet<i32>,
    pub contacts: BTreeSet<i32>,
}

pub const INPUT_PIPELINE_ENTRY: &str = "Prototype.Input.Down";

#[derive(Debug, Clone, Serialize)]
pub struct PipelineNodeResult {
    pub name: String,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineExecution {
    pub report: PipelineProbeResult,
    pub task_id: Option<i64>,
    pub nodes: Vec<PipelineNodeResult>,
}

#[derive(Debug, Clone, Copy)]
pub enum MailStep {
    Navigate,
    Inspect,
    Claim,
    AfterClaim,
    Confirm,
    AfterConfirm,
    ReturnList,
    ReturnWorld,
}

impl MailStep {
    pub fn entry(self) -> &'static str {
        match self {
            Self::Navigate => "Mail.Navigate",
            Self::Inspect => "Mail.Inspect",
            Self::Claim => "Mail.Claim",
            Self::AfterClaim => "Mail.AfterClaim",
            Self::Confirm => "Mail.Confirm",
            Self::AfterConfirm => "Mail.AfterConfirm",
            Self::ReturnList => "Mail.ReturnList",
            Self::ReturnWorld => "Mail.ReturnWorld",
        }
    }
}

impl NativeSession {
    // 任意入口只供离线验证；游戏仅开放由受限配置生成的诊断流程。
    pub fn probe_fixture_pipeline(
        &mut self,
        entry: &str,
        inputs: &PipelineInputPlan,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<PipelineProbeResult> {
        ensure!(
            self.game_window.is_none(),
            "离线 Pipeline 探针不能使用游戏控制器"
        );
        self.run_pipeline(entry, inputs, cancel, timeout)
    }

    pub(crate) fn wait_tasker_idle(&self) {
        // 任务状态可能先于 Tasker 收尾更新，不能只检查原任务的终态。
        while self.tasker.running() {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(crate) fn stop_pipeline(&self) -> Result<MaaStatus> {
        let id = self.tasker.post_stop()?;
        ensure!(id > 0, "Maa 拒绝 Pipeline 停止请求");
        let status = Job::for_tasker(&self.tasker, id).wait();
        self.wait_tasker_idle();
        ensure!(status.is_success(), "Maa Pipeline 停止失败：{status}");
        Ok(status)
    }

    pub(crate) fn run_pipeline(
        &mut self,
        entry: &str,
        inputs: &PipelineInputPlan,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<PipelineProbeResult> {
        self.run_pipeline_override(entry, inputs, cancel, timeout, "{}")
    }

    fn run_pipeline_override(
        &mut self,
        entry: &str,
        inputs: &PipelineInputPlan,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
        pipeline: &str,
    ) -> Result<PipelineProbeResult> {
        Ok(self
            .run_pipeline_detailed(entry, inputs, cancel, timeout, pipeline, false)?
            .report)
    }

    // 固定图入口与游戏入口分开校验，避免离线测试误用真实游戏控制器。
    fn validate_mail_fixture(&self) -> Result<()> {
        ensure!(self.game_window.is_none(), "邮件固定图入口不接受游戏控制器");
        ensure!(
            self.input_mode == Some(crate::InputMode::Foreground),
            "邮件原型使用前台键鼠语义"
        );
        Ok(())
    }

    pub fn prepare_fixture_mail(&mut self) -> Result<()> {
        self.validate_mail_fixture()?;
        self.release_inputs()
    }

    pub fn prepare_game_mail(&mut self, cancel: Arc<AtomicBool>, delay_ms: u32) -> Result<()> {
        ensure!(self.game_window.is_some(), "邮件实机入口需要游戏控制器");
        ensure!(
            self.input_mode == Some(crate::InputMode::Foreground),
            "邮件首版只支持前台键鼠"
        );
        ensure!(delay_ms <= 10_000, "邮件延时超出限制");
        self.release_inputs()?;
        crate::input::wait_delay(Duration::from_millis(delay_ms.into()), &cancel, &mut None);
        Ok(())
    }

    pub fn game_mail_step(
        &mut self,
        step: MailStep,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<PipelineExecution> {
        ensure!(self.game_window.is_some(), "邮件实机入口需要游戏控制器");
        ensure!(
            self.input_mode == Some(crate::InputMode::Foreground),
            "邮件首版只支持前台键鼠"
        );
        let config = od_core::mail::MailConfig::recovery_inputs()[1];
        self.prepare_input_target(config, &cancel, &mut None)?;
        self.run_pipeline_detailed(
            step.entry(),
            &PipelineInputPlan {
                keys: [18].into(),
                contacts: [0].into(),
            },
            cancel,
            timeout,
            "{}",
            true,
        )
    }

    pub fn fixture_mail_step(
        &mut self,
        step: MailStep,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<PipelineExecution> {
        self.validate_mail_fixture()?;
        self.run_pipeline_detailed(
            step.entry(),
            &PipelineInputPlan {
                keys: [18].into(),
                contacts: [0].into(),
            },
            cancel,
            timeout,
            "{}",
            true,
        )
    }

    pub(crate) fn run_pipeline_detailed(
        &mut self,
        entry: &str,
        inputs: &PipelineInputPlan,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
        pipeline: &str,
        collect_nodes: bool,
    ) -> Result<PipelineExecution> {
        ensure!(!entry.is_empty(), "Pipeline 入口不能为空");
        ensure!(!timeout.is_zero(), "Pipeline 超时必须大于零");
        ensure!(
            inputs.keys.iter().all(|key| *key >= 0)
                && inputs.contacts.iter().all(|contact| *contact >= 0),
            "Pipeline 输入清单不能包含负数"
        );
        ensure!(
            !self.tasker.running(),
            "Maa Tasker 仍在运行，不能启动另一个 Pipeline"
        );
        if self.input_cleanup_pending
            || !self.held_keys.is_empty()
            || !self.held_contacts.is_empty()
        {
            self.release_inputs()?;
        }
        let started = Instant::now();
        let mut stopped_at = None;
        let mut task_id = None;
        let mut report = PipelineProbeResult {
            sdk_version: SDK_VERSION.into(),
            entry: entry.into(),
            end: PipelineEnd::Succeeded,
            native_status: "NotStarted".into(),
            stop_status: None,
            enqueue_us: 0,
            elapsed_ms: 0,
            release_ms: 0,
            stop_ms: None,
            released: false,
        };
        let execution = (|| -> Result<()> {
            if cancel.load(Ordering::Acquire) {
                stopped_at = Some(Instant::now());
                report.end = PipelineEnd::Cancelled;
                return Ok(());
            }
            // 即使原生按下失败或 auto_up 未报告释放错误，也保留显式释放责任。
            self.held_keys.extend(&inputs.keys);
            self.held_contacts.extend(&inputs.contacts);
            self.input_cleanup_pending = true;
            self.pipeline_controller_pending = true;
            let enqueue_at = Instant::now();
            let job = self.tasker.post_task(entry, pipeline)?;
            task_id = Some(job.id);
            report.enqueue_us = enqueue_at.elapsed().as_micros() as u64;
            ensure!(job.id > 0, "Maa 拒绝 Pipeline 启动请求");
            loop {
                let status = job.status();
                ensure!(
                    status != MaaStatus::INVALID,
                    "Maa 返回无效 Pipeline 任务 ID"
                );
                if status.done() && !self.tasker.running() {
                    break;
                }
                if stopped_at.is_none() {
                    // 同时出现取消与超时时，优先保留主动停止的含义。
                    let cancelled = cancel.load(Ordering::Acquire);
                    if cancelled || started.elapsed() >= timeout {
                        stopped_at = Some(Instant::now());
                        report.end = if cancelled {
                            PipelineEnd::Cancelled
                        } else {
                            PipelineEnd::TimedOut
                        };
                        report.stop_status = Some(self.stop_pipeline()?.to_string());
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let status = job.wait();
            report.native_status = status.to_string();
            if stopped_at.is_none() {
                report.end = if status.is_success() {
                    PipelineEnd::Succeeded
                } else {
                    PipelineEnd::Failed
                };
            }
            Ok(())
        })();
        // SDK 接口错误也先收尾。原生阻塞时继续等待，不伪造已释放的终态。
        let settle = if execution.is_err() && self.tasker.running() {
            let result = self.stop_pipeline();
            self.wait_tasker_idle();
            result.map(|_| ())
        } else {
            Ok(())
        };
        let release_at = Instant::now();
        let cleanup = self.release_inputs_observed(&cancel, &mut stopped_at);
        report.release_ms = release_at.elapsed().as_millis() as u64;
        report.elapsed_ms = started.elapsed().as_millis() as u64;
        report.stop_ms = stopped_at.map(|at| at.elapsed().as_millis() as u64);
        if report.end == PipelineEnd::Succeeded && stopped_at.is_some() {
            report.end = PipelineEnd::Cancelled;
        }
        let mut errors: Vec<_> = [execution, settle, cleanup]
            .into_iter()
            .filter_map(Result::err)
            .map(|error| format!("{error:#}"))
            .collect();
        if !errors.is_empty() && report.end == PipelineEnd::Failed {
            errors.insert(0, format!("Maa Pipeline 失败：{}", report.native_status));
        }
        ensure!(errors.is_empty(), "{}", errors.join("；"));
        report.released = true;
        // 终态与检查点读取 SDK 保存的任务详情，不依赖可能丢弃的展示回调。
        let mut nodes = Vec::new();
        if let Some(id) = task_id.filter(|_| collect_nodes) {
            let detail = self
                .tasker
                .get_task_detail(id)?
                .ok_or_else(|| anyhow::anyhow!("Maa 未保存任务详情"))?;
            for node in detail.nodes {
                let node = node.ok_or_else(|| anyhow::anyhow!("Maa 未保存节点详情"))?;
                nodes.push(PipelineNodeResult {
                    name: node.node_name,
                    completed: node.completed,
                });
            }
        }
        Ok(PipelineExecution {
            report,
            task_id,
            nodes,
        })
    }

    pub fn input_pipeline(
        &mut self,
        config: InputPipelineConfig,
        cancel: Arc<AtomicBool>,
    ) -> Result<InputPipelineResult> {
        config.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.input_mode == Some(config.input.mode),
            "输入模式与当前控制器不匹配"
        );
        ensure!(
            !self.tasker.running(),
            "Maa Tasker 仍在运行，不能启动输入 Pipeline"
        );
        if self.input_cleanup_pending
            || !self.held_keys.is_empty()
            || !self.held_contacts.is_empty()
        {
            self.release_inputs()?;
        }
        let mut stopped_at = None;
        crate::input::wait_delay(
            Duration::from_millis(config.input.start_delay_ms.into()),
            &cancel,
            &mut stopped_at,
        );
        if !cancel.load(Ordering::Acquire) {
            self.prepare_input_target(config.input, &cancel, &mut stopped_at)?;
        }
        let (pipeline, inputs) = input_pipeline_definition(config)?;
        let mut report = self.run_pipeline_override(
            INPUT_PIPELINE_ENTRY,
            &inputs,
            cancel,
            Duration::from_millis(config.timeout_ms.into()),
            &pipeline,
        )?;
        // 启动延时 / 截图期间观察到的取消也保留；计时不包含控制器重建。
        if let Some(at) = stopped_at {
            report.stop_ms = Some(at.elapsed().as_millis() as u64);
        }
        Ok(InputPipelineResult {
            config,
            pipeline: report,
        })
    }
}

fn input_pipeline_definition(config: InputPipelineConfig) -> Result<(String, PipelineInputPlan)> {
    use od_core::InputProbeAction;
    use serde_json::json;
    config.validate().map_err(anyhow::Error::msg)?;
    let mut inputs = PipelineInputPlan::default();
    let (mut down, mut up) = match config.input.action {
        InputProbeAction::Key { key } => {
            inputs.keys.insert(key);
            (
                json!({"action":"KeyDown", "key":key, "auto_up":true}),
                json!({"action":"KeyUp", "key":key}),
            )
        }
        action => {
            let (contact, x, y, _) = crate::input::touch_args(action).expect("鼠标动作有通道");
            inputs.contacts.insert(contact);
            (
                json!({"action":"TouchDown", "contact":contact, "target":[x,y], "pressure":0, "auto_up":true}),
                json!({"action":"TouchUp", "contact":contact}),
            )
        }
    };
    let base = json!({"recognition":"DirectHit", "pre_delay":0, "post_delay":0, "rate_limit":0, "repeat":1, "repeat_delay":0, "enabled":true, "inverse":false, "on_error":[], "next":[]});
    for node in [&mut down, &mut up] {
        for (key, value) in base.as_object().expect("对象") {
            node[key] = value.clone();
        }
    }
    down["next"] = json!(["Prototype.Input.Hold"]);
    up["pre_delay"] = json!(config.input.hold_ms % 25);
    let mut hold = base;
    hold["action"] = json!("DoNothing");
    hold["repeat"] = json!(config.input.hold_ms / 25 + 1);
    hold["repeat_delay"] = json!(25);
    hold["next"] = json!(["Prototype.Input.Up"]);
    Ok((
        json!({INPUT_PIPELINE_ENTRY:down, "Prototype.Input.Hold":hold, "Prototype.Input.Up":up})
            .to_string(),
        inputs,
    ))
}
