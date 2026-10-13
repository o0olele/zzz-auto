use anyhow::{Context, Result, bail, ensure};
use od_core::{KeyboardPermission, WindowInfo};
use std::{ffi::c_void, os::windows::io::AsRawHandle, path::Path, ptr};
use tokio::net::windows::named_pipe::{NamedPipeClient, NamedPipeServer, ServerOptions};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LocalFree, RECT},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, SECURITY_ATTRIBUTES,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER, TokenIntegrityLevel, TokenUser,
    },
    System::{
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            CreateMutexW, GetCurrentProcessId, OpenProcess, OpenProcessToken,
            PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
    },
    UI::WindowsAndMessaging::{
        EnumWindows, GetClientRect, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
        IsWindow, IsWindowVisible,
    },
};

pub struct OwnedHandle(HANDLE);

// HANDLE 的所有权可以移动到原生工作线程，访问仍保持串行。
unsafe impl Send for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn checked_handle(handle: HANDLE) -> Result<OwnedHandle> {
    ensure!(!handle.is_null(), "{}", std::io::Error::last_os_error());
    Ok(OwnedHandle(handle))
}

fn process(pid: u32) -> Result<OwnedHandle> {
    checked_handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) })
}

fn session(pid: u32) -> Result<u32> {
    let mut result = 0;
    ensure!(
        unsafe { ProcessIdToSessionId(pid, &mut result) } != 0,
        "无法检查进程会话"
    );
    Ok(result)
}

fn user_sid(pid: u32) -> Result<String> {
    let process = process(pid)?;
    let mut token = ptr::null_mut();
    ensure!(
        unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) } != 0,
        "无法读取进程身份"
    );
    let token = checked_handle(token)?;
    let mut length = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut length);
    }
    ensure!(length > 0, "进程没有可读取的用户身份");
    // u64 缓冲保证 TOKEN_USER 的指针对齐。
    let mut buffer = vec![0_u64; (length as usize).div_ceil(8)];
    ensure!(
        unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        } != 0,
        "无法读取用户 SID"
    );
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut sid = ptr::null_mut();
    ensure!(
        unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } != 0,
        "无法转换用户 SID"
    );
    let mut count = 0;
    unsafe {
        while *sid.add(count) != 0 {
            count += 1;
        }
        let result = String::from_utf16_lossy(std::slice::from_raw_parts(sid, count));
        LocalFree(sid.cast());
        Ok(result)
    }
}

pub fn verify_peer(pid: u32, expected_pid: u32) -> Result<()> {
    ensure!(pid == expected_pid, "命名管道连接来源与宿主进程不符");
    let own_pid = unsafe { GetCurrentProcessId() };
    ensure!(
        session(pid)? == session(own_pid)?,
        "命名管道连接来自另一交互会话"
    );
    ensure!(
        user_sid(pid)? == user_sid(own_pid)?,
        "命名管道连接来自另一用户"
    );
    Ok(())
}

pub fn verify_client(pipe: &NamedPipeServer, expected_pid: u32) -> Result<()> {
    let mut pid = 0;
    ensure!(
        unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle().cast(), &mut pid) } != 0,
        "无法检查管道客户端身份"
    );
    verify_peer(pid, expected_pid)
}

pub fn verify_server(pipe: &NamedPipeClient, expected_pid: u32) -> Result<()> {
    let mut pid = 0;
    ensure!(
        unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle().cast(), &mut pid) } != 0,
        "无法检查管道服务器身份"
    );
    verify_peer(pid, expected_pid)
}

pub fn create_private_pipe(name: &str) -> Result<NamedPipeServer> {
    ensure!(
        name.starts_with(r"\\.\pipe\zzz-auto-") && name.len() < 160,
        "管道名称不合法"
    );
    let sddl = wide(&format!(
        "D:P(A;;GA;;;{})",
        user_sid(unsafe { GetCurrentProcessId() })?
    ));
    let mut descriptor = ptr::null_mut();
    ensure!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } != 0,
        "无法配置管道权限"
    );
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let result = unsafe {
        ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                name,
                (&attributes as *const SECURITY_ATTRIBUTES)
                    .cast::<c_void>()
                    .cast_mut(),
            )
    };
    unsafe {
        LocalFree(descriptor);
    }
    result.context("创建私有命名管道失败")
}

pub struct GameLease {
    _handle: OwnedHandle,
}

impl GameLease {
    pub fn acquire(pid: u32) -> Result<Self> {
        let own_pid = unsafe { GetCurrentProcessId() };
        ensure!(session(pid)? == session(own_pid)?, "游戏属于另一交互会话");
        let name = wide(&format!("Local\\zzz-auto.game.{}.{}", session(pid)?, pid));
        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        let error = unsafe { GetLastError() };
        let handle = checked_handle(handle)?;
        ensure!(
            error != ERROR_ALREADY_EXISTS,
            "已有另一个 zzz-auto 引擎控制此游戏进程"
        );
        Ok(Self { _handle: handle })
    }
}

pub fn window_info(hwnd: usize) -> Result<WindowInfo> {
    let handle = hwnd as HWND;
    ensure!(unsafe { IsWindow(handle) } != 0, "游戏窗口已失效");
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(handle, &mut pid);
    }
    let process = process(pid)?;
    let mut executable = vec![0_u16; 32768];
    let mut length = executable.len() as u32;
    ensure!(
        unsafe { QueryFullProcessImageNameW(process.0, 0, executable.as_mut_ptr(), &mut length) }
            != 0,
        "无法读取游戏进程路径，检查权限是否匹配"
    );
    let executable = String::from_utf16_lossy(&executable[..length as usize]);
    let mut title = [0_u16; 512];
    let title_length = unsafe { GetWindowTextW(handle, title.as_mut_ptr(), title.len() as i32) };
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    ensure!(
        unsafe { GetClientRect(handle, &mut rect) } != 0,
        "无法读取窗口尺寸"
    );
    Ok(WindowInfo {
        hwnd: format!("{hwnd:x}"),
        pid,
        executable,
        title: String::from_utf16_lossy(&title[..title_length.max(0) as usize]),
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
        keyboard_permission: keyboard_permission(pid).ok(),
    })
}

pub fn process_integrity(pid: u32) -> Result<u32> {
    let target = process(pid)?;
    let mut token = ptr::null_mut();
    ensure!(
        unsafe { OpenProcessToken(target.0, TOKEN_QUERY, &mut token) } != 0,
        "无法读取进程键盘权限：{}",
        std::io::Error::last_os_error()
    );
    let token = checked_handle(token)?;
    let mut length = 0;
    unsafe {
        GetTokenInformation(
            token.0,
            TokenIntegrityLevel,
            ptr::null_mut(),
            0,
            &mut length,
        );
    }
    ensure!(
        length as usize >= std::mem::size_of::<TOKEN_MANDATORY_LABEL>(),
        "无法读取进程完整性级别"
    );
    let mut buffer = vec![0_u64; (length as usize).div_ceil(8)];
    ensure!(
        unsafe {
            GetTokenInformation(
                token.0,
                TokenIntegrityLevel,
                buffer.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        } != 0,
        "无法检查键盘输入权限"
    );
    let label = unsafe { &*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
    ensure!(!label.Label.Sid.is_null(), "进程没有完整性 SID");
    let count = unsafe { GetSidSubAuthorityCount(label.Label.Sid) };
    ensure!(
        !count.is_null() && unsafe { *count } > 0,
        "进程完整性 SID 无效"
    );
    let authority = unsafe { GetSidSubAuthority(label.Label.Sid, u32::from(*count - 1)) };
    ensure!(!authority.is_null(), "进程完整性级别无效");
    Ok(unsafe { *authority })
}

pub fn keyboard_permission(game_pid: u32) -> Result<KeyboardPermission> {
    Ok(KeyboardPermission::from_integrity(
        process_integrity(unsafe { GetCurrentProcessId() })?,
        process_integrity(game_pid)?,
    ))
}

pub fn check_keyboard_target(window: &WindowInfo, foreground: bool) -> Result<()> {
    let current = parse_game_window(&window.hwnd)?;
    ensure!(current.pid == window.pid, "游戏窗口身份已变化，请重新连接");
    ensure!(
        current.width == 1920 && current.height == 1080,
        "游戏客户区须为 1920×1080，请恢复窗口后重新连接"
    );
    let permission =
        keyboard_permission(current.pid).context("无法确认键鼠权限，请检查游戏与工具的运行权限")?;
    ensure!(
        permission.compatible,
        "游戏权限高于引擎，Windows 会限制键鼠输入。请从托盘退出工具，再以管理员身份启动工具；启动器正常启动不代表游戏没有管理员权限"
    );
    if foreground {
        let mut foreground_pid = 0;
        unsafe {
            GetWindowThreadProcessId(GetForegroundWindow(), &mut foreground_pid);
        }
        ensure!(
            foreground_pid == current.pid,
            "启动延时已结束，但游戏不在前台；未发送输入。请增加延时并在延时期间切回游戏"
        );
    }
    Ok(())
}

pub fn is_game_window(window: &WindowInfo) -> bool {
    Path::new(&window.executable)
        .file_name()
        .is_some_and(|name| {
            name.to_string_lossy()
                .eq_ignore_ascii_case("ZenlessZoneZero.exe")
        })
}

unsafe extern "system" fn collect_window(hwnd: HWND, data: isize) -> i32 {
    if unsafe { IsWindowVisible(hwnd) } != 0
        && let Ok(window) = window_info(hwnd as usize)
        && is_game_window(&window)
        && window.width > 0
        && window.height > 0
    {
        unsafe { &mut *(data as *mut Vec<WindowInfo>) }.push(window);
    }
    1
}

pub fn game_windows() -> Result<Vec<WindowInfo>> {
    let mut windows = Vec::new();
    ensure!(
        unsafe {
            EnumWindows(
                Some(collect_window),
                (&mut windows as *mut Vec<WindowInfo>) as isize,
            )
        } != 0,
        "窗口枚举失败"
    );
    Ok(windows)
}

pub fn parse_game_window(hwnd: &str) -> Result<WindowInfo> {
    let handle =
        usize::from_str_radix(hwnd.trim_start_matches("0x"), 16).context("窗口句柄无效")?;
    let window = window_info(handle)?;
    if !is_game_window(&window) {
        bail!("仅允许连接绝区零游戏进程");
    }
    Ok(window)
}
