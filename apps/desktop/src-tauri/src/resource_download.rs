use od_core::runtime_resources::{
    RuntimeDownloadEvent, RuntimeDownloadProgress, RuntimeDownloadSource, RuntimeDownloadedArchive,
};
use od_platform::DesktopLayout;
use od_protocol::ApiError;
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

fn error(message: impl ToString) -> ApiError {
    ApiError::new("resource_download_failed", message.to_string())
}

pub fn run_configured(
    layout: &DesktopLayout,
    source: &RuntimeDownloadSource,
    proxy: od_core::download_proxy::DownloadProxy,
    expected_release: Option<od_core::runtime_feed::RuntimeRelease>,
    cancel: &AtomicBool,
    mut observer: impl FnMut(RuntimeDownloadProgress),
) -> Result<Option<RuntimeDownloadedArchive>, ApiError> {
    proxy
        .validate()
        .map_err(|message| ApiError::new("invalid_proxy", message))?;
    if cancel.load(Ordering::Acquire) {
        return Ok(None);
    }
    let output = layout
        .data
        .join("runtime-resources/downloads")
        .join(format!("{}.zip", source.sha256));
    let mut child = Command::new(&layout.engine)
        .creation_flags(0x08000000)
        .arg("--root")
        .arg(&layout.root)
        .args(["runtime-resources", "download-managed", "--output"])
        .arg(&output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| error("无法启动资源下载进程"))?;
    let mut input = child.stdin.take().expect("下载进程标准输入已连接");
    let initial = serde_json::to_writer(
        &mut input,
        &od_core::download_proxy::ManagedDownloadRequest {
            source: source.clone(),
            proxy,
            expected_release,
        },
    )
    .map_err(|_| error("无法提交资源下载信息"))
    .and_then(|_| {
        input
            .write_all(b"\n")
            .map_err(|_| error("无法提交资源下载信息"))
    });
    if let Err(error) = initial {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let stdout = child.stdout.take().expect("下载进程标准输出已连接");
    let (sender, receiver) = mpsc::sync_channel(4);
    let reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            let message = match reader.by_ref().take(65537).read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) if line.len() <= 65536 && line.last() == Some(&b'\n') => {
                    serde_json::from_slice::<RuntimeDownloadEvent>(&line)
                        .map_err(|_| error("下载进程返回无效状态"))
                }
                _ => Err(error("无法读取下载进程状态")),
            };
            let failed = message.is_err();
            if sender.send(message).is_err() || failed {
                break;
            }
        }
    });
    let started = Instant::now();
    let mut cancelled_at = None;
    let mut terminal = None;
    let result = loop {
        if cancel.load(Ordering::Acquire) && cancelled_at.is_none() {
            let _ = input.write_all(b"\n");
            cancelled_at = Some(Instant::now());
        }
        if started.elapsed() > Duration::from_secs(180)
            || cancelled_at.is_some_and(|time| time.elapsed() > Duration::from_secs(30))
        {
            break Err(error("下载进程未及时收尾，已结束；可重试检查缓存"));
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(RuntimeDownloadEvent::Progress(progress)))
                if terminal.is_none()
                    && progress.total_bytes == source.bytes
                    && progress.completed_bytes <= source.bytes =>
            {
                observer(progress)
            }
            Ok(Ok(event)) if terminal.is_none() => match event {
                RuntimeDownloadEvent::Completed(archive)
                    if archive.archive == output.display().to_string()
                        && archive.sha256 == source.sha256
                        && archive.bytes == source.bytes =>
                {
                    terminal = Some(Ok(Some(archive)))
                }
                RuntimeDownloadEvent::Cancelled => terminal = Some(Ok(None)),
                RuntimeDownloadEvent::Failed(message) => terminal = Some(Err(error(message))),
                _ => break Err(error("下载进程返回无效状态")),
            },
            Ok(Err(error)) => break Err(error),
            Ok(_) => break Err(error("下载进程返回重复终态")),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break terminal.unwrap_or_else(|| Err(error("下载进程提前退出")));
            }
        }
    };
    // 异常、取消超时或协议失败均先确认子进程退出，再允许开始下一项。
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|_| error("无法确认下载进程退出"));
    drop(receiver);
    let _ = reader.join();
    let status = status?;
    if !status.success() && result.is_ok() {
        return Err(error("下载进程异常退出"));
    }
    result
}
