#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod host;
mod resources;
mod session;

use host::Host;
use od_protocol::{ApiError, Command, DesktopUpdate, Payload};
use tauri::{
    Manager, WindowEvent,
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
};

fn main_window(window: &tauri::WebviewWindow) -> Result<(), ApiError> {
    let url = window
        .url()
        .map_err(|e| ApiError::new("forbidden_window", e.to_string()))?;
    let local = url.scheme() == "tauri" && url.host_str() == Some("localhost")
        || url.scheme() == "http" && url.host_str() == Some("tauri.localhost")
        || cfg!(debug_assertions)
            && url.scheme() == "http"
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            && url.port() == Some(1420);
    if window.label() == "main" && local {
        Ok(())
    } else {
        Err(ApiError::new("forbidden_window", "该窗口没有引擎访问权限"))
    }
}

#[tauri::command]
async fn engine_request(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Host>,
    command: Command,
) -> Result<Payload, ApiError> {
    main_window(&window)?;
    if matches!(command, Command::Shutdown) {
        return Err(ApiError::new("forbidden_command", "请使用托盘退出应用"));
    }
    state.request(command).await
}

#[tauri::command]
async fn reconnect_engine(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, Host>,
) -> Result<(), ApiError> {
    main_window(&window)?;
    state.start(&app).await
}

#[tauri::command]
async fn elevate_engine(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, Host>,
) -> Result<(), ApiError> {
    main_window(&window)?;
    state.elevate(&app).await
}

#[tauri::command]
async fn recover_inputs(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, Host>,
) -> Result<String, ApiError> {
    main_window(&window)?;
    state.recover(&app).await
}

#[tauri::command]
fn runtime_resource_state(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Host>,
) -> Result<od_core::runtime_resources::DesktopRuntimeResources, ApiError> {
    main_window(&window)?;
    Ok(state.resources.snapshot())
}

#[tauri::command]
fn start_runtime_resource_job(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, Host>,
    request: od_core::runtime_resources::RuntimeResourceRequest,
) -> Result<od_core::runtime_resources::DesktopRuntimeResources, ApiError> {
    main_window(&window)?;
    state.start_resources(&app, request)
}

#[tauri::command]
fn subscribe_engine(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Host>,
    channel: tauri::ipc::Channel<DesktopUpdate>,
) -> Result<(), ApiError> {
    main_window(&window)?;
    *state
        .channel
        .lock()
        .map_err(|_| ApiError::new("channel_error", "状态通道不可用"))? = Some(channel);
    Ok(())
}

fn main() {
    let app = tauri::Builder::default()
        .manage(Host::new())
        .invoke_handler(tauri::generate_handler![
            engine_request,
            reconnect_engine,
            elevate_engine,
            recover_inputs,
            runtime_resource_state,
            start_runtime_resource_job,
            subscribe_engine
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let show = MenuItem::with_id(app, "show", "打开主窗口", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "停止并退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let pixels: Vec<u8> = (0..32 * 32)
                .flat_map(|index| {
                    let x = index % 32;
                    let y = index / 32;
                    if (7..25).contains(&x)
                        && ((8..12).contains(&y)
                            || (20..24).contains(&y)
                            || (12..20).contains(&y) && (x + y > 29) && (x + y < 35))
                    {
                        [255, 255, 255, 255]
                    } else {
                        [15, 108, 189, 255]
                    }
                })
                .collect();
            TrayIconBuilder::new()
                .icon(tauri::image::Image::new_owned(pixels, 32, 32))
                .tooltip("绝区零一条龙 · Rust 原型")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            match app.state::<Host>().shutdown().await {
                                Ok(()) => app.exit(0),
                                Err(error) => {
                                    app.state::<Host>().report_error(error);
                                    if let Some(window) = app.get_webview_window("main") {
                                        let _ = window.show();
                                        let _ = window.set_focus();
                                    }
                                }
                            }
                        });
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(event, TrayIconEvent::DoubleClick { .. })
                        && let Some(window) = tray.app_handle().get_webview_window("main")
                    {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                })
                .build(app)?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let initial = handle.state::<Host>().start(&handle).await;
                if let Err(error) = initial {
                    eprintln!("{error}");
                }
                loop {
                    if handle.state::<Host>().resources.switching() {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        continue;
                    }
                    let (privilege, snapshot) = handle.state::<Host>().poll(&handle).await;
                    let update = match snapshot {
                        Ok(Payload::Snapshot(snapshot)) => DesktopUpdate::Snapshot(snapshot),
                        Err(error) => DesktopUpdate::Offline(error),
                        _ => unreachable!(),
                    };
                    if let Ok(mut channel) = handle.state::<Host>().channel.lock()
                        && channel.as_ref().is_some_and(|channel| {
                            if let Some(privilege) = privilege {
                                let _ = channel.send(DesktopUpdate::Privilege(privilege));
                            }
                            channel.send(update).is_err()
                        })
                    {
                        channel.take();
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Tauri 桌面初始化失败");
    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            let _ = tauri::async_runtime::block_on(app.state::<Host>().shutdown());
        }
    });
}
