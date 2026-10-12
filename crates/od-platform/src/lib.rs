mod paths;
pub use paths::{DesktopLayout, desktop_layout};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
compile_error!("首版引擎仅支持 Windows x64 MSVC");
