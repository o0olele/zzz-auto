use od_protocol::ApiError;
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, OFN_DONTADDTORECENT, OFN_FILEMUSTEXIST,
    OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};

pub fn pick(owner: isize) -> Result<Option<String>, ApiError> {
    let mut file = vec![0_u16; 32768];
    let filter: Vec<u16> = "运行资源 ZIP\0*.zip\0\0".encode_utf16().collect();
    let title: Vec<u16> = "选择运行资源包\0".encode_utf16().collect();
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
                "resource_picker_failed",
                format!("无法选择资源包：{error}"),
            ))
        };
    }
    let length = file
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(file.len());
    String::from_utf16(&file[..length])
        .map(Some)
        .map_err(|_| ApiError::new("invalid_resources", "资源包路径不是有效 Unicode"))
}
