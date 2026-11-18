#[cfg(target_os = "linux")]
#[path = "platform/linux.rs"]
mod implementation;
#[cfg(target_os = "windows")]
#[path = "platform/windows.rs"]
mod implementation;
#[cfg(not(any(target_os = "windows", target_os = "linux", target_arch = "wasm32")))]
#[path = "platform/unsupported.rs"]
mod implementation;
#[cfg(target_arch = "wasm32")]
#[path = "platform/wasm.rs"]
pub(crate) mod wasm;
#[cfg(target_arch = "wasm32")]
use wasm as implementation;

pub(crate) use implementation::start_audio_capture;
