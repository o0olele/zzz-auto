use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GameLaunchSettings {
    pub executable: String,
    pub use_arguments: bool,
    pub fullscreen: bool,
    pub borderless: bool,
    pub monitor: u8,
}

impl Default for GameLaunchSettings {
    fn default() -> Self {
        Self {
            executable: String::new(),
            use_arguments: false,
            fullscreen: false,
            borderless: false,
            monitor: 1,
        }
    }
}

impl GameLaunchSettings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=4).contains(&self.monitor) {
            return Err("显示器序号必须为 1..4");
        }
        let path = &self.executable;
        if path.is_empty() {
            return Ok(());
        }
        // 配置可在游戏尚未安装时导入；真实文件与进程在启动时检查。
        let bytes = path.as_bytes();
        if path.encode_utf16().count() > 32700
            || path
                .chars()
                .any(|c| c.is_control() || matches!(c, '"' | '<' | '>' | '|' | '?' | '*'))
            || bytes.len() < 4
            || !bytes[0].is_ascii_alphabetic()
            || bytes[1] != b':'
            || !matches!(bytes[2], b'\\' | b'/')
            || path[2..].contains(':')
            || path
                .split(['\\', '/'])
                .any(|part| part == "." || part == ".." || part.ends_with([' ', '.']))
            || !path
                .rsplit(['\\', '/'])
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("ZenlessZoneZero.exe"))
        {
            return Err("请选择本地绝对路径中的 ZenlessZoneZero.exe");
        }
        Ok(())
    }

    pub fn arguments(&self) -> Vec<String> {
        if !self.use_arguments {
            return Vec::new();
        }
        let mut args = vec![
            "-screen-width".into(),
            "1920".into(),
            "-screen-height".into(),
            "1080".into(),
            "-screen-fullscreen".into(),
            if self.fullscreen { "1" } else { "0" }.into(),
            "-monitor".into(),
            self.monitor.to_string(),
        ];
        if self.borderless {
            args.push("-popupwindow".into());
        }
        args
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GameLaunchResult {
    pub instance_id: u32,
    pub pid: u32,
}
