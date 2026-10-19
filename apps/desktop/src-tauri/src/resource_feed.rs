use od_core::runtime_feed::{
    ManagedFeedRequest, RuntimeFeedCatalog, RuntimeFeedEvent, RuntimeReleaseSource,
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

fn error(message: &str) -> ApiError {
    ApiError::new("resource_feed_failed", message)
}
pub fn run(
    layout: &DesktopLayout,
    request: ManagedFeedRequest,
    cancel: &AtomicBool,
) -> Result<Option<(RuntimeFeedCatalog, Vec<RuntimeReleaseSource>)>, ApiError> {
    if cancel.load(Ordering::Acquire) {
        return Ok(None);
    }
    let expected = request.source.sha256.clone();
    let mut child = Command::new(&layout.engine)
        .creation_flags(0x08000000)
        .arg("--root")
        .arg(&layout.root)
        .args(["runtime-resources", "discover-managed"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| error("无法启动发布清单进程"))?;
    let mut input = child.stdin.take().expect("清单标准输入已连接");
    let initial = serde_json::to_writer(&mut input, &request)
        .map_err(|_| error("无法提交发布清单来源"))
        .and_then(|()| {
            input
                .write_all(b"\n")
                .map_err(|_| error("无法提交发布清单来源"))
        });
    if let Err(error) = initial {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let stdout = child.stdout.take().expect("清单标准输出已连接");
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut stream = BufReader::new(stdout);
        let mut line = Vec::new();
        let result = match stream.by_ref().take(1048577).read_until(b'\n', &mut line) {
            Ok(_) if line.len() <= 1048576 && line.last() == Some(&b'\n') => {
                serde_json::from_slice::<RuntimeFeedEvent>(&line)
                    .map_err(|_| error("发布清单进程返回无效结果"))
            }
            _ => Err(error("发布清单进程结果缺失或过大")),
        };
        let _ = sender.send(result.map(Some));
        let mut extra = [0];
        let _ = sender.send(if stream.read(&mut extra).is_ok_and(|n| n == 0) {
            Ok(None)
        } else {
            Err(error("发布清单进程返回额外内容"))
        });
    });
    let started = Instant::now();
    let mut cancelled_at = None;
    let mut terminal = None;
    let mut finished = false;
    let result = loop {
        if cancel.load(Ordering::Acquire) && cancelled_at.is_none() {
            let _ = input.write_all(b"\n");
            cancelled_at = Some(Instant::now());
        }
        if started.elapsed() > Duration::from_secs(180)
            || cancelled_at.is_some_and(|t: Instant| t.elapsed() > Duration::from_secs(30))
        {
            break Err(error("发布清单进程未及时收尾"));
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(event)) if terminal.is_none() && !finished => {
                terminal = Some(match event {
                    Some(RuntimeFeedEvent::Completed { catalog, sources })
                        if catalog.id == expected
                            && catalog.releases.len() == sources.len()
                            && !sources.is_empty()
                            && sources.len() <= 64
                            && catalog
                                .releases
                                .iter()
                                .zip(&sources)
                                .all(|(option, source)| option.release == source.release) =>
                    {
                        Ok(Some((catalog, sources)))
                    }
                    Some(RuntimeFeedEvent::Cancelled) => Ok(None),
                    Some(RuntimeFeedEvent::Failed(message)) => {
                        Err(ApiError::new("resource_feed_failed", message))
                    }
                    _ => Err(error("发布清单进程返回不一致结果")),
                });
            }
            Ok(Ok(None)) if terminal.is_some() => {
                finished = true;
            }
            Ok(Err(error)) => break Err(error),
            Ok(_) => break Err(error("发布清单进程重复返回结果")),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break if finished {
                    terminal.unwrap_or_else(|| Err(error("发布清单进程提前退出")))
                } else {
                    Err(error("发布清单进程未完整退出"))
                };
            }
        }
    };
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait();
    drop(receiver);
    let _ = reader.join();
    if !status.is_ok_and(|s| s.success()) && result.is_ok() {
        return Err(error("发布清单进程异常退出"));
    }
    if cancel.load(Ordering::Acquire) && result.is_ok() {
        return Ok(None);
    }
    result
}
