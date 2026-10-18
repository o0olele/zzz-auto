use od_platform::{DesktopLayout, EngineProcess};
use od_protocol::{
    ApiError, Command, EngineGreeting, EnginePrivilege, Handshake, MAX_REQUEST_BYTES,
    MAX_RESPONSE_BYTES, PROTOCOL_VERSION, Payload, Request, Response, read_frame, write_frame,
};
use std::{ffi::OsString, future::Future, time::Duration};
use tokio::net::windows::named_pipe::NamedPipeServer;
use uuid::Uuid;

#[path = "recovery.rs"]
pub mod recovery;
use recovery::{RecoveryJournal, recovery_error};

pub fn offline(error: impl std::fmt::Display) -> ApiError {
    ApiError::new("engine_offline", error.to_string())
}

pub fn launch_error(error: anyhow::Error) -> ApiError {
    if error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.raw_os_error() == Some(1223))
    {
        ApiError::new(
            "elevation_cancelled",
            "已取消 Windows 授权，当前引擎保持连接",
        )
    } else {
        offline(error)
    }
}

pub fn require_idle(payload: Payload) -> Result<(), ApiError> {
    match payload {
        Payload::Snapshot(snapshot)
            if snapshot
                .run
                .as_ref()
                .is_none_or(|run| run.state.is_terminal()) =>
        {
            Ok(())
        }
        Payload::Snapshot(_) => Err(ApiError::new(
            "busy",
            "已有任务运行或正在收尾，请等待完成后重试",
        )),
        _ => Err(offline("无法确认当前运行状态")),
    }
}

pub struct EngineSession {
    #[allow(dead_code)] // 桌面日志使用；独立通信专项复用本文件时可能不读取。
    pub id: Uuid,
    pipe: Option<NamedPipeServer>,
    process: EngineProcess,
    pub privilege: EnginePrivilege,
    journal: Option<RecoveryJournal>,
}

impl EngineSession {
    pub async fn switch_to(
        &mut self,
        candidate: impl Future<Output = Result<Self, ApiError>>,
    ) -> Result<Self, ApiError> {
        require_idle(self.request(Command::Snapshot).await?)?;
        tokio::pin!(candidate);
        let mut candidate = loop {
            tokio::select! {
                result = &mut candidate => break result?,
                _ = tokio::time::sleep(Duration::from_secs(10)) => {
                    // 用户可能长时间停留在 UAC；保持旧引擎心跳，取消授权仍可使用。
                    self.request(Command::Snapshot).await?;
                }
            }
        };
        if let Err(error) = self.shutdown().await {
            candidate.disconnect();
            let _ = candidate.wait_exit(Duration::from_secs(15)).await;
            return Err(error);
        }
        Ok(candidate)
    }

    pub async fn launch(layout: &DesktopLayout, elevated: bool) -> Result<Self, ApiError> {
        let journal = RecoveryJournal::open(&layout.data)?;
        journal.require_clean()?;
        let mut session = Self::launch_role(layout, elevated, "serve-client").await?;
        session.journal = Some(journal);
        Ok(session)
    }

    async fn launch_role(
        layout: &DesktopLayout,
        elevated: bool,
        role: &str,
    ) -> Result<Self, ApiError> {
        let name = format!(r"\\.\pipe\zzz-auto-{}", Uuid::new_v4().simple());
        // 必须先占有名称，才能启动可能具有更高权限的客户端。
        let mut pipe = od_platform::create_private_pipe(&name).map_err(offline)?;
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let args: Vec<OsString> = vec![
            "--root".into(),
            layout.root.as_os_str().into(),
            "--data".into(),
            layout.data.as_os_str().into(),
            role.into(),
            "--pipe".into(),
            name.into(),
            "--parent-pid".into(),
            std::process::id().to_string().into(),
        ];
        let executable = layout.engine.clone();
        let mut process =
            tokio::task::spawn_blocking(move || EngineProcess::spawn(&executable, &args, elevated))
                .await
                .map_err(offline)?
                .map_err(launch_error)?;
        let connect = async {
            loop {
                tokio::select! {
                    result = pipe.connect() => { result.map_err(offline)?; break; }
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {
                        if process.exited().map_err(offline)? { return Err(offline("引擎在配对前退出")); }
                    }
                }
            }
            od_platform::verify_client(&pipe, process.id()).map_err(offline)?;
            write_frame(
                &mut pipe,
                &Handshake {
                    version: PROTOCOL_VERSION,
                    token: token.clone(),
                },
                MAX_REQUEST_BYTES,
            )
            .await
            .map_err(offline)?;
            let greeting: EngineGreeting = read_frame(&mut pipe, MAX_REQUEST_BYTES)
                .await
                .map_err(offline)?;
            if greeting.version != PROTOCOL_VERSION || greeting.token != token {
                return Err(offline("引擎配对信息不符"));
            }
            if elevated && greeting.engine_integrity < 0x3000 {
                return Err(offline("Windows 未启动具有管理员权限的引擎"));
            }
            Ok(EnginePrivilege {
                engine_integrity: greeting.engine_integrity,
                desktop_integrity: od_platform::process_integrity(std::process::id())
                    .map_err(offline)?,
            })
        };
        let privilege = tokio::time::timeout(Duration::from_secs(15), connect)
            .await
            .map_err(|_| offline("引擎配对超时，未启动游戏任务"))??;
        Ok(Self {
            id: Uuid::new_v4(),
            pipe: Some(pipe),
            process,
            privilege,
            journal: None,
        })
    }

    pub fn connected(&self) -> bool {
        self.pipe.is_some()
    }

    pub fn exited(&mut self) -> Result<bool, ApiError> {
        self.process.exited().map_err(offline)
    }

    pub fn disconnect(&mut self) {
        self.pipe.take();
    }

    pub async fn request(&mut self, command: Command) -> Result<Payload, ApiError> {
        if matches!(
            command,
            Command::PreviewConfigImport { .. } | Command::ApplyConfigImport { .. }
        ) {
            self.require_recovered()?;
            if let Some(journal) = &self.journal {
                journal.require_clean()?;
            }
        }
        let mail_start = matches!(
            command,
            Command::StartMail {
                allow_input: true,
                ..
            }
        );
        let input = match &command {
            Command::StartCoffeeFromWorld {
                config,
                allow_input: true,
            } => {
                config
                    .validate()
                    .map_err(|error| ApiError::new("invalid_input", error))?;
                Some(od_core::coffee_approach::recovery_inputs().to_vec())
            }
            Command::StartCoffee {
                config,
                allow_input: true,
            } => {
                config
                    .validate()
                    .map_err(|error| ApiError::new("invalid_input", error))?;
                Some(od_core::coffee_run::CoffeeConfig::recovery_inputs().to_vec())
            }
            Command::StartMap {
                config,
                allow_input: true,
            } => {
                config
                    .validate()
                    .map_err(|error| ApiError::new("invalid_input", error))?;
                Some(od_core::map_transport::MapConfig::recovery_inputs().to_vec())
            }
            Command::StartMail {
                config,
                allow_input: true,
            } => {
                config
                    .validate()
                    .map_err(|error| ApiError::new("invalid_input", error))?;
                Some(od_core::mail::MailConfig::recovery_inputs().to_vec())
            }
            Command::StartGroupTrial {
                allow_input: true, ..
            } => Some(od_core::group_run::recovery_inputs()),
            Command::StartInputProbe {
                config,
                allow_input: true,
            } => Some(vec![*config]),
            Command::StartInputPipeline {
                config,
                allow_input: true,
            } => {
                config
                    .validate()
                    .map_err(|error| ApiError::new("invalid_input", error))?;
                Some(vec![config.input])
            }
            _ => None,
        };
        if let Some(configs) = &input {
            for config in configs {
                config
                    .validate()
                    .map_err(|error| ApiError::new("invalid_input", error))?;
            }
            let snapshot = self.request_raw(Command::Snapshot).await?;
            require_idle(snapshot.clone())?;
            if let Some(journal) = &mut self.journal {
                journal.observe(&snapshot)?;
                journal.require_clean()?;
            }
            let Payload::Snapshot(snapshot) = snapshot else {
                return Err(offline("无法确认游戏身份"));
            };
            let window = snapshot
                .window
                .ok_or_else(|| ApiError::new("not_connected", "请先连接游戏窗口"))?;
            let plan = od_platform::InputRecoveryPlan {
                version: if matches!(command, Command::StartGroupTrial { .. }) {
                    6
                } else if matches!(command, Command::StartCoffeeFromWorld { .. }) {
                    5
                } else if matches!(command, Command::StartCoffee { .. }) {
                    4
                } else if matches!(command, Command::StartMap { .. }) {
                    3
                } else if mail_start {
                    2
                } else {
                    1
                },
                engine: od_platform::ProcessStamp::read(self.process.id())
                    .map_err(recovery_error)?,
                game: od_platform::ProcessStamp::read(window.pid).map_err(recovery_error)?,
                window,
                config: configs[0],
                additional_inputs: configs.iter().skip(1).copied().collect(),
                engine_integrity: self.privilege.engine_integrity,
                run_id: None,
            };
            if let Some(journal) = &mut self.journal {
                journal.arm(plan)?;
            }
        } else if let Command::Resume { run_id } = command {
            let snapshot = self.request_raw(Command::Snapshot).await?;
            let Payload::Snapshot(snapshot) = snapshot else {
                return Err(offline("无法确认暂停状态"));
            };
            if !snapshot.run.as_ref().is_some_and(|run| {
                run.id == run_id
                    && run.state == od_core::RunState::Paused
                    && (run.mail.is_some() || run.map.is_some() || run.coffee.is_some())
            }) {
                return Err(ApiError::new(
                    "invalid_run_state",
                    "流程尚未确认暂停或运行 ID 已过期",
                ));
            }
            let version = if snapshot.run.as_ref().is_some_and(|run| run.group.is_some()) {
                6
            } else if snapshot
                .run
                .as_ref()
                .and_then(|run| run.coffee.as_ref())
                .is_some_and(|coffee| coffee.approach.is_some())
            {
                5
            } else if snapshot
                .run
                .as_ref()
                .is_some_and(|run| run.coffee.is_some())
            {
                4
            } else if snapshot.run.as_ref().is_some_and(|run| run.map.is_some()) {
                3
            } else {
                2
            };
            let valid = self
                .journal
                .as_ref()
                .ok_or_else(|| recovery_error("缺少流程恢复登记"))
                .and_then(|journal| {
                    if version == 2 {
                        journal.require_mail_run(run_id)
                    } else {
                        journal.require_pipeline_run(run_id, version)
                    }
                });
            if let Err(error) = valid {
                self.disconnect();
                return Err(error);
            }
        } else if !matches!(
            command,
            Command::Snapshot
                | Command::ModelResources
                | Command::StartModelResources { .. }
                | Command::CancelModelResources { .. }
                | Command::AccountConfig
                | Command::PreviewGroup { .. }
                | Command::RunHistory { .. }
                | Command::QueryRunHistory { .. }
                | Command::ExportRunHistory { .. }
                | Command::RetryRunSave
                | Command::Stop { .. }
                | Command::Pause { .. }
                | Command::Events { .. }
                | Command::Shutdown
        ) && let Some(journal) = &self.journal
        {
            journal.require_clean()?;
        }
        let result = self.request_raw(command).await;
        if let Some(journal) = &mut self.journal {
            match &result {
                Ok(payload) => {
                    if input.is_some() {
                        journal.accepted(payload)?;
                    }
                    journal.observe(payload)?;
                }
                Err(error) if input.is_some() && error.code != "engine_offline" => {
                    journal.clear()?
                }
                _ => {}
            }
            let failed_cleanup = matches!(&result, Ok(Payload::Snapshot(snapshot)) if snapshot.run.as_ref().is_some_and(|run| run.state.is_terminal()))
                && journal.plan.is_some();
            if failed_cleanup {
                self.disconnect();
                return Err(recovery_error("引擎未提供输入释放证明，已请求其退出"));
            }
        }
        if !self.connected() {
            self.require_recovered()?;
        }
        result
    }

    async fn request_raw(&mut self, command: Command) -> Result<Payload, ApiError> {
        let pipe = self
            .pipe
            .as_mut()
            .ok_or_else(|| offline("引擎已断开，正在等待退出"))?;
        let request = Request {
            version: PROTOCOL_VERSION,
            id: Uuid::new_v4(),
            command,
        };
        let call = async {
            write_frame(pipe, &request, MAX_REQUEST_BYTES)
                .await
                .map_err(offline)?;
            let response: Response = read_frame(pipe, MAX_RESPONSE_BYTES)
                .await
                .map_err(offline)?;
            if response.id != request.id || response.version != PROTOCOL_VERSION {
                return Err(offline("引擎响应与请求不符"));
            }
            if let Some(error) = response.error {
                return Err(error);
            }
            response.result.ok_or_else(|| offline("引擎返回空响应"))
        };
        match tokio::time::timeout(Duration::from_secs(15), call).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) if error.code != "engine_offline" => Err(error),
            result => {
                self.disconnect();
                Err(match result {
                    Ok(Err(error)) => error,
                    _ => offline("引擎响应超时，已断开管道并请求收尾；退出前不能重连"),
                })
            }
        }
    }

    pub async fn shutdown(&mut self) -> Result<(), ApiError> {
        if self.connected() && self.journal.as_ref().is_some_and(|j| j.plan.is_some()) {
            // 退出应先停止并读取明确的释放证明；Shutdown 的 Ack 不代表释放成功。
            let snapshot = self.request(Command::Snapshot).await;
            if let Ok(Payload::Snapshot(snapshot)) = snapshot
                && let Some(run) = snapshot.run
                && !run.state.is_terminal()
            {
                let _ = self.request(Command::Stop { run_id: run.id }).await;
                let _ = tokio::time::timeout(Duration::from_secs(15), async {
                    while self.connected()
                        && self.journal.as_ref().is_some_and(|j| j.plan.is_some())
                    {
                        if let Ok(Payload::Snapshot(snapshot)) =
                            self.request(Command::Snapshot).await
                            && snapshot.run.is_none_or(|r| r.state.is_terminal())
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await;
            }
        }
        if self.connected() {
            let _ = self.request(Command::Shutdown).await;
        }
        self.disconnect();
        self.wait_exit(Duration::from_secs(15)).await?;
        self.require_recovered()
    }

    pub fn require_recovered(&self) -> Result<(), ApiError> {
        if let Some(journal) = &self.journal {
            journal.require_clean()?;
        }
        Ok(())
    }

    pub fn reload_recovery(&mut self, layout: &DesktopLayout) -> Result<(), ApiError> {
        self.journal = Some(RecoveryJournal::open(&layout.data)?);
        self.require_recovered()
    }

    pub async fn recover(layout: &DesktopLayout) -> Result<String, ApiError> {
        let mut journal = RecoveryJournal::open(&layout.data)?;
        let Some(plan) = journal.plan.clone() else {
            return Ok("没有待恢复的输入记录".into());
        };
        if !plan.engine.exited().map_err(recovery_error)? {
            return Err(recovery_error("旧引擎仍未退出"));
        }
        let elevated = plan.engine_integrity
            > od_platform::process_integrity(std::process::id()).map_err(recovery_error)?;
        let mut helper = Self::launch_role(layout, elevated, "recover-input")
            .await
            .map_err(|error| {
                if error.code == "elevation_cancelled" {
                    recovery_error("已取消恢复进程的 Windows 授权，遗留输入尚未确认释放")
                } else {
                    recovery_error(error)
                }
            })?;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            let pipe = helper.pipe.as_mut().expect("恢复进程已配对");
            write_frame(pipe, &plan, MAX_REQUEST_BYTES)
                .await
                .map_err(recovery_error)?;
            let result: Result<String, ApiError> = read_frame(pipe, MAX_REQUEST_BYTES)
                .await
                .map_err(recovery_error)?;
            result
        })
        .await
        .map_err(|_| recovery_error("恢复进程响应超时"))?;
        helper.disconnect();
        helper
            .wait_exit(Duration::from_secs(5))
            .await
            .map_err(recovery_error)?;
        let message = result?;
        journal.clear()?;
        Ok(message)
    }

    pub async fn wait_exit(&mut self, timeout: Duration) -> Result<(), ApiError> {
        tokio::time::timeout(timeout, async {
            while !self.exited()? {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Ok(())
        })
        .await
        .map_err(|_| {
            ApiError::new(
                "engine_stopping",
                "引擎尚未退出，仍在等待原生收尾；请稍后重试，不能启动另一个引擎",
            )
        })?
    }
}
