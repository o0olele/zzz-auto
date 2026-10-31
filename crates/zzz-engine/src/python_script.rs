//! 开发脚本进程：只传观察和语义决策，键鼠与战斗仍由已有执行器持有。
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::Value;
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub const VERSION: u32 = 1;
const MAX_LINE: u64 = 32 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentConfig {
    pub enabled: bool,
    pub python: PathBuf,
    pub app: String,
    pub settings: PathBuf,
}

impl DevelopmentConfig {
    pub fn directory() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../python-scripts")
    }
    pub fn read() -> Result<Option<Self>> {
        if !cfg!(debug_assertions) {
            return Ok(None);
        }
        let directory = Self::directory().canonicalize()?;
        let local = directory.join("development.local.json");
        let path = if local.is_file() {
            local
        } else {
            directory.join("development.json")
        };
        let config: Self = serde_json::from_slice(&std::fs::read(&path)?)
            .with_context(|| format!("Python 开发配置无效：{}", path.display()))?;
        if !config.enabled {
            return Ok(None);
        }
        ensure!(
            config.app == "map_exploration",
            "当前仅接入空洞探索 Python 开发脚本"
        );
        let settings = directory.join(&config.settings).canonicalize()?;
        ensure!(
            settings.starts_with(&directory),
            "Python 设置文件须位于 python-scripts 内"
        );
        Ok(Some(Self { settings, ..config }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptAction {
    Wait,
    Guide,
    Road,
    Marker,
    Arrive,
    Face,
    Interact,
    Recover,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptDecision {
    version: u32,
    id: u64,
    pub action: ScriptAction,
    #[serde(default)]
    pub guide: Option<od_core::world_patrol::PatrolPoint>,
    pub reason: String,
}

pub struct ScriptProcess {
    child: Child,
    requests: Option<mpsc::Sender<Vec<u8>>>,
    responses: mpsc::Receiver<Result<Value>>,
    writer: Option<JoinHandle<()>>,
    reader: Option<JoinHandle<()>>,
    sequence: u64,
    trace: Option<File>,
    stderr_path: Option<PathBuf>,
}

impl ScriptProcess {
    pub fn launch(config: &DevelopmentConfig, trace: Option<&Path>) -> Result<Self> {
        let mut command = Command::new(&config.python);
        command
            .args(["-u", "-B"])
            .arg(DevelopmentConfig::directory().join("run.py"))
            .arg("worker")
            .arg("--settings")
            .arg(&config.settings)
            .env("PYTHONIOENCODING", "utf-8");
        Self::spawn(command, &config.app, trace)
    }
    fn spawn(mut command: Command, app: &str, trace: Option<&Path>) -> Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let stderr_path = trace.map(|path| path.with_extension("stderr.log"));
        if let Some(path) = &stderr_path {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            command.stderr(Stdio::from(
                OpenOptions::new().create(true).append(true).open(path)?,
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW：不弹出脚本控制台。
        }
        let mut child = command.spawn().context(
            "无法启动 Python；请检查 python-scripts/development.local.json 中的解释器路径",
        )?;
        let mut stdin = child.stdin.take().context("Python 标准输入缺失")?;
        let stdout = child.stdout.take().context("Python 标准输出缺失")?;
        let (request_tx, request_rx) = mpsc::channel::<Vec<u8>>();
        let (response_tx, response_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            for request in request_rx {
                if stdin
                    .write_all(&request)
                    .and_then(|()| stdin.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = vec![];
                let result = (|| -> Result<Value> {
                    let count = reader
                        .by_ref()
                        .take(MAX_LINE + 1)
                        .read_until(b'\n', &mut line)?;
                    ensure!(count > 0, "Python 脚本已退出或关闭输出");
                    ensure!(
                        count as u64 <= MAX_LINE && line.last() == Some(&b'\n'),
                        "Python 响应超长或不完整"
                    );
                    Ok(serde_json::from_slice(&line)
                        .context("Python 响应必须是单行 JSON，日志请写到 stderr")?)
                })();
                let failed = result.is_err();
                if response_tx.send(result).is_err() || failed {
                    break;
                }
            }
        });
        let mut process = Self {
            child,
            requests: Some(request_tx),
            responses: response_rx,
            writer: Some(writer),
            reader: Some(reader),
            sequence: 0,
            trace: None,
            stderr_path,
        };
        let hello = process
            .responses
            .recv_timeout(Duration::from_secs(5))
            .context("Python 启动握手超时")?
            .map_err(|error| anyhow::anyhow!("{error}；{}", process.diagnostics()))?;
        ensure!(
            hello["version"] == VERSION && hello["ready"] == true && hello["app"] == app,
            "Python 启动失败或协议不匹配：{}",
            hello["error"].as_str().unwrap_or("未返回有效握手")
        );
        if let Some(path) = trace {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            process.trace = Some(OpenOptions::new().create(true).append(true).open(path)?);
        }
        Ok(process)
    }

    pub fn decide(
        &mut self,
        mut request: Value,
        cancel: &AtomicBool,
    ) -> Result<Option<ScriptDecision>> {
        self.sequence += 1;
        request["version"] = VERSION.into();
        request["id"] = self.sequence.into();
        let mut encoded = serde_json::to_vec(&request)?;
        ensure!(encoded.len() < 512 * 1024, "Python 观察数据超长");
        encoded.push(b'\n');
        self.requests
            .as_ref()
            .context("Python 通道已关闭")?
            .send(encoded)
            .context("Python 输入已关闭")?;
        let started = Instant::now();
        let response = loop {
            if cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
            ensure!(
                started.elapsed() < Duration::from_millis(750),
                "Python 决策超时，停止任务并释放输入"
            );
            match self.responses.recv_timeout(Duration::from_millis(5)) {
                Ok(value) => {
                    break value
                        .map_err(|error| anyhow::anyhow!("{error}；{}", self.diagnostics()))?;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => bail!("Python 输出断开：{error}"),
            }
        };
        if let Some(trace) = self.trace.as_mut() {
            serde_json::to_writer(
                &mut *trace,
                &serde_json::json!({"request":request,"response":response}),
            )?;
            trace.write_all(b"\n")?;
            trace.flush()?;
        }
        if let Some(error) = response["error"].as_str() {
            bail!("Python 脚本异常：{error}");
        }
        let decision: ScriptDecision = serde_json::from_value(response)?;
        ensure!(
            decision.version == VERSION && decision.id == self.sequence,
            "Python 返回了过期或不匹配的决策"
        );
        ensure!(
            !decision.reason.is_empty() && decision.reason.chars().count() <= 200,
            "Python 决策说明无效"
        );
        Ok(Some(decision))
    }
    fn diagnostics(&self) -> String {
        let Some(path) = &self.stderr_path else {
            return "无 Python 标准错误日志".into();
        };
        let read = || -> Result<String> {
            let mut file = File::open(path)?;
            let length = file.metadata()?.len();
            file.seek(SeekFrom::Start(length.saturating_sub(4096)))?;
            let mut bytes = vec![];
            file.read_to_end(&mut bytes)?;
            Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
        };
        match read() {
            Ok(text) if !text.is_empty() => text,
            _ => format!("错误日志：{}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(body: &str) -> Result<ScriptProcess> {
        let code = format!(
            "import json,sys,time\nprint(json.dumps(dict(version=1,ready=True,app='map_exploration')),flush=True)\nr=json.loads(sys.stdin.readline())\n{body}\n"
        );
        let mut command = Command::new("python");
        command.args(["-u", "-B", "-c", &code]);
        ScriptProcess::spawn(command, "map_exploration", None)
    }

    #[test]
    fn rejects_stale_unknown_oversized_and_crashed_workers() -> Result<()> {
        for body in [
            "print(json.dumps(dict(version=1,id=999,action='wait',reason='stale')),flush=True)",
            "print(json.dumps(dict(version=1,id=r['id'],action='press_any_key',reason='unknown')),flush=True)",
            "print('x'*40000,flush=True)",
            "sys.exit(1)",
            "print('not-json',flush=True)",
        ] {
            let mut process = worker(body)?;
            assert!(
                process
                    .decide(serde_json::json!({}), &AtomicBool::new(false))
                    .is_err(),
                "{body}"
            );
        }
        Ok(())
    }

    #[test]
    fn hung_python_times_out_and_cancel_does_not_wait_for_python() -> Result<()> {
        let mut process = worker("time.sleep(30)")?;
        let started = Instant::now();
        let error = process
            .decide(serde_json::json!({}), &AtomicBool::new(false))
            .unwrap_err();
        assert!(error.to_string().contains("超时"));
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(process);
        let mut process = worker("time.sleep(30)")?;
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            signal.store(true, Ordering::Release);
        });
        let started = Instant::now();
        assert!(process.decide(serde_json::json!({}), &cancel)?.is_none());
        assert!(started.elapsed() < Duration::from_millis(250));
        thread.join().unwrap();
        Ok(())
    }

    #[test]
    fn keeps_python_source_error_in_the_failure_reason() -> Result<()> {
        let mut process = worker(
            "print(json.dumps(dict(version=1,id=r['id'],error='map_exploration.py:123 test error')),flush=True)",
        )?;
        let error = process
            .decide(serde_json::json!({}), &AtomicBool::new(false))
            .unwrap_err();
        assert!(error.to_string().contains("map_exploration.py:123"));
        Ok(())
    }
}

impl Drop for ScriptProcess {
    fn drop(&mut self) {
        self.requests.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
