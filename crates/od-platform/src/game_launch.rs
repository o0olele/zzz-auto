use crate::windows::{OwnedHandle, checked_handle};
use anyhow::{Context, Result, ensure};
use od_core::game_launch::GameLaunchSettings;
use std::{
    ffi::OsString,
    io::{Read, Seek, SeekFrom},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_NO_MORE_FILES, GetLastError, RPC_E_CHANGED_MODE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    System::{
        Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        Threading::{CreateMutexW, GetProcessId, ReleaseMutex, WaitForSingleObject},
    },
    UI::{
        Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NO_CONSOLE, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
            SHELLEXECUTEINFOW, ShellExecuteExW,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

// 实际启动与测试替身共用入口；不通过桌面协议提供可替换执行器。
pub trait GameLauncher: Send {
    fn launch(&mut self, settings: &GameLaunchSettings) -> Result<u32>;
}

#[derive(Default)]
pub struct NativeGameLauncher {
    process: Option<GameProcess>,
}

impl GameLauncher for NativeGameLauncher {
    fn launch(&mut self, settings: &GameLaunchSettings) -> Result<u32> {
        let name: Vec<_> = "Local\\zzz-auto-game-launch\0".encode_utf16().collect();
        let handle = checked_handle(unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) })?;
        let status = unsafe { WaitForSingleObject(handle.0, 0) };
        ensure!(
            matches!(
                status,
                WAIT_OBJECT_0 | windows_sys::Win32::Foundation::WAIT_ABANDONED
            ),
            "其他引擎正在启动游戏，请稍后检查游戏窗口"
        );
        struct LaunchGuard(OwnedHandle);
        impl Drop for LaunchGuard {
            fn drop(&mut self) {
                unsafe { ReleaseMutex(self.0.0) };
            }
        }
        let _guard = LaunchGuard(handle);
        if let Some(process) = &self.process {
            ensure!(
                process.exited()?,
                "已提交的游戏仍在运行或启动中，请勿重复启动"
            );
        }
        ensure!(
            !game_process_exists()?,
            "已有游戏进程，请等待窗口出现并手动连接"
        );
        let process = GameProcess::spawn(settings)?;
        let pid = process.pid;
        self.process = Some(process);
        Ok(pid)
    }
}

pub fn game_process_exists() -> Result<bool> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    ensure!(
        snapshot != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
        "无法读取游戏进程列表：{}",
        std::io::Error::last_os_error()
    );
    let snapshot = checked_handle(snapshot)?;
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut found = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    while found != 0 {
        let length = entry
            .szExeFile
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(entry.szExeFile.len());
        if String::from_utf16_lossy(&entry.szExeFile[..length])
            .eq_ignore_ascii_case("ZenlessZoneZero.exe")
        {
            return Ok(true);
        }
        found = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    ensure!(
        unsafe { GetLastError() } == ERROR_NO_MORE_FILES,
        "无法确认现有游戏进程"
    );
    Ok(false)
}

// 只持有查询句柄；释放它不会关闭游戏。
pub struct GameProcess {
    handle: OwnedHandle,
    pub pid: u32,
}

impl GameProcess {
    pub fn spawn(settings: &GameLaunchSettings) -> Result<Self> {
        settings.validate().map_err(anyhow::Error::msg)?;
        ensure!(!settings.executable.is_empty(), "请先保存游戏路径");
        let path = dunce::canonicalize(&settings.executable)
            .context("游戏文件不存在或无法读取，请重新选择路径")?;
        ensure!(
            path.is_file()
                && path
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("ZenlessZoneZero.exe")),
            "游戏真实路径必须指向 ZenlessZoneZero.exe"
        );
        GameLaunchSettings {
            executable: path.to_str().context("游戏路径不是有效 Unicode")?.into(),
            ..settings.clone()
        }
        .validate()
        .map_err(anyhow::Error::msg)?;
        validate_game_binary(&path)?;
        let directory = path.parent().context("游戏目录无效")?;
        let file = wide(&path)?;
        let directory = wide(directory)?;
        let arguments = crate::windows_arguments(
            &settings
                .arguments()
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>(),
        )?;
        let parameters = wide(Path::new(&arguments))?;
        let verb: Vec<_> = "open\0".encode_utf16().collect();
        let initialized = unsafe {
            CoInitializeEx(
                ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        };
        ensure!(
            initialized >= 0 || initialized == RPC_E_CHANGED_MODE,
            "无法初始化游戏启动线程"
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
            lpFile: file.as_ptr(),
            lpDirectory: directory.as_ptr(),
            lpParameters: parameters.as_ptr(),
            nShow: SW_SHOWNORMAL,
            ..Default::default()
        };
        if unsafe { ShellExecuteExW(&mut launch) } == 0 {
            return Err(std::io::Error::last_os_error())
                .context("Windows 未启动游戏；请核对路径或取消的授权，也可手动打开游戏");
        }
        let handle = checked_handle(launch.hProcess)
            .context("Windows 未返回游戏进程句柄，请先检查游戏是否已经打开")?;
        let pid = unsafe { GetProcessId(handle.0) };
        ensure!(pid != 0, "无法读取游戏进程编号，请先检查游戏窗口");
        Ok(Self { handle, pid })
    }

    pub fn exited(&self) -> Result<bool> {
        match unsafe { WaitForSingleObject(self.handle.0, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(std::io::Error::last_os_error().into()),
        }
    }
}

fn wide(path: &Path) -> Result<Vec<u16>> {
    let mut units: Vec<_> = path.as_os_str().encode_wide().collect();
    ensure!(!units.contains(&0), "游戏路径或参数包含 NUL");
    units.push(0);
    Ok(units)
}

fn validate_game_binary(path: &Path) -> Result<()> {
    // 在交给 Windows Shell 前排除目录、文本和错误架构，避免系统弹出关联程序选择器。
    let mut file = std::fs::File::open(path)?;
    let mut dos = [0_u8; 64];
    file.read_exact(&mut dos)
        .context("游戏文件不是有效的 Windows x64 程序")?;
    ensure!(&dos[..2] == b"MZ", "游戏文件不是 Windows 程序");
    let offset = u32::from_le_bytes(dos[60..64].try_into().expect("固定 DOS 字段")) as u64;
    ensure!(
        offset >= 64
            && offset
                .checked_add(26)
                .is_some_and(|end| end <= file.metadata().map(|m| m.len()).unwrap_or(0)),
        "游戏 PE 文件头无效"
    );
    file.seek(SeekFrom::Start(offset))?;
    let mut pe = [0_u8; 26];
    file.read_exact(&mut pe)?;
    ensure!(
        &pe[..4] == b"PE\0\0"
            && u16::from_le_bytes([pe[4], pe[5]]) == 0x8664
            && u16::from_le_bytes([pe[22], pe[23]]) & 2 != 0
            && u16::from_le_bytes([pe[24], pe[25]]) == 0x20b,
        "游戏文件必须为 Windows x64 可执行程序"
    );
    Ok(())
}
