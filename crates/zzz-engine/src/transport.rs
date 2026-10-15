use crate::runtime::{Engine, EnginePaths};
use anyhow::{Result, ensure};
use od_protocol::{
    ApiError, Command, EngineGreeting, Handshake, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES,
    PROTOCOL_VERSION, Request, Response, read_frame, write_frame,
};
use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::windows::named_pipe::ClientOptions,
};

pub async fn serve_managed(engine: Engine, name: &str, token: &str, parent_pid: u32) -> Result<()> {
    let mut pipe = od_platform::create_private_pipe(name)?;
    let session = async {
        tokio::time::timeout(Duration::from_secs(20), pipe.connect()).await??;
        od_platform::verify_client(&pipe, parent_pid)?;
        let handshake: Handshake = tokio::time::timeout(
            Duration::from_secs(5),
            read_frame(&mut pipe, MAX_REQUEST_BYTES),
        )
        .await??;
        ensure!(
            handshake.version == PROTOCOL_VERSION && handshake.token == token,
            "协议版本或配对信息不符"
        );
        write_frame(
            &mut pipe,
            &serde_json::json!({"version": PROTOCOL_VERSION, "paired": true}),
            MAX_RESPONSE_BYTES,
        )
        .await?;
        serve_requests(&engine, &mut pipe).await
    }
    .await;
    engine.cancel_active();
    let _ = engine.shutdown().await;
    session
}

// Tokio ClientOptions 默认 SECURITY_IDENTIFICATION，禁止服务端模拟提升后的客户端。
pub async fn serve_connected(paths: EnginePaths, name: &str, parent_pid: u32) -> Result<()> {
    let (mut pipe, greeting) = pair_client(name, parent_pid).await?;
    // 身份检查完成后才初始化 SDK / 数据；不会接收任意程序或路径命令。
    let engine = Engine::new(paths)?;
    let session = async {
        write_frame(&mut pipe, &greeting, MAX_REQUEST_BYTES).await?;
        serve_requests(&engine, &mut pipe).await
    }
    .await;
    engine.cancel_active();
    let _ = engine.shutdown().await;
    session
}

async fn pair_client(
    name: &str,
    parent_pid: u32,
) -> Result<(
    tokio::net::windows::named_pipe::NamedPipeClient,
    EngineGreeting,
)> {
    ensure!(
        name.starts_with(r"\\.\pipe\zzz-auto-") && name.len() < 160,
        "管道名称不合法"
    );
    let mut pipe = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match ClientOptions::new().open(name) {
                Ok(pipe) => break Ok(pipe),
                Err(error) if matches!(error.raw_os_error(), Some(2 | 231)) => {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) => break Err(error),
            }
        }
    })
    .await??;
    od_platform::verify_server(&pipe, parent_pid)?;
    let handshake: Handshake = tokio::time::timeout(
        Duration::from_secs(5),
        read_frame(&mut pipe, MAX_REQUEST_BYTES),
    )
    .await??;
    ensure!(
        handshake.version == PROTOCOL_VERSION
            && handshake.token.len() == 64
            && handshake.token.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "托管配对信息无效"
    );
    let greeting = EngineGreeting {
        version: PROTOCOL_VERSION,
        token: handshake.token,
        engine_integrity: od_platform::process_integrity(std::process::id())?,
    };
    Ok((pipe, greeting))
}

pub async fn recover_connected(name: &str, parent_pid: u32) -> Result<()> {
    let (mut pipe, greeting) = pair_client(name, parent_pid).await?;
    write_frame(&mut pipe, &greeting, MAX_REQUEST_BYTES).await?;
    let plan: od_platform::InputRecoveryPlan = tokio::time::timeout(
        Duration::from_secs(5),
        read_frame(&mut pipe, MAX_REQUEST_BYTES),
    )
    .await??;
    let result = tokio::task::spawn_blocking(move || od_platform::recover_input(&plan))
        .await?
        .map_err(|error| ApiError::new("input_recovery_failed", format!("{error:#}")));
    write_frame(&mut pipe, &result, MAX_REQUEST_BYTES).await?;
    Ok(())
}

async fn serve_requests(
    engine: &Engine,
    pipe: &mut (impl AsyncRead + AsyncWrite + Unpin),
) -> Result<()> {
    let mut cache: HashMap<uuid::Uuid, (String, Response)> = HashMap::new();
    let mut order = VecDeque::new();
    loop {
        let request: Request = tokio::time::timeout(
            Duration::from_secs(45),
            read_frame(&mut *pipe, MAX_REQUEST_BYTES),
        )
        .await??;
        let fingerprint = serde_json::to_string(&request.command)?;
        let shutdown = matches!(request.command, Command::Shutdown);
        let response = if request.version != PROTOCOL_VERSION {
            Response::from_result(
                request.id,
                Err(ApiError::new("protocol_version", "通信协议版本不匹配")),
            )
        } else if let Some((previous, response)) = cache.get(&request.id) {
            if previous == &fingerprint {
                response.clone()
            } else {
                Response::from_result(
                    request.id,
                    Err(ApiError::new(
                        "request_id_conflict",
                        "同一请求 ID 不能用于不同命令",
                    )),
                )
            }
        } else {
            let response = Response::from_result(request.id, engine.execute(request.command).await);
            if order.len() == 128
                && let Some(old) = order.pop_front()
            {
                cache.remove(&old);
            }
            order.push_back(request.id);
            cache.insert(request.id, (fingerprint, response.clone()));
            response
        };
        write_frame(&mut *pipe, &response, MAX_RESPONSE_BYTES).await?;
        if shutdown && response.error.is_none() {
            return Ok(());
        }
    }
}
