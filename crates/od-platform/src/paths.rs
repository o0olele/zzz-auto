use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

pub struct DesktopLayout {
    pub root: PathBuf,
    pub engine: PathBuf,
    pub data: PathBuf,
    pub portable: bool,
}

pub fn desktop_layout(
    executable: &Path,
    resource_directory: &Path,
    user_data_directory: &Path,
    development_root: Option<&Path>,
) -> Result<DesktopLayout> {
    let directory = executable.parent().context("无法定位程序目录")?;
    ensure!(directory.is_absolute(), "程序目录必须是绝对路径");
    if directory.join("portable.flag").is_file() {
        return Ok(DesktopLayout {
            root: directory.to_path_buf(),
            engine: directory.join("zzz-auto-engine.exe"),
            data: directory.join("data"),
            portable: true,
        });
    }
    if let Some(root) = development_root {
        return Ok(DesktopLayout {
            root: root.to_path_buf(),
            engine: root.join("target/debug/zzz-engine.exe"),
            data: user_data_directory.to_path_buf(),
            portable: false,
        });
    }
    Ok(DesktopLayout {
        root: resource_directory.to_path_buf(),
        engine: directory.join("zzz-auto-engine.exe"),
        data: user_data_directory.to_path_buf(),
        portable: false,
    })
}
