use crate::windows::{OwnedHandle, checked_handle};
use anyhow::{Result, ensure};
use od_core::{InputMode, InputProbeAction, InputProbeConfig, MouseButton, WindowInfo};
use serde::{Deserialize, Serialize};
use windows_sys::Win32::{
    Foundation::{
        ERROR_INVALID_PARAMETER, FILETIME, GetLastError, POINT, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Graphics::Gdi::{ClientToScreen, ScreenToClient},
    System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        WaitForSingleObject,
    },
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

// PID 可复用；持久记录同时保存创建时间，恢复时重新持有进程句柄。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessStamp {
    pub pid: u32,
    pub created: u64,
}

fn creation(handle: &OwnedHandle) -> Result<u64> {
    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    ensure!(
        unsafe { GetProcessTimes(handle.0, created, exited, kernel, user) } != 0,
        "无法检查进程创建时间：{}",
        std::io::Error::last_os_error()
    );
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

impl ProcessStamp {
    pub fn read(pid: u32) -> Result<Self> {
        let handle =
            checked_handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) })?;
        Ok(Self {
            pid,
            created: creation(&handle)?,
        })
    }

    pub fn exited(&self) -> Result<bool> {
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                self.pid,
            )
        };
        if raw.is_null() {
            // 仅“进程不存在”能视为退出；拒绝访问不能视为崩溃。
            ensure!(
                unsafe { GetLastError() } == ERROR_INVALID_PARAMETER,
                "无法确认旧进程是否退出：{}",
                std::io::Error::last_os_error()
            );
            return Ok(true);
        }
        let handle = checked_handle(raw)?;
        if creation(&handle)? != self.created {
            return Ok(true);
        }
        match unsafe { WaitForSingleObject(handle.0, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(std::io::Error::last_os_error().into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRecoveryPlan {
    pub version: u32,
    pub engine: ProcessStamp,
    pub game: ProcessStamp,
    pub window: WindowInfo,
    pub config: InputProbeConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_inputs: Vec<InputProbeConfig>,
    pub engine_integrity: u32,
    pub run_id: Option<String>,
}

impl InputRecoveryPlan {
    pub fn validate(&self) -> Result<()> {
        ensure!(matches!(self.version, 1..=8), "输入恢复记录版本不支持");
        if self.version == 8 {
            ensure!(
                self.additional_inputs.len() <= 128
                    && self
                        .additional_inputs
                        .iter()
                        .all(|input| input.mode == self.config.mode),
                "v8 战斗恢复输入过多或输入模式不一致"
            );
            for input in &self.additional_inputs {
                input.validate().map_err(anyhow::Error::msg)?;
            }
        } else if self.version == 1 {
            ensure!(self.additional_inputs.is_empty(), "v1 恢复记录不能增加输入");
        } else if self.version == 2 {
            let expected = od_core::mail::MailConfig::recovery_inputs();
            ensure!(
                self.additional_inputs.len() == 1
                    && self.config == expected[0]
                    && self.additional_inputs[0] == expected[1],
                "v2 邮件恢复仅允许释放 Alt 和鼠标左键"
            );
        } else if self.version == 7 {
            let expected = od_core::basic_daily::recovery_inputs();
            ensure!(
                self.config == expected[0] && self.additional_inputs == expected[1..],
                "v7 首批日常恢复仅允许释放 Alt、鼠标左键、N、F、Escape 和 W"
            );
        } else if self.version == 6 {
            let expected = od_core::group_run::recovery_inputs();
            ensure!(
                self.config == expected[0] && self.additional_inputs == expected[1..],
                "v6 应用组恢复仅允许释放 Alt、鼠标左键、N 和 F"
            );
        } else if self.version == 5 {
            let expected = od_core::coffee_approach::recovery_inputs();
            ensure!(
                self.config == expected[0] && self.additional_inputs == expected[1..],
                "v5 咖啡路线恢复仅允许释放鼠标左键、N 和 F"
            );
        } else {
            ensure!(
                self.additional_inputs.is_empty()
                    && self.config == od_core::map_transport::MapConfig::recovery_inputs()[0],
                "v3 地图 / v4 咖啡恢复仅允许释放鼠标左键"
            );
        }
        self.config.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.engine.pid > 0
                && self.game.pid == self.window.pid
                && self.game.pid > 0
                && self.engine.pid != self.game.pid
                && self.engine.created > 0
                && self.game.created > 0,
            "输入恢复记录进程身份无效"
        );
        ensure!(crate::is_game_window(&self.window), "恢复目标不是绝区零");
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRelease {
    KeyUp(u16),
    MouseUp(MouseButton),
    GamepadOwnerExit,
}

// 恢复动作只能从已确认的单次诊断推导，永远不能投递按下、移动或重放。
pub fn input_release(config: InputProbeConfig) -> Result<InputRelease> {
    config.validate().map_err(anyhow::Error::msg)?;
    if config.mode.is_gamepad() {
        return Ok(InputRelease::GamepadOwnerExit);
    }
    Ok(match config.action {
        InputProbeAction::Key { key } => InputRelease::KeyUp(key as u16),
        InputProbeAction::MouseButton { button, .. } => InputRelease::MouseUp(button),
        _ => unreachable!("已验证的键鼠诊断没有手柄动作"),
    })
}

fn release_scan(key: u16) -> (u16, bool) {
    let scan = unsafe { MapVirtualKeyW(u32::from(key), MAPVK_VK_TO_VSC_EX) };
    // 部分键盘布局对方向键仍只返回低位扫描码，须保留独立导航键的扩展位。
    let extended = scan & 0xff00 == 0xe000
        || matches!(key, 0x21..=0x28 | 0x2c..=0x2e | 0x5b..=0x5d | 0x6f | 0x90 | 0xa3 | 0xa5);
    ((scan & 0xff) as u16, extended)
}

pub fn win32_release_event(release: InputRelease) -> Result<INPUT> {
    let mut event = INPUT::default();
    match release {
        InputRelease::KeyUp(key) => {
            ensure!((1..=254).contains(&key), "恢复 VK 无效");
            event.r#type = INPUT_KEYBOARD;
            let (scan, extended) = release_scan(key);
            event.Anonymous.ki = KEYBDINPUT {
                wVk: key,
                wScan: scan,
                dwFlags: KEYEVENTF_KEYUP | if extended { KEYEVENTF_EXTENDEDKEY } else { 0 },
                ..Default::default()
            };
        }
        InputRelease::MouseUp(button) => {
            event.r#type = INPUT_MOUSE;
            let (flags, data) = match button {
                MouseButton::Left => (MOUSEEVENTF_LEFTUP, 0),
                MouseButton::Right => (MOUSEEVENTF_RIGHTUP, 0),
                MouseButton::Middle => (MOUSEEVENTF_MIDDLEUP, 0),
                MouseButton::X1 => (MOUSEEVENTF_XUP, 1),
                MouseButton::X2 => (MOUSEEVENTF_XUP, 2),
            };
            event.Anonymous.mi = MOUSEINPUT {
                dwFlags: flags,
                mouseData: data,
                ..Default::default()
            };
        }
        InputRelease::GamepadOwnerExit => anyhow::bail!("手柄恢复不能生成键鼠事件"),
    }
    Ok(event)
}

fn recovery_windows(pid: u32) -> Result<Vec<usize>> {
    struct Targets {
        pid: u32,
        windows: Vec<usize>,
    }
    unsafe extern "system" fn collect(hwnd: *mut std::ffi::c_void, context: isize) -> i32 {
        let targets = unsafe { &mut *(context as *mut Targets) };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if pid == targets.pid {
            targets.windows.push(hwnd as usize);
        }
        1
    }
    let mut targets = Targets {
        pid,
        windows: Vec::new(),
    };
    ensure!(
        unsafe { EnumWindows(Some(collect), (&mut targets as *mut Targets) as isize) } != 0,
        "无法枚举原游戏的输入窗口"
    );
    ensure!(
        !targets.windows.is_empty() && targets.windows.len() <= 16,
        "游戏窗口数量异常，未发送恢复消息"
    );
    Ok(targets.windows)
}

pub fn recover_input(plan: &InputRecoveryPlan) -> Result<String> {
    plan.validate()?;
    if matches!(plan.version, 2 | 5) {
        let mut errors = Vec::new();
        for config in std::iter::once(plan.config).chain(plan.additional_inputs.iter().copied()) {
            let mut single = plan.clone();
            single.version = 1;
            single.config = config;
            single.additional_inputs.clear();
            if let Err(error) = recover_input(&single) {
                errors.push(format!("{error:#}"));
            }
        }
        ensure!(errors.is_empty(), "{}", errors.join("；"));
        return Ok(if plan.version == 5 {
            "已释放咖啡路线记录中的鼠标左键、N 和 F；请确认游戏状态再重连，不会重放传送或点单"
                .into()
        } else {
            "已释放邮件记录中的 Alt 和鼠标左键；请确认游戏状态再重连，不会续跑或重复领取".into()
        });
    }
    ensure!(plan.engine.exited()?, "旧引擎仍在运行，不能发送恢复输入");
    let release = input_release(plan.config)?;
    if release == InputRelease::GamepadOwnerExit {
        // ViGEmClient 的契约：拥有进程终止时，驱动移除其虚拟设备。
        // 不创建新手柄冒充对旧设备的复位；真实驱动与游戏响应另列实机验收。
        return Ok(
            "已确认旧引擎退出，ViGEm 将移除该进程拥有的虚拟手柄；请确认游戏已停止接收手柄输入"
                .into(),
        );
    }
    let background = matches!(
        plan.config.mode,
        InputMode::Background | InputMode::BackgroundPost
    );
    let mut _lease = None;
    if plan.game.exited()? {
        if background {
            return Ok("原游戏进程已退出，无需向旧窗口发送释放消息".into());
        }
    } else {
        _lease = Some(crate::GameLease::acquire(plan.game.pid)?);
        let current = crate::parse_game_window(&plan.window.hwnd)?;
        ensure!(
            current.pid == plan.game.pid && ProcessStamp::read(current.pid)? == plan.game,
            "游戏窗口身份已变化，不能向新窗口发送恢复消息"
        );
        ensure!(
            crate::keyboard_permission(current.pid)?.compatible,
            "恢复进程权限不足，请授权提升恢复进程"
        );
    }
    ensure!(
        crate::process_integrity(std::process::id())? >= plan.engine_integrity,
        "恢复进程权限低于原引擎，未发送释放"
    );
    if background {
        let hwnd = usize::from_str_radix(&plan.window.hwnd, 16)? as *mut std::ffi::c_void;
        let (message, wparam, lparam) = match release {
            InputRelease::KeyUp(key) => {
                let (scan, extended) = release_scan(key);
                (
                    WM_KEYUP,
                    usize::from(key),
                    (0xc0000001_u32 | (u32::from(scan) << 16) | if extended { 1 << 24 } else { 0 })
                        as isize,
                )
            }
            InputRelease::MouseUp(button) => {
                let InputProbeAction::MouseButton { x, y, .. } = plan.config.action else {
                    unreachable!()
                };
                let (msg, button) = match button {
                    MouseButton::Left => (WM_LBUTTONUP, 0),
                    MouseButton::Right => (WM_RBUTTONUP, 0),
                    MouseButton::Middle => (WM_MBUTTONUP, 0),
                    MouseButton::X1 => (WM_XBUTTONUP, 1 << 16),
                    MouseButton::X2 => (WM_XBUTTONUP, 2 << 16),
                };
                (msg, button, ((y as u32) << 16 | x as u32) as isize)
            }
            _ => unreachable!(),
        };
        // Maa 可能把输入路由至游戏弹窗；覆盖同一进程仍存活的顶层窗口。
        for target in recovery_windows(plan.game.pid)? {
            let target = target as *mut std::ffi::c_void;
            let mut target_lparam = lparam;
            if matches!(release, InputRelease::MouseUp(_)) && target != hwnd {
                let InputProbeAction::MouseButton { x, y, .. } = plan.config.action else {
                    unreachable!()
                };
                let mut point = POINT { x, y };
                ensure!(
                    unsafe { ClientToScreen(hwnd, &mut point) } != 0
                        && unsafe { ScreenToClient(target, &mut point) } != 0,
                    "无法转换原游戏弹窗坐标"
                );
                ensure!(
                    i16::try_from(point.x).is_ok() && i16::try_from(point.y).is_ok(),
                    "弹窗坐标超出消息范围"
                );
                target_lparam =
                    ((u32::from(point.y as u16) << 16) | u32::from(point.x as u16)) as isize;
            }
            let post = plan.config.mode == InputMode::BackgroundPost;
            // 同步消息不能作为 PostMessage 队列栅栏，否则 KeyUp 可能抢在旧 KeyDown 前执行。
            ensure!(
                unsafe {
                    SendMessageTimeoutW(
                        target,
                        if post { WM_NULL } else { message },
                        if post { 0 } else { wparam },
                        if post { 0 } else { target_lparam },
                        SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
                        1500,
                        std::ptr::null_mut(),
                    )
                } != 0,
                "游戏窗口无响应或拒绝恢复，请停止使用自动化并手动恢复输入"
            );
            if post {
                ensure!(
                    unsafe { PostMessageW(target, message, wparam, target_lparam) } != 0,
                    "释放消息无法入队，请停止使用自动化并手动恢复输入"
                );
            }
        }
        if plan.config.mode == InputMode::BackgroundPost {
            return Ok(
                "已按原队列顺序排入释放消息，尚未确认游戏消费；请确认游戏恢复后再手动重连".into(),
            );
        }
    } else {
        let event = win32_release_event(release)?;
        ensure!(
            unsafe { SendInput(1, &event, std::mem::size_of::<INPUT>() as i32) } == 1,
            "Windows 拒绝释放输入，请停止使用自动化并手动恢复输入"
        );
    }
    Ok("已发送记录中的键鼠释放；请确认游戏恢复后再重新连接，之前的动作不会重放".into())
}
