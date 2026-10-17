use od_core::shortcuts::{ShortcutAction, ShortcutSettings};
use std::{
    sync::{
        Mutex,
        mpsc::{self, RecvTimeoutError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows_sys::Win32::UI::{
    Input::KeyboardAndMouse::{MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey},
    WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY},
};

enum Request {
    Configure(
        ShortcutSettings,
        tokio::sync::oneshot::Sender<Result<(), String>>,
    ),
    Close,
}

// 注册、消息读取和注销始终在同一线程；回调只负责投递控制命令。
pub struct GlobalHotkeys {
    sender: mpsc::Sender<Request>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl GlobalHotkeys {
    pub fn start(callback: impl Fn(ShortcutAction) + Send + 'static) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("global-shortcuts".into())
            .spawn(move || {
                let mut active = Vec::new();
                let mut next_id = 1;
                loop {
                    match receiver.recv_timeout(Duration::from_millis(10)) {
                        Ok(Request::Configure(settings, reply)) => {
                            // 每次使用不同 ID，已排队的旧消息不能被解释成新动作。
                            clear(&mut active);
                            let mut discarded: MSG = unsafe { std::mem::zeroed() };
                            while unsafe {
                                PeekMessageW(
                                    &mut discarded,
                                    std::ptr::null_mut(),
                                    WM_HOTKEY,
                                    WM_HOTKEY,
                                    PM_REMOVE,
                                )
                            } != 0
                            {}
                            let result = register(&settings, &mut active, &mut next_id);
                            if result.is_err() {
                                clear(&mut active);
                            }
                            let _ = reply.send(result);
                        }
                        Ok(Request::Close) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    let mut message: MSG = unsafe { std::mem::zeroed() };
                    while unsafe {
                        PeekMessageW(
                            &mut message,
                            std::ptr::null_mut(),
                            WM_HOTKEY,
                            WM_HOTKEY,
                            PM_REMOVE,
                        )
                    } != 0
                    {
                        if let Some((_, action)) =
                            active.iter().find(|(id, _)| *id == message.wParam as i32)
                        {
                            callback(*action);
                        }
                    }
                }
                clear(&mut active);
            })?;
        Ok(Self {
            sender,
            worker: Mutex::new(Some(worker)),
        })
    }

    pub async fn configure(&self, settings: ShortcutSettings) -> Result<(), String> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.sender
            .send(Request::Configure(settings, sender))
            .map_err(|_| "快捷键线程已退出".to_string())?;
        receiver
            .await
            .map_err(|_| "快捷键线程未返回注册结果".to_string())?
    }

    pub fn close(&self) {
        let _ = self.sender.send(Request::Close);
        if let Some(worker) = self.worker.lock().expect("快捷键线程锁").take() {
            let _ = worker.join();
        }
    }
}

impl Drop for GlobalHotkeys {
    fn drop(&mut self) {
        self.close();
    }
}

fn clear(active: &mut Vec<(i32, ShortcutAction)>) {
    for (id, _) in active.drain(..) {
        unsafe {
            UnregisterHotKey(std::ptr::null_mut(), id);
        }
    }
}

fn register(
    settings: &ShortcutSettings,
    active: &mut Vec<(i32, ShortcutAction)>,
    next_id: &mut i32,
) -> Result<(), String> {
    settings.validate().map_err(str::to_string)?;
    for (key, action) in [
        (settings.pause_resume, ShortcutAction::PauseResume),
        (settings.stop, ShortcutAction::Stop),
    ] {
        let Some(key) = key else {
            continue;
        };
        let id = *next_id;
        *next_id = if id >= 0xbfff { 1 } else { id + 1 };
        if unsafe { RegisterHotKey(std::ptr::null_mut(), id, MOD_NOREPEAT, key.virtual_key()) } == 0
        {
            return Err(format!(
                "{key:?} 注册失败，可能已被其他程序占用：{}",
                std::io::Error::last_os_error()
            ));
        }
        active.push((id, action));
    }
    Ok(())
}
