use crate::windows::{OwnedHandle, checked_handle};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::{OsStr, OsString},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        process::CommandExt,
    },
    path::Path,
    process::{Child, Command, Stdio},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{GetLastError, RPC_E_CHANGED_MODE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        Threading::{GetProcessId, WaitForSingleObject},
    },
    UI::{
        Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NO_CONSOLE, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
            SHELLEXECUTEINFOW, ShellExecuteExW,
        },
        WindowsAndMessaging::SW_HIDE,
    },
};

// 按 Windows CommandLineToArgvW / Rust 参数规则转义；不经过 PowerShell 或 cmd。
pub fn windows_arguments(args: &[OsString]) -> Result<OsString> {
    let mut output = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            output.push(32);
        }
        output.push(34);
        let mut slashes = 0;
        for unit in arg.encode_wide() {
            ensure!(unit != 0, "启动参数不能包含 NUL");
            if unit == 92 {
                slashes += 1;
                continue;
            }
            output.extend(std::iter::repeat_n(
                92,
                if unit == 34 { slashes * 2 + 1 } else { slashes },
            ));
            slashes = 0;
            output.push(unit);
        }
        output.extend(std::iter::repeat_n(92, slashes * 2));
        output.push(34);
    }
    Ok(OsString::from_wide(&output))
}

fn wide(value: &OsStr) -> Result<Vec<u16>> {
    let mut units: Vec<_> = value.encode_wide().collect();
    ensure!(!units.contains(&0), "启动路径不能包含 NUL");
    units.push(0);
    Ok(units)
}

enum ProcessHandle {
    Normal(Child),
    Elevated(OwnedHandle),
}

pub struct EngineProcess {
    handle: ProcessHandle,
    pid: u32,
}

impl EngineProcess {
    // 调用方只能传入可信宿主定位的引擎；前端不提供可执行路径或启动参数。
    pub fn spawn(executable: &Path, args: &[OsString], elevated: bool) -> Result<Self> {
        // Shell 不一定接受 std::fs::canonicalize 的扩展路径前缀。
        let executable = dunce::canonicalize(executable).context("随附引擎不存在")?;
        ensure!(executable.is_file(), "引擎路径不是文件");
        if !elevated {
            let child = Command::new(&executable)
                .args(args)
                .creation_flags(0x08000000)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let pid = child.id();
            return Ok(Self {
                handle: ProcessHandle::Normal(child),
                pid,
            });
        }
        let parameters = wide(&windows_arguments(args)?)?;
        let executable = wide(executable.as_os_str())?;
        let verb = wide(OsStr::new("runas"))?;
        let initialized = unsafe {
            CoInitializeEx(
                ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        };
        ensure!(
            initialized >= 0 || initialized == RPC_E_CHANGED_MODE,
            "无法初始化提权启动线程：{initialized}"
        );
        struct ComGuard(bool);
        impl Drop for ComGuard {
            fn drop(&mut self) {
                if self.0 {
                    unsafe { CoUninitialize() };
                }
            }
        }
        let _com = ComGuard(initialized >= 0);
        let mut launch = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS
                | SEE_MASK_NOASYNC
                | SEE_MASK_FLAG_NO_UI
                | SEE_MASK_NO_CONSOLE,
            lpVerb: verb.as_ptr(),
            lpFile: executable.as_ptr(),
            lpParameters: parameters.as_ptr(),
            nShow: SW_HIDE,
            ..Default::default()
        };
        if unsafe { ShellExecuteExW(&mut launch) } == 0 {
            return Err(std::io::Error::from_raw_os_error(unsafe { GetLastError() } as i32).into());
        }
        let handle = checked_handle(launch.hProcess).context("Windows 未返回引擎进程句柄")?;
        let pid = unsafe { GetProcessId(handle.0) };
        ensure!(pid != 0, "无法取得提升后的引擎进程 ID");
        Ok(Self {
            handle: ProcessHandle::Elevated(handle),
            pid,
        })
    }

    pub fn id(&self) -> u32 {
        self.pid
    }

    pub fn exited(&mut self) -> Result<bool> {
        match &mut self.handle {
            ProcessHandle::Normal(child) => Ok(child.try_wait()?.is_some()),
            ProcessHandle::Elevated(handle) => match unsafe { WaitForSingleObject(handle.0, 0) } {
                WAIT_OBJECT_0 => Ok(true),
                WAIT_TIMEOUT => Ok(false),
                _ => Err(std::io::Error::last_os_error().into()),
            },
        }
    }
    // 不实现强制 kill；断开托管管道后由引擎停止任务并自行退出。
}
