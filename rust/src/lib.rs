//! UniFFI wrapper around the pure-Rust `spice-client` crate, exposing a small
//! synchronous surface to Kotlin that mirrors `rdp-transport`'s `RdpClient`:
//! a frame callback (RGBA), a session-lifecycle callback, and input methods.
//!
//! `spice-client` is async (Tokio) and `SpiceClientShared` is the complete
//! client (main + display + inputs + cursor channels); we own a multi-thread
//! Tokio runtime, drive `connect`/`start_event_loop` on it, and `block_on`
//! each input call. Frames arrive via `set_display_update_callback`, fired
//! from the display channel's run loop on a runtime worker thread.

use std::sync::{Arc, Mutex};

use log::{error, info};
use spice_client::{
    CursorShape, DisplaySurface, MouseButton as SpiceMouseButton, SpiceClientShared,
};

uniffi::setup_scaffolding!();

const PRIMARY_CHANNEL: u8 = 0;

fn init_logging() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Debug)
                .with_tag("SpiceNative"),
        );
    });
}

#[derive(Debug, uniffi::Error)]
pub enum SpiceError {
    ConnectionFailed,
    AuthenticationFailed,
    ProtocolError,
    Disconnected,
    IoError,
}

impl std::fmt::Display for SpiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpiceError::ConnectionFailed => write!(f, "Connection failed"),
            SpiceError::AuthenticationFailed => write!(f, "Authentication failed"),
            SpiceError::ProtocolError => write!(f, "Protocol error"),
            SpiceError::Disconnected => write!(f, "Disconnected"),
            SpiceError::IoError => write!(f, "I/O error"),
        }
    }
}

/// Advisory desktop size requested by the client. SPICE servers may ignore it
/// (QEMU resizes to the guest's mode); the real size arrives with each frame.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SpiceConfig {
    pub width: u16,
    pub height: u16,
}

/// One display surface, RGBA (4 bytes/pixel), row-major, no padding.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FrameData {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[uniffi::export(with_foreign)]
pub trait FrameCallback: Send + Sync {
    fn on_frame(&self, frame: FrameData);
}

/// One hardware-cursor update: decoded RGBA shape plus position and visibility.
/// `pixels` is `width*height*4` RGBA with alpha already composited from the
/// wire ALPHA/MONO shape; empty when the cursor has no shape. Mirrors RDP's
/// `onCursorUpdate`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CursorData {
    pub width: u16,
    pub height: u16,
    pub hot_x: u16,
    pub hot_y: u16,
    pub x: i32,
    pub y: i32,
    pub visible: bool,
    pub pixels: Vec<u8>,
}

#[uniffi::export(with_foreign)]
pub trait CursorCallback: Send + Sync {
    fn on_cursor(&self, cursor: CursorData);
}

/// Lifecycle + error surface, mirroring RDP's `SessionCallback`.
#[uniffi::export(with_foreign)]
pub trait SessionCallback: Send + Sync {
    fn on_connected(&self, width: u16, height: u16);
    fn on_error(&self, message: String);
    fn on_disconnected(&self);
}

#[derive(uniffi::Object)]
pub struct SpiceClient {
    rt: tokio::runtime::Runtime,
    config: SpiceConfig,
    inner: Mutex<Option<Arc<SpiceClientShared>>>,
    frame_cb: Mutex<Option<Arc<dyn FrameCallback>>>,
    cursor_cb: Mutex<Option<Arc<dyn CursorCallback>>>,
    session_cb: Mutex<Option<Arc<dyn SessionCallback>>>,
}

#[uniffi::export]
impl SpiceClient {
    #[uniffi::constructor]
    pub fn new(config: SpiceConfig) -> Arc<Self> {
        init_logging();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime");
        Arc::new(Self {
            rt,
            config,
            inner: Mutex::new(None),
            frame_cb: Mutex::new(None),
            cursor_cb: Mutex::new(None),
            session_cb: Mutex::new(None),
        })
    }

    pub fn set_frame_callback(&self, cb: Arc<dyn FrameCallback>) {
        *self.frame_cb.lock().unwrap() = Some(cb);
    }

    pub fn set_cursor_callback(&self, cb: Arc<dyn CursorCallback>) {
        *self.cursor_cb.lock().unwrap() = Some(cb);
    }

    pub fn set_session_callback(&self, cb: Arc<dyn SessionCallback>) {
        *self.session_cb.lock().unwrap() = Some(cb);
    }

    /// Connect, wire the frame callback, and start the channel run loops.
    /// `password` is the SPICE ticket (RSA-OAEP); pass None for an
    /// unticketed (disable-ticketing) server.
    pub fn connect(
        &self,
        host: String,
        port: u16,
        password: Option<String>,
    ) -> Result<(), SpiceError> {
        let mut client = SpiceClientShared::new(host, port);
        if let Some(pw) = password {
            if !pw.is_empty() {
                self.rt.block_on(client.set_password(pw));
            }
        }
        let client = Arc::new(client);

        // connect() establishes the main channel then synchronously connects
        // the display/inputs/cursor channels into the shared inner maps.
        self.rt.block_on(client.connect()).map_err(|e| {
            error!("SPICE connect failed: {}", e);
            map_err(&e)
        })?;

        // Wire frames before starting the run loops so none are missed.
        if let Some(cb) = self.frame_cb.lock().unwrap().clone() {
            let res = self.rt.block_on(client.set_display_update_callback(
                PRIMARY_CHANNEL,
                move |surface: &DisplaySurface| {
                    cb.on_frame(surface_to_frame(surface));
                },
            ));
            if let Err(e) = res {
                // Non-fatal: a server may number its display channel
                // differently; log and continue (input/cursor may still work).
                error!("set_display_update_callback failed: {}", e);
            }
        }

        // Wire cursor updates before the run loop takes the channel lock.
        if let Some(cb) = self.cursor_cb.lock().unwrap().clone() {
            let res = self.rt.block_on(client.set_cursor_update_callback(
                PRIMARY_CHANNEL,
                move |shape: &CursorShape, pos: (i32, i32), visible: bool| {
                    cb.on_cursor(cursor_to_data(shape, pos, visible));
                },
            ));
            if let Err(e) = res {
                // Non-fatal: a headless server may omit the cursor channel.
                error!("set_cursor_update_callback failed: {}", e);
            }
        }

        self.rt.block_on(client.start_event_loop()).map_err(|e| {
            error!("SPICE start_event_loop failed: {}", e);
            map_err(&e)
        })?;

        *self.inner.lock().unwrap() = Some(client);

        if let Some(cb) = self.session_cb.lock().unwrap().clone() {
            cb.on_connected(self.config.width, self.config.height);
        }
        info!("SPICE session established");
        Ok(())
    }

    pub fn disconnect(&self) {
        let client = self.inner.lock().unwrap().take();
        if let Some(client) = client {
            self.rt.block_on(client.disconnect());
        }
        if let Some(cb) = self.session_cb.lock().unwrap().clone() {
            cb.on_disconnected();
        }
    }

    pub fn is_connected(&self) -> bool {
        self.inner.lock().unwrap().is_some()
    }

    pub fn send_key(&self, scancode: u32, pressed: bool) {
        if let Some(client) = self.client() {
            let _ = if pressed {
                self.rt.block_on(client.send_key_down(PRIMARY_CHANNEL, scancode))
            } else {
                self.rt.block_on(client.send_key_up(PRIMARY_CHANNEL, scancode))
            };
        }
    }

    pub fn send_mouse_move(&self, x: i32, y: i32) {
        if let Some(client) = self.client() {
            let _ = self
                .rt
                .block_on(client.send_mouse_motion(PRIMARY_CHANNEL, x, y));
        }
    }

    pub fn send_mouse_button(&self, button: MouseButton, pressed: bool) {
        if let Some(client) = self.client() {
            let _ = self.rt.block_on(client.send_mouse_button(
                PRIMARY_CHANNEL,
                map_button(button),
                pressed,
            ));
        }
    }

    /// Vertical wheel: positive `delta` scrolls up, negative down.
    pub fn send_mouse_wheel(&self, delta: i32) {
        if let Some(client) = self.client() {
            let _ = self
                .rt
                .block_on(client.send_mouse_wheel(PRIMARY_CHANNEL, 0, delta));
        }
    }
}

impl SpiceClient {
    fn client(&self) -> Option<Arc<SpiceClientShared>> {
        self.inner.lock().unwrap().clone()
    }
}

fn map_button(b: MouseButton) -> SpiceMouseButton {
    match b {
        MouseButton::Left => SpiceMouseButton::Left,
        MouseButton::Right => SpiceMouseButton::Right,
        MouseButton::Middle => SpiceMouseButton::Middle,
    }
}

fn map_err(e: &spice_client::SpiceError) -> SpiceError {
    use spice_client::SpiceError as E;
    match e {
        E::Io(_) => SpiceError::IoError,
        E::AuthenticationFailed => SpiceError::AuthenticationFailed,
        E::Connection(_) => SpiceError::ConnectionFailed,
        E::ConnectionClosed => SpiceError::Disconnected,
        _ => SpiceError::ProtocolError,
    }
}

/// Swap R↔B in a 4-byte-per-pixel buffer, RGBA → BGRA.
///
/// The decoder builds surfaces and cursors as RGBA (R first). Android's
/// `ARGB_8888 Bitmap.copyPixelsFromBuffer` reads each pixel as the little-endian
/// `0xAARRGGBB` int — i.e. bytes **B, G, R, A** — so it expects BGRA, the same
/// order IronRDP feeds for RDP. Handing it RGBA renders red and blue swapped
/// (#286). We swap once here at the FFI boundary so the on-device Bitmap is
/// correct; host tools (`spice-cli`) read the RGBA `DisplaySurface` directly and
/// are unaffected.
fn rgba_to_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    out
}

/// Convert a SPICE display surface into a [FrameData] for the Android Bitmap.
/// The surface `data` is RGBA; we emit BGRA — see [rgba_to_bgra].
fn surface_to_frame(surface: &DisplaySurface) -> FrameData {
    FrameData {
        width: surface.width as u16,
        height: surface.height as u16,
        pixels: rgba_to_bgra(&surface.data),
    }
}

/// Convert a decoded SPICE cursor shape + position/visibility into [CursorData].
/// `shape.data` is RGBA (alpha composited by the cursor channel); emit BGRA to
/// match the framebuffer's byte order — see [rgba_to_bgra].
fn cursor_to_data(shape: &CursorShape, pos: (i32, i32), visible: bool) -> CursorData {
    CursorData {
        width: shape.width,
        height: shape.height,
        hot_x: shape.hot_spot_x,
        hot_y: shape.hot_spot_y,
        x: pos.0,
        y: pos.1,
        visible,
        pixels: rgba_to_bgra(&shape.data),
    }
}
