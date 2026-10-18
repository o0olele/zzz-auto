use crate::{
    ProcessStamp,
    windows::{OwnedHandle, checked_handle},
};
use anyhow::{Context, Result, ensure};
use od_core::{WindowInfo, after_done::AfterDoneAction};
use std::{
    os::windows::{ffi::OsStringExt, process::CommandExt},
    path::PathBuf,
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_NOT_ALL_ASSIGNED, GetLastError, SetLastError},
    Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    },
    System::{
        Power::SetSuspendState,
        SystemInformation::GetSystemDirectoryW,
        Threading::{CREATE_NO_WINDOW, GetCurrentProcess, OpenProcessToken},
    },
    UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE},
};

#[derive(Clone)]
pub struct AfterDoneTarget {
    pub process: ProcessStamp,
    pub window: WindowInfo,
}

impl AfterDoneTarget {
    pub fn capture(window: WindowInfo) -> Result<Self> {
        ensure!(crate::is_game_window(&window), "结束后动作目标不是绝区零");
        let current = crate::parse_game_window(&window.hwnd)?;
        ensure!(current.pid == window.pid, "游戏窗口归属已变化");
        Ok(Self {
            process: ProcessStamp::read(window.pid)?,
            window,
        })
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(!self.process.exited()?, "本次运行的游戏进程已退出或变化");
        let current = crate::parse_game_window(&self.window.hwnd)?;
        ensure!(
            current.pid == self.process.pid && self.process == ProcessStamp::read(current.pid)?,
            "本次运行的游戏进程或窗口已变化"
        );
        Ok(())
    }
}

pub fn system_shutdown_executable() -> Result<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    ensure!(
        length > 0 && (length as usize) < buffer.len(),
        "无法确定 Windows 系统目录"
    );
    Ok(
        PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length as usize]))
            .join("shutdown.exe"),
    )
}

struct ShutdownPrivilege {
    token: OwnedHandle,
    previous: TOKEN_PRIVILEGES,
}
impl Drop for ShutdownPrivilege {
    fn drop(&mut self) {
        unsafe {
            AdjustTokenPrivileges(
                self.token.0,
                0,
                &self.previous,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
            );
        }
    }
}

fn shutdown_privilege() -> Result<ShutdownPrivilege> {
    let mut raw = ptr::null_mut();
    ensure!(
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut raw,
            )
        } != 0,
        "无法读取电源操作权限：{}",
        std::io::Error::last_os_error()
    );
    let token = checked_handle(raw)?;
    let mut luid = Default::default();
    let name: Vec<u16> = "SeShutdownPrivilege\0".encode_utf16().collect();
    ensure!(
        unsafe { LookupPrivilegeValueW(ptr::null(), name.as_ptr(), &mut luid) } != 0,
        "无法查询电源操作权限"
    );
    let privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    let mut previous = TOKEN_PRIVILEGES::default();
    let mut length = 0;
    unsafe {
        SetLastError(0);
    }
    ensure!(
        unsafe {
            AdjustTokenPrivileges(
                token.0,
                0,
                &privileges,
                std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
                &mut previous,
                &mut length,
            )
        } != 0
            && unsafe { GetLastError() } != ERROR_NOT_ALL_ASSIGNED,
        "当前用户没有可用的电源操作权限"
    );
    Ok(ShutdownPrivilege { token, previous })
}

// 只由宿主在倒计时及收尾检查后调用；测试使用注入的执行器。
pub fn execute_after_done(action: AfterDoneAction, target: &AfterDoneTarget) -> Result<()> {
    target.validate()?;
    match action {
        AfterDoneAction::None => anyhow::bail!("没有可执行的结束后动作"),
        AfterDoneAction::CloseGame => {
            let hwnd = usize::from_str_radix(target.window.hwnd.trim_start_matches("0x"), 16)
                .context("游戏窗口句柄无效")?;
            ensure!(
                unsafe { PostMessageW(hwnd as _, WM_CLOSE, 0, 0) } != 0,
                "无法关闭游戏，可能存在权限差异：{}",
                std::io::Error::last_os_error()
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            while !target.process.exited()? {
                ensure!(
                    Instant::now() < deadline,
                    "游戏未响应关闭请求，请手动检查；未强制结束进程"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(())
        }
        AfterDoneAction::Shutdown => {
            let status = std::process::Command::new(system_shutdown_executable()?)
                .args(["/s", "/t", "0"])
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .context("无法提交 Windows 关机请求")?;
            ensure!(status.success(), "Windows 拒绝关机请求：{status}");
            Ok(())
        }
        AfterDoneAction::Hibernate | AfterDoneAction::Sleep => {
            let _privilege = shutdown_privilege()?;
            ensure!(
                unsafe { SetSuspendState(action == AfterDoneAction::Hibernate, false, false) },
                "Windows 拒绝休眠或睡眠请求：{}",
                std::io::Error::last_os_error()
            );
            Ok(())
        }
    }
}
