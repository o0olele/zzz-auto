use crate::runtime::Engine;
use anyhow::{Result, ensure};
use od_protocol::{
    ApiError, Command, Handshake, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, PROTOCOL_VERSION, Request,
    Response, read_frame, write_frame,
};
use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
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
        let mut cache: HashMap<uuid::Uuid, (String, Response)> = HashMap::new();
        let mut order = VecDeque::new();
        loop {
            let request: Request = tokio::time::timeout(
                Duration::from_secs(45),
                read_frame(&mut pipe, MAX_REQUEST_BYTES),
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
                let response =
                    Response::from_result(request.id, engine.execute(request.command).await);
                if order.len() == 128
                    && let Some(old) = order.pop_front()
                {
                    cache.remove(&old);
                }
                order.push_back(request.id);
                cache.insert(request.id, (fingerprint, response.clone()));
                response
            };
            write_frame(&mut pipe, &response, MAX_RESPONSE_BYTES).await?;
            if shutdown && response.error.is_none() {
                return Ok(());
            }
        }
    }
    .await;
    // 配对失败、超时、桌面退出和断线均先停止并等待原生收尾。
    engine.cancel_active();
    let _ = engine.shutdown().await;
    session
}
