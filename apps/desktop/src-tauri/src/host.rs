use od_protocol::{
    ApiError, Command, Handshake, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, PROTOCOL_VERSION, Payload,
    Request, Response, read_frame, write_frame,
};
use std::{
    io::Write,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Child, Stdio},
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tokio::{
    net::windows::named_pipe::{ClientOptions, NamedPipeClient},
    sync::Mutex,
};
use uuid::Uuid;

struct Session {
    pipe: NamedPipeClient,
    child: Child,
}

impl Drop for Session {
    fn drop(&mut self) {
        // 桌面原型只开放截图和无输入任务。普通退出优先走 Shutdown。
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

pub struct Host {
    session: Mutex<Option<Session>>,
    pub channel: std::sync::Mutex<Option<tauri::ipc::Channel<od_protocol::DesktopUpdate>>>,
}

fn error(error: impl std::fmt::Display) -> ApiError {
    ApiError::new("engine_offline", error.to_string())
}

impl Host {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
            channel: std::sync::Mutex::new(None),
        }
    }

    pub async fn start(&self, app: &AppHandle) -> Result<(), ApiError> {
        let mut session = self.session.lock().await;
        if session.is_some() {
            return Ok(());
        }
        let development_root: Option<PathBuf> = if cfg!(debug_assertions) {
            Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../..")
                    .canonicalize()
                    .map_err(error)?,
            )
        } else {
            None
        };
        let layout = od_platform::desktop_layout(
            &std::env::current_exe().map_err(error)?,
            &app.path().resource_dir().map_err(error)?,
            &app.path().app_data_dir().map_err(error)?,
            development_root.as_deref(),
        )
        .map_err(error)?;
        if !layout.engine.is_file() {
            return Err(error(if cfg!(debug_assertions) && !layout.portable {
                "缺少随附引擎，请运行 npm run prepare:engine"
            } else {
                "缺少随附引擎，请重新解压完整测试包"
            }));
        }
        let pipe_name = format!(r"\\.\pipe\zzz-auto-{}", Uuid::new_v4().simple());
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let mut child = std::process::Command::new(layout.engine)
            .arg("--root")
            .arg(&layout.root)
            .arg("--data")
            .arg(layout.data)
            .args([
                "serve",
                "--pipe",
                &pipe_name,
                "--parent-pid",
                &std::process::id().to_string(),
            ])
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(error)?;
        let connect = async {
            child
                .stdin
                .take()
                .ok_or_else(|| error("无法建立引擎配对通道"))?
                .write_all(format!("{token}\n").as_bytes())
                .map_err(error)?;
            let mut pipe = loop {
                match ClientOptions::new().open(&pipe_name) {
                    Ok(pipe) => break pipe,
                    Err(_) => {
                        if let Some(status) = child.try_wait().map_err(error)? {
                            return Err(error(format!("引擎提前退出：{status}")));
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            };
            od_platform::verify_server(&pipe, child.id()).map_err(error)?;
            write_frame(
                &mut pipe,
                &Handshake {
                    version: PROTOCOL_VERSION,
                    token,
                },
                MAX_REQUEST_BYTES,
            )
            .await
            .map_err(error)?;
            let paired: serde_json::Value = read_frame(&mut pipe, MAX_RESPONSE_BYTES)
                .await
                .map_err(error)?;
            if paired["paired"] != true || paired["version"] != PROTOCOL_VERSION {
                return Err(error("引擎配对失败"));
            }
            Ok::<_, ApiError>(pipe)
        };
        let result = tokio::time::timeout(Duration::from_secs(15), connect).await;
        match result {
            Ok(Ok(pipe)) => {
                *session = Some(Session { pipe, child });
                Ok(())
            }
            failure => {
                let _ = child.kill();
                let _ = child.wait();
                Err(match failure {
                    Ok(Err(error)) => error,
                    _ => error("引擎启动超时"),
                })
            }
        }
    }

    pub async fn request(&self, command: Command) -> Result<Payload, ApiError> {
        let mut guard = self.session.lock().await;
        let session = guard
            .as_mut()
            .ok_or_else(|| error("引擎未连接，使用“重连引擎”重新启动"))?;
        let request = Request {
            version: PROTOCOL_VERSION,
            id: Uuid::new_v4(),
            command,
        };
        let call = async {
            write_frame(&mut session.pipe, &request, MAX_REQUEST_BYTES)
                .await
                .map_err(error)?;
            let response: Response = read_frame(&mut session.pipe, MAX_RESPONSE_BYTES)
                .await
                .map_err(error)?;
            if response.id != request.id || response.version != PROTOCOL_VERSION {
                return Err(error("引擎响应与请求不符"));
            }
            if let Some(error) = response.error {
                return Err(error);
            }
            response.result.ok_or_else(|| error("引擎返回空响应"))
        };
        match tokio::time::timeout(Duration::from_secs(15), call).await {
            Ok(Ok(payload)) => Ok(payload),
            Ok(Err(failure)) if failure.code != "engine_offline" => Err(failure),
            result => {
                guard.take();
                Err(match result {
                    Ok(Err(error)) => error,
                    _ => error("原生操作超过响应时限，引擎已断开；请查看验收记录并重连"),
                })
            }
        }
    }

    pub async fn shutdown(&self) {
        let _ = self.request(Command::Shutdown).await;
        self.session.lock().await.take();
    }
}
