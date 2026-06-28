mod frame;
mod output;

pub use frame::VideoFrame;
pub use output::{create_video_output, VideoOutput};

#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

#[cfg(target_arch = "wasm32")]
pub use output::create_wasm_video_output;

#[cfg(target_arch = "wasm32")]
pub use wasm::WasmVideoOutput;
