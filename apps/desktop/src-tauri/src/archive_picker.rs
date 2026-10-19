use od_protocol::ApiError;
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, OFN_DONTADDTORECENT, OFN_FILEMUSTEXIST,
    OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};

pub fn pick(owner: isize) -> Result<Option<String>, ApiError> {
    pick_file(owner, "运行资源 ZIP\0*.zip\0\0", "选择运行资源包\0").map_err(|error| {
        ApiError::new(
            if error.code == "invalid_path" {
                "invalid_resources"
            } else {
                "resource_picker_failed"
            },
            error.message,
        )
    })
}

pub fn pick_game(owner: isize) -> Result<Option<String>, ApiError> {
    let result = pick_file(
        owner,
        "绝区零游戏程序\0ZenlessZoneZero.exe\0\0",
        "选择 ZenlessZoneZero.exe\0",
    )?;
    if let Some(path) = &result {
        let settings = od_core::game_launch::GameLaunchSettings {
            executable: path.clone(),
            ..Default::default()
        };
        settings
            .validate()
            .map_err(|error| ApiError::new("invalid_game_launch", error))?;
    }
    Ok(result)
}

pub fn pick_config_source(owner: isize) -> Result<Option<String>, ApiError> {
    let Some(file) = pick_file(
        owner,
        "旧版账号配置\0one_dragon.yml\0\0",
        "选择旧程序 config 下的 one_dragon.yml\0",
    )?
    else {
        return Ok(None);
    };
    let path = std::path::Path::new(&file);
    let valid = path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("one_dragon.yml"))
        && path
            .parent()
            .and_then(|parent| parent.file_name())
            .is_some_and(|name| name.eq_ignore_ascii_case("config"));
    if !valid {
        return Err(ApiError::new(
            "config_picker_failed",
            "请选择旧程序 config 目录中的 one_dragon.yml",
        ));
    }
    path.parent()
        .and_then(|parent| parent.parent())
        .and_then(|root| root.to_str())
        .map(|root| Some(root.to_owned()))
        .ok_or_else(|| ApiError::new("config_picker_failed", "无法确定旧程序目录"))
}

fn pick_file(owner: isize, filter: &str, title: &str) -> Result<Option<String>, ApiError> {
    let mut file = vec![0_u16; 32768];
    let filter: Vec<u16> = filter.encode_utf16().collect();
    let title: Vec<u16> = title.encode_utf16().collect();
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner as _,
        lpstrFilter: filter.as_ptr(),
        nFilterIndex: 1,
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: file.len() as u32,
        lpstrTitle: title.as_ptr(),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_DONTADDTORECENT,
        ..Default::default()
    };
    // 缓冲区和窗口句柄在同步调用期间保持有效，取消不产生安装任务。
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        let error = unsafe { CommDlgExtendedError() };
        return if error == 0 {
            Ok(None)
        } else {
            Err(ApiError::new(
                "file_picker_failed",
                format!("无法打开文件选择器：{error}"),
            ))
        };
    }
    let length = file
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(file.len());
    String::from_utf16(&file[..length])
        .map(Some)
        .map_err(|_| ApiError::new("invalid_path", "文件路径不是有效 Unicode"))
}
