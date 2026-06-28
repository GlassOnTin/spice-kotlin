use super::{VideoFrame, VideoOutput};
use crate::channels::cursor::CursorShape;
use crate::channels::display::DisplaySurface;
use std::any::Any;
use std::sync::{Arc, Mutex};
use tracing::{debug, error};
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};

pub struct WasmVideoOutput {
    current_frame: Arc<Mutex<Option<VideoFrame>>>,
    frame_count: Arc<Mutex<u64>>,
    canvas: Arc<Mutex<Option<HtmlCanvasElement>>>,
    context: Arc<Mutex<Option<CanvasRenderingContext2d>>>,
    cursor_shape: Arc<Mutex<Option<CursorShape>>>,
    cursor_position: Arc<Mutex<(i32, i32)>>,
    cursor_visible: Arc<Mutex<bool>>,
}

impl WasmVideoOutput {
    pub fn new() -> Self {
        Self {
            current_frame: Arc::new(Mutex::new(None)),
            frame_count: Arc::new(Mutex::new(0)),
            canvas: Arc::new(Mutex::new(None)),
            context: Arc::new(Mutex::new(None)),
            cursor_shape: Arc::new(Mutex::new(None)),
            cursor_position: Arc::new(Mutex::new((0, 0))),
            cursor_visible: Arc::new(Mutex::new(false)),
        }
    }

    /// Set the canvas element to render to
    pub fn set_canvas(&self, canvas: HtmlCanvasElement) {
        // Get 2D rendering context
        let context = canvas
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|ctx| ctx.dyn_into::<CanvasRenderingContext2d>().ok());

        if let Ok(mut canvas_lock) = self.canvas.lock() {
            *canvas_lock = Some(canvas);
        }

        if let Ok(mut context_lock) = self.context.lock() {
            *context_lock = context;
        }

        debug!("Canvas and 2D context set for WASM video output");
    }

    /// Update cursor state (to be called from cursor channel callback)
    pub fn update_cursor(&self, cursor: &CursorShape, position: (i32, i32), visible: bool) {
        if let Ok(mut cursor_shape_lock) = self.cursor_shape.lock() {
            *cursor_shape_lock = Some(cursor.clone());
        }
        if let Ok(mut position_lock) = self.cursor_position.lock() {
            *position_lock = position;
        }
        if let Ok(mut visible_lock) = self.cursor_visible.lock() {
            *visible_lock = visible;
        }

        // Trigger a re-render with cursor overlay
        self.render_cursor();
    }

    /// Render cursor overlay on canvas
    fn render_cursor(&self) {
        let cursor_visible = match self.cursor_visible.lock() {
            Ok(lock) => *lock,
            Err(_) => return,
        };

        if !cursor_visible {
            return;
        }

        let cursor_shape = match self.cursor_shape.lock() {
            Ok(lock) => lock.clone(),
            Err(_) => return,
        };

        let cursor = match cursor_shape {
            Some(c) => c,
            None => return,
        };

        let position = match self.cursor_position.lock() {
            Ok(lock) => *lock,
            Err(_) => return,
        };

        let context_lock = match self.context.lock() {
            Ok(lock) => lock,
            Err(_) => return,
        };

        let context = match context_lock.as_ref() {
            Some(ctx) => ctx,
            None => return,
        };

        // Convert BGRA cursor data to RGBA
        let mut rgba_data = Vec::with_capacity(cursor.data.len());
        for chunk in cursor.data.chunks(4) {
            if chunk.len() == 4 {
                rgba_data.push(chunk[2]); // R (was B)
                rgba_data.push(chunk[1]); // G
                rgba_data.push(chunk[0]); // B (was R)
                rgba_data.push(chunk[3]); // A
            }
        }

        // Create ImageData for cursor
        let cursor_image = match ImageData::new_with_u8_clamped_array(
            wasm_bindgen::Clamped(&rgba_data),
            cursor.width as u32,
        ) {
            Ok(img) => img,
            Err(e) => {
                error!("Failed to create cursor ImageData: {:?}", e);
                return;
            }
        };

        // Draw cursor at position (adjusted for hotspot)
        let x = (position.0 - cursor.hot_spot_x as i32) as f64;
        let y = (position.1 - cursor.hot_spot_y as i32) as f64;

        if let Err(e) = context.put_image_data(&cursor_image, x, y) {
            error!("Failed to render cursor: {:?}", e);
        }
    }

    /// Render surface data to canvas
    fn render_to_canvas(&self, surface: &DisplaySurface) {
        let context_lock = match self.context.lock() {
            Ok(lock) => lock,
            Err(e) => {
                error!("Failed to lock context: {}", e);
                return;
            }
        };

        let context = match context_lock.as_ref() {
            Some(ctx) => ctx,
            None => {
                debug!("No canvas context available for rendering");
                return;
            }
        };

        // Update canvas size if needed
        if let Ok(canvas_lock) = self.canvas.lock() {
            if let Some(canvas) = canvas_lock.as_ref() {
                if canvas.width() != surface.width || canvas.height() != surface.height {
                    canvas.set_width(surface.width);
                    canvas.set_height(surface.height);
                    debug!("Resized canvas to {}x{}", surface.width, surface.height);
                }
            }
        }

        // Create ImageData from surface data
        // SPICE uses BGRA format, web canvas expects RGBA
        let mut rgba_data = Vec::with_capacity(surface.data.len());

        for chunk in surface.data.chunks(4) {
            if chunk.len() == 4 {
                // Convert BGRA to RGBA
                rgba_data.push(chunk[2]); // R (was B)
                rgba_data.push(chunk[1]); // G
                rgba_data.push(chunk[0]); // B (was R)
                rgba_data.push(chunk[3]); // A
            }
        }

        // Create ImageData and put it on canvas
        match ImageData::new_with_u8_clamped_array(wasm_bindgen::Clamped(&rgba_data), surface.width)
        {
            Ok(image_data) => {
                if let Err(e) = context.put_image_data(&image_data, 0.0, 0.0) {
                    error!("Failed to put image data on canvas: {:?}", e);
                } else {
                    // Render cursor overlay after display surface
                    self.render_cursor();
                }
            }
            Err(e) => {
                error!("Failed to create ImageData: {:?}", e);
            }
        }
    }
}

#[async_trait::async_trait(?Send)]
impl VideoOutput for WasmVideoOutput {
    async fn update_frame(&self, surface: &DisplaySurface) {
        // Render to canvas
        self.render_to_canvas(surface);

        // Also store as VideoFrame for API compatibility
        let frame = VideoFrame::from_surface(surface);
        if let Ok(mut current) = self.current_frame.lock() {
            *current = Some(frame);
        }
        if let Ok(mut count) = self.frame_count.lock() {
            *count += 1;
        }

        debug!(
            "Frame {} rendered: {}x{}",
            self.frame_count.lock().ok().map(|c| *c).unwrap_or(0),
            surface.width,
            surface.height
        );
    }

    async fn get_current_frame(&self) -> Option<VideoFrame> {
        self.current_frame.lock().ok()?.clone()
    }

    async fn get_frame_count(&self) -> u64 {
        self.frame_count.lock().ok().map(|c| *c).unwrap_or(0)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
