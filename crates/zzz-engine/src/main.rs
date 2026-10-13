use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand, ValueEnum};
use maa_adapter::{
    CaptureMethod, InferenceDevice, InputMode, InputProbeAction, InputProbeConfig, NativeSession,
};
use od_core::{InputSide, MouseButton};
use std::{
    io::{BufRead, Read},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use zzz_engine::runtime::{Engine, EnginePaths};

#[derive(Parser)]
#[command(version, about = "绝区零 Rust / Maa 引擎：M1 原生集成原型")]
struct Args {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long)]
    sdk: Option<PathBuf>,
    #[arg(long)]
    resources: Option<PathBuf>,
    #[arg(long)]
    data: Option<PathBuf>,
    #[command(subcommand)]
    command: Action,
}

#[derive(Clone, Copy, ValueEnum)]
enum Capture {
    Gdi,
    PrintWindow,
    FramePool,
    Dxgi,
    ScreenDc,
}
#[derive(Clone, Copy, ValueEnum)]
enum Input {
    Foreground,
    ForegroundLegacy,
    Background,
    BackgroundPost,
    Xbox,
    Ds4,
}

#[derive(Clone, Copy, ValueEnum)]
enum ProbeAction {
    Key,
    Mouse,
    Stick,
    Trigger,
}

#[derive(Clone, Copy, ValueEnum)]
enum Side {
    Left,
    Right,
}
impl From<Side> for InputSide {
    fn from(side: Side) -> Self {
        match side {
            Side::Left => Self::Left,
            Side::Right => Self::Right,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Button {
    Left,
    Right,
    Middle,
    X1,
    X2,
}
impl From<Button> for MouseButton {
    fn from(button: Button) -> Self {
        match button {
            Button::Left => Self::Left,
            Button::Right => Self::Right,
            Button::Middle => Self::Middle,
            Button::X1 => Self::X1,
            Button::X2 => Self::X2,
        }
    }
}

#[derive(Subcommand)]
enum Action {
    Doctor,
    Applications,
    Windows,
    Serve {
        #[arg(long)]
        pipe: String,
        #[arg(long)]
        parent_pid: u32,
    },
    Probe {
        #[arg(long)]
        fixture: PathBuf,
        #[arg(long)]
        cancel_after_ms: Option<u64>,
    },
    Capture {
        #[arg(long)]
        hwnd: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, value_enum, default_value = "frame-pool")]
        method: Capture,
    },
    Recognize {
        #[arg(long)]
        fixture: PathBuf,
        #[arg(long)]
        image: PathBuf,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        params: PathBuf,
        #[arg(long)]
        directml: Option<i32>,
    },
    InputProbe {
        #[arg(long)]
        hwnd: String,
        #[arg(long, value_enum, default_value = "key")]
        action: ProbeAction,
        #[arg(long, required_if_eq("action", "key"), conflicts_with_all = ["button", "side", "x", "y", "value"])]
        key: Option<i32>,
        #[arg(long, value_enum, required_if_eq("action", "mouse"), conflicts_with_all = ["side", "value"])]
        button: Option<Button>,
        #[arg(long, value_enum, required_if_eq_any = [("action", "stick"), ("action", "trigger")])]
        side: Option<Side>,
        #[arg(long, allow_hyphen_values = true, required_if_eq_any = [("action", "mouse"), ("action", "stick")])]
        x: Option<i32>,
        #[arg(long, allow_hyphen_values = true, required_if_eq_any = [("action", "mouse"), ("action", "stick")])]
        y: Option<i32>,
        #[arg(long, required_if_eq("action", "trigger"), conflicts_with_all = ["x", "y"])]
        value: Option<i32>,
        #[arg(long, default_value_t = 100)]
        hold_ms: u64,
        #[arg(long, default_value_t = 3000)]
        start_delay_ms: u32,
        #[arg(long)]
        cancel_after_ms: Option<u64>,
        #[arg(long, value_enum, default_value = "foreground")]
        mode: Input,
        #[arg(long, value_enum, default_value = "frame-pool")]
        capture: Capture,
        #[arg(long)]
        allow_input: bool,
    },
}

fn capture(method: Capture) -> CaptureMethod {
    match method {
        Capture::Gdi => CaptureMethod::Gdi,
        Capture::PrintWindow => CaptureMethod::PrintWindow,
        Capture::FramePool => CaptureMethod::FramePool,
        Capture::Dxgi => CaptureMethod::Dxgi,
        Capture::ScreenDc => CaptureMethod::ScreenDc,
    }
}
fn print(value: impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let root = args.root.canonicalize().context("工程或程序根目录不存在")?;
    let paths = EnginePaths {
        sdk: args.sdk.unwrap_or_else(|| root.join("sdk/maa/bin")),
        resources: args.resources.unwrap_or_else(|| root.join("resources")),
        data: args.data.unwrap_or_else(|| root.join("data")),
    };
    match args.command {
        Action::Applications => return print(od_core::applications()?),
        Action::Windows => return print(od_platform::game_windows()?),
        Action::Serve { pipe, parent_pid } => {
            let mut token = String::new();
            std::io::stdin().lock().take(256).read_line(&mut token)?;
            let token = token.trim();
            ensure!(
                token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
                "配对密钥格式无效"
            );
            let engine = Engine::new(paths)?;
            return zzz_engine::transport::serve_managed(engine, &pipe, token, parent_pid).await;
        }
        _ => {}
    }
    let sdk = maa_adapter::load_sdk(&paths.sdk, &paths.data.join("maa-log"))?;
    match args.command {
        Action::Doctor => print(sdk),
        Action::Probe {
            fixture,
            cancel_after_ms,
        } => {
            let mut native =
                NativeSession::for_fixture(&fixture, &paths.resources, InferenceDevice::Cpu)?;
            let cancel = Arc::new(AtomicBool::new(false));
            if let Some(delay) = cancel_after_ms {
                let flag = cancel.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(delay));
                    flag.store(true, std::sync::atomic::Ordering::Release);
                });
            }
            print(native.health_check(cancel, Duration::from_secs(10))?)
        }
        Action::Capture {
            hwnd,
            output,
            method,
        } => {
            use base64::Engine;
            let window = od_platform::parse_game_window(&hwnd)?;
            let native = NativeSession::for_game(
                &window,
                &paths.resources,
                capture(method),
                InputMode::Foreground,
                InferenceDevice::Cpu,
            )?;
            let screenshot = native.screenshot()?;
            std::fs::write(
                &output,
                base64::engine::general_purpose::STANDARD.decode(&screenshot.png_base64)?,
            )?;
            print(
                serde_json::json!({"path": output, "width": screenshot.width, "height": screenshot.height, "elapsed_ms": screenshot.elapsed_ms, "capture_method": screenshot.capture_method, "quality": screenshot.quality}),
            )
        }
        Action::Recognize {
            fixture,
            image,
            kind,
            params,
            directml,
        } => {
            let device = directml.map(InferenceDevice::DirectMl).unwrap_or_default();
            let native = NativeSession::for_fixture(&fixture, &paths.resources, device)?;
            print(native.recognize(
                &image,
                &kind,
                &serde_json::from_slice(&std::fs::read(params)?)?,
            )?)
        }
        Action::InputProbe {
            hwnd,
            action,
            key,
            button,
            side,
            x,
            y,
            value,
            hold_ms,
            start_delay_ms,
            cancel_after_ms,
            mode,
            capture: method,
            allow_input,
        } => {
            ensure!(
                allow_input,
                "输入探针会操作游戏，必须显式指定 --allow-input"
            );
            ensure!((1..=2000).contains(&hold_ms), "按住时间须为 1..2000ms");
            let action = match action {
                ProbeAction::Key => InputProbeAction::Key {
                    key: key.context("缺少 --key")?,
                },
                ProbeAction::Mouse => {
                    ensure!(
                        key.is_none() && side.is_none() && value.is_none(),
                        "鼠标动作不接受按键、手柄或力度参数"
                    );
                    InputProbeAction::MouseButton {
                        button: button.context("缺少 --button")?.into(),
                        x: x.context("缺少 --x")?,
                        y: y.context("缺少 --y")?,
                    }
                }
                ProbeAction::Stick => {
                    ensure!(
                        key.is_none() && button.is_none() && value.is_none(),
                        "摇杆动作不接受按键、鼠标或力度参数"
                    );
                    InputProbeAction::Stick {
                        stick: side.context("缺少 --side")?.into(),
                        x: x.context("缺少 --x")?,
                        y: y.context("缺少 --y")?,
                    }
                }
                ProbeAction::Trigger => {
                    ensure!(
                        key.is_none() && button.is_none() && x.is_none() && y.is_none(),
                        "扳机动作不接受按键、鼠标或坐标参数"
                    );
                    InputProbeAction::Trigger {
                        trigger: side.context("缺少 --side")?.into(),
                        value: value.context("缺少 --value")?,
                    }
                }
            };
            let mode = match mode {
                Input::Foreground => InputMode::Foreground,
                Input::ForegroundLegacy => InputMode::ForegroundLegacy,
                Input::Background => InputMode::Background,
                Input::BackgroundPost => InputMode::BackgroundPost,
                Input::Xbox => InputMode::Xbox,
                Input::Ds4 => InputMode::Ds4,
            };
            let config = InputProbeConfig {
                mode,
                action,
                hold_ms: hold_ms as u32,
                start_delay_ms,
            };
            config.validate().map_err(anyhow::Error::msg)?;
            let window = od_platform::parse_game_window(&hwnd)?;
            ensure!(
                window.width == 1920 && window.height == 1080,
                "游戏客户区须为 1920×1080"
            );
            let mut native = NativeSession::for_game(
                &window,
                &paths.resources,
                capture(method),
                mode,
                InferenceDevice::Cpu,
            )?;
            let cancel = Arc::new(AtomicBool::new(false));
            let interrupt = cancel.clone();
            let signal = tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    interrupt.store(true, std::sync::atomic::Ordering::Release);
                }
            });
            let timer = cancel_after_ms.map(|delay| {
                let flag = cancel.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    flag.store(true, std::sync::atomic::Ordering::Release);
                })
            });
            let result = native.input_probe(config, cancel);
            signal.abort();
            if let Some(timer) = timer {
                timer.abort();
            }
            print(result?)
        }
        _ => unreachable!(),
    }
}
