use anyhow::{Context, Result, ensure};
use od_core::SdkStatus;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use windows_sys::Win32::System::LibraryLoader::{
    AddDllDirectory, GetModuleFileNameW, GetModuleHandleW, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
    SetDefaultDllDirectories,
};

pub const SDK_VERSION: &str = "5.14.3";
pub const BINDING_VERSION: &str = "1.25.2";
static LOADED_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Deserialize)]
struct Manifest {
    binaries: Vec<Binary>,
}
#[derive(Deserialize)]
struct Binary {
    path: String,
    sha256: String,
    bytes: u64,
}

pub fn load_sdk(directory: &Path, log_directory: &Path) -> Result<SdkStatus> {
    let directory = directory
        .canonicalize()
        .context("Maa SDK 未安装，请执行 scripts/setup-sdk.ps1")?;
    let mut loaded = LOADED_PATH
        .lock()
        .map_err(|_| anyhow::anyhow!("SDK 初始化锁失效"))?;
    if let Some(previous) = loaded.as_ref() {
        ensure!(previous == &directory, "同一进程不能加载不同目录的 Maa SDK");
    } else {
        let manifest: Manifest =
            serde_json::from_str(include_str!("../../../resources/sdk-manifest.json"))?;
        for binary in manifest.binaries {
            let bytes = std::fs::read(directory.join(&binary.path))
                .with_context(|| format!("SDK 缺少 {}", binary.path))?;
            ensure!(
                bytes.len() as u64 == binary.bytes
                    && hex::encode(Sha256::digest(&bytes)) == binary.sha256,
                "SDK 校验失败：{}，请重新安装固定版本",
                binary.path
            );
        }
        let wide: Vec<u16> = directory
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // 进程内 SDK 终身加载；目录 cookie 也保留到进程退出。
        ensure!(
            unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) } != 0,
            "无法限制 DLL 搜索目录"
        );
        ensure!(
            !unsafe { AddDllDirectory(wide.as_ptr()) }.is_null(),
            "无法注册 SDK 依赖目录"
        );
        maa_framework::load_library(&directory.join("MaaFramework.dll"))
            .map_err(anyhow::Error::msg)?;
        *loaded = Some(directory.clone());
        std::fs::create_dir_all(log_directory)?;
        maa_framework::configure_logging(log_directory.to_str().context("日志目录不是 UTF-8")?)?;
        maa_framework::set_save_draw(false)?;
        maa_framework::set_save_on_error(false)?;
        maa_framework::set_debug_mode(false)?;
    }
    let version = maa_framework::maa_version();
    ensure!(
        version.trim_start_matches('v') == SDK_VERSION,
        "SDK 实际版本 {version} 与固定版本 {SDK_VERSION} 不符"
    );
    drop(loaded);
    verify_loaded_modules()?;
    Ok(SdkStatus {
        ready: true,
        version: Some(version.into()),
        expected_version: SDK_VERSION.into(),
        binding_version: BINDING_VERSION.into(),
        directory: directory.display().to_string(),
        message: "固定版本 SDK 与原生 DLL 校验通过".into(),
    })
}

pub(crate) fn verify_loaded_modules() -> Result<()> {
    let directory = LOADED_PATH
        .lock()
        .map_err(|_| anyhow::anyhow!("SDK 初始化锁失效"))?
        .clone()
        .context("SDK 尚未加载")?;
    let manifest: Manifest =
        serde_json::from_str(include_str!("../../../resources/sdk-manifest.json"))?;
    for binary in manifest.binaries {
        let name: Vec<u16> = binary.path.encode_utf16().chain(Some(0)).collect();
        let module = unsafe { GetModuleHandleW(name.as_ptr()) };
        if module.is_null() {
            continue;
        }
        let mut path = vec![0_u16; 32768];
        let length =
            unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) } as usize;
        ensure!(
            length > 0 && length < path.len(),
            "无法检查 {} 实际加载路径",
            binary.path
        );
        let actual = PathBuf::from(String::from_utf16_lossy(&path[..length])).canonicalize()?;
        let expected = directory.join(&binary.path).canonicalize()?;
        ensure!(
            actual == expected,
            "{} 实际从非受控目录加载：{}",
            binary.path,
            actual.display()
        );
    }
    Ok(())
}
