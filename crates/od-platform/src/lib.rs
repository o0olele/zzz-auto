mod paths;
pub use paths::{DesktopLayout, desktop_layout};
pub mod download;
pub mod model_cache;
#[cfg(windows)]
pub mod shortcuts;

#[cfg(windows)]
mod engine_process;
#[cfg(windows)]
pub use engine_process::{EngineProcess, windows_arguments};

#[cfg(windows)]
mod input_recovery;
#[cfg(windows)]
pub use input_recovery::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
compile_error!("首版引擎仅支持 Windows x64 MSVC");
