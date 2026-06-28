use super::VideoFrame;
use crate::channels::display::DisplaySurface;
use std::any::Any;
use std::sync::Arc;

/// Trait for video output handling
#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
#[allow(async_fn_in_trait)]
pub trait VideoOutput: Send + Sync {
    /// Update the current frame
    async fn update_frame(&self, surface: &DisplaySurface);

    /// Get the current frame
    async fn get_current_frame(&self) -> Option<VideoFrame>;

    /// Get the total frame count
    async fn get_frame_count(&self) -> u64;

    /// Downcast to concrete type (for accessing platform-specific methods)
    fn as_any(&self) -> &dyn Any;
}

/// Trait for video output handling - WASM version
#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
#[allow(async_fn_in_trait)]
pub trait VideoOutput {
    /// Update the current frame
    async fn update_frame(&self, surface: &DisplaySurface);

    /// Get the current frame
    async fn get_current_frame(&self) -> Option<VideoFrame>;

    /// Get the total frame count
    async fn get_frame_count(&self) -> u64;

    /// Downcast to concrete type (for accessing platform-specific methods)
    fn as_any(&self) -> &dyn Any;
}

/// Create a platform-specific VideoOutput implementation
pub fn create_video_output() -> Arc<dyn VideoOutput> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Arc::new(super::native::NativeVideoOutput::new())
    }

    #[cfg(target_arch = "wasm32")]
    {
        Arc::new(super::wasm::WasmVideoOutput::new())
    }
}

/// Create a WASM video output (allows access to set_canvas method)
#[cfg(target_arch = "wasm32")]
pub fn create_wasm_video_output() -> Arc<super::wasm::WasmVideoOutput> {
    Arc::new(super::wasm::WasmVideoOutput::new())
}
