//! Cursor channel implementation for hardware cursor support

use crate::channels::{Channel, ChannelConnection};
use crate::error::{Result, SpiceError};
use crate::protocol::*;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Decode a SPICE cursor shape into opaque-or-alpha RGBA (`width*height*4`).
/// Returns None for unsupported types or truncated input. Free fn so it's
/// unit-testable without a live channel.
///
/// - ALPHA: 32bpp premultiplied BGRA on the wire (Cairo ARGB32, LE) → RGBA.
/// - MONO: two stacked 1bpp bitmaps, AND mask then XOR mask. Per Windows
///   semantics: AND=1,XOR=0 transparent; AND=0,XOR=0 black; AND=0,XOR=1 white;
///   AND=1,XOR=1 (screen invert) rendered black (best effort without the
///   framebuffer underneath).
/// - COLOR4/8/16/24/32 are not implemented.
fn decode_cursor_shape(type_: u8, width: u16, height: u16, raw: &[u8]) -> Option<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 {
        return None;
    }
    match type_ {
        SPICE_CURSOR_TYPE_ALPHA => {
            let need = w * h * 4;
            if raw.len() < need {
                return None;
            }
            let mut out = Vec::with_capacity(need);
            for px in raw[..need].chunks_exact(4) {
                out.push(px[2]); // R
                out.push(px[1]); // G
                out.push(px[0]); // B
                out.push(px[3]); // A
            }
            Some(out)
        }
        SPICE_CURSOR_TYPE_MONO => {
            let stride = w.div_ceil(8);
            let need = stride * h * 2; // AND mask then XOR mask
            if raw.len() < need {
                return None;
            }
            let and = &raw[..stride * h];
            let xor = &raw[stride * h..need];
            let mut out = vec![0u8; w * h * 4];
            for y in 0..h {
                for x in 0..w {
                    let byte = y * stride + x / 8;
                    let bit = 7 - (x % 8);
                    let a = (and[byte] >> bit) & 1;
                    let xo = (xor[byte] >> bit) & 1;
                    let (r, g, b, al) = match (a, xo) {
                        (1, 0) => (0, 0, 0, 0),         // transparent
                        (0, 0) => (0, 0, 0, 255),       // black
                        (0, 1) => (255, 255, 255, 255), // white
                        _ => (0, 0, 0, 255),            // invert → black (best effort)
                    };
                    let o = (y * w + x) * 4;
                    out[o] = r;
                    out[o + 1] = g;
                    out[o + 2] = b;
                    out[o + 3] = al;
                }
            }
            Some(out)
        }
        _ => None,
    }
}

/// Fixed-layout fields of a CURSOR_SET message body (packed wire). The shape
/// header (offset 7..24) is only meaningful when `flags` lacks NONE and the
/// buffer is at least 24 bytes; the pixel data then starts at offset 24.
struct CursorSetFields {
    pos: (i32, i32),
    visible: bool,
    flags: u16,
    unique: u64,
    type_: u8,
    width: u16,
    height: u16,
    hot_spot_x: u16,
    hot_spot_y: u16,
}

/// Parse the CURSOR_SET prefix (position + visible + flags) and, when present,
/// the 17-byte shape header. Returns None only if the 7-byte prefix is missing;
/// header fields stay zero unless the full 24-byte header is present. Free fn so
/// the wire offsets are unit-testable without a live channel.
fn parse_cursor_set(data: &[u8]) -> Option<CursorSetFields> {
    if data.len() < 7 {
        return None;
    }
    let x = i16::from_le_bytes([data[0], data[1]]) as i32;
    let y = i16::from_le_bytes([data[2], data[3]]) as i32;
    let mut f = CursorSetFields {
        pos: (x, y),
        visible: data[4] != 0,
        flags: u16::from_le_bytes([data[5], data[6]]),
        unique: 0,
        type_: 0,
        width: 0,
        height: 0,
        hot_spot_x: 0,
        hot_spot_y: 0,
    };
    if data.len() >= 24 {
        f.unique = u64::from_le_bytes([
            data[7], data[8], data[9], data[10], data[11], data[12], data[13], data[14],
        ]);
        f.type_ = data[15];
        f.width = u16::from_le_bytes([data[16], data[17]]);
        f.height = u16::from_le_bytes([data[18], data[19]]);
        f.hot_spot_x = u16::from_le_bytes([data[20], data[21]]);
        f.hot_spot_y = u16::from_le_bytes([data[22], data[23]]);
    }
    Some(f)
}

/// Cursor shape data
#[derive(Debug, Clone)]
pub struct CursorShape {
    pub width: u16,
    pub height: u16,
    pub hot_spot_x: u16,
    pub hot_spot_y: u16,
    pub data: Vec<u8>,
    pub mask: Option<Vec<u8>>,
}

/// Cursor update callback type
#[cfg(target_arch = "wasm32")]
pub type CursorUpdateCallback = Arc<dyn Fn(&CursorShape, (i32, i32), bool)>;

/// Cursor update callback type
#[cfg(not(target_arch = "wasm32"))]
pub type CursorUpdateCallback = Arc<dyn Fn(&CursorShape, (i32, i32), bool) + Send + Sync>;

/// Cursor channel for handling mouse cursor updates
pub struct CursorChannel {
    pub(crate) connection: ChannelConnection,
    current_cursor: Option<CursorShape>,
    cursor_cache: HashMap<u64, CursorShape>,
    cursor_visible: bool,
    cursor_position: (i32, i32),
    update_callback: Option<CursorUpdateCallback>,
}

impl CursorChannel {
    pub async fn new(host: &str, port: u16, channel_id: u8) -> Result<Self> {
        Self::new_with_connection_id(host, port, channel_id, None).await
    }

    pub async fn new_with_connection_id(
        host: &str,
        port: u16,
        channel_id: u8,
        connection_id: Option<u32>,
    ) -> Result<Self> {
        let mut connection =
            ChannelConnection::new(host, port, ChannelType::Cursor, channel_id).await?;
        if let Some(conn_id) = connection_id {
            connection.set_connection_id(conn_id);
        }
        connection.handshake().await?;

        Ok(Self {
            connection,
            current_cursor: None,
            cursor_cache: HashMap::new(),
            cursor_visible: true,
            cursor_position: (0, 0),
            update_callback: None,
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn new_websocket(websocket_url: &str, channel_id: u8) -> Result<Self> {
        Self::new_websocket_with_auth(websocket_url, channel_id, None).await
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn new_websocket_with_auth(
        websocket_url: &str,
        channel_id: u8,
        auth_token: Option<String>,
    ) -> Result<Self> {
        Self::new_websocket_with_auth_and_session(websocket_url, channel_id, auth_token, None, None)
            .await
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn new_websocket_with_auth_and_session(
        websocket_url: &str,
        channel_id: u8,
        auth_token: Option<String>,
        password: Option<String>,
        connection_id: Option<u32>,
    ) -> Result<Self> {
        let mut connection = ChannelConnection::new_websocket_with_auth(
            websocket_url,
            ChannelType::Cursor,
            channel_id,
            auth_token,
        )
        .await?;
        if let Some(pwd) = password {
            connection.set_password(pwd);
        }
        if let Some(conn_id) = connection_id {
            connection.set_connection_id(conn_id);
        }
        connection.handshake().await?;

        Ok(Self {
            connection,
            current_cursor: None,
            cursor_cache: HashMap::new(),
            cursor_visible: true,
            cursor_position: (0, 0),
            update_callback: None,
        })
    }

    pub async fn initialize(&mut self) -> Result<()> {
        info!("Cursor channel {} initialized", self.connection.channel_id);
        Ok(())
    }

    pub fn get_current_cursor(&self) -> Option<&CursorShape> {
        self.current_cursor.as_ref()
    }

    pub fn is_cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub fn get_cursor_position(&self) -> (i32, i32) {
        self.cursor_position
    }

    /// Set the cursor update callback
    pub fn set_update_callback(&mut self, callback: CursorUpdateCallback) {
        self.update_callback = Some(callback);
    }

    /// Notify cursor update callback if set
    fn notify_cursor_update(&self) {
        if let Some(callback) = &self.update_callback {
            if let Some(cursor) = &self.current_cursor {
                callback(cursor, self.cursor_position, self.cursor_visible);
            }
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        loop {
            let (header, data) = self.connection.read_message().await?;
            self.handle_message(&header, &data).await?;
        }
    }

    async fn handle_cursor_init(&mut self, data: &[u8]) -> Result<()> {
        if data.len() >= 8 {
            let visible = u16::from_le_bytes([data[0], data[1]]) != 0;
            let x = i16::from_le_bytes([data[2], data[3]]) as i32;
            let y = i16::from_le_bytes([data[4], data[5]]) as i32;
            let trail_len = u16::from_le_bytes([data[6], data[7]]);

            self.cursor_visible = visible;
            self.cursor_position = (x, y);

            info!(
                "Cursor init - visible: {}, position: ({}, {}), trail: {}",
                visible, x, y, trail_len
            );

            self.notify_cursor_update();
        }
        Ok(())
    }

    async fn handle_cursor_set(&mut self, data: &[u8]) -> Result<()> {
        // SpiceMsgCursorSet (packed wire):
        //   position Point16 {x i16, y i16} | visible u8 | cursor {
        //     flags u16 | [ header(17) | data[] ] }
        // The header is only present when flags != NONE; pixel data follows it
        // unless FROM_CACHE. Header(17) = unique u64, type u8, width u16,
        // height u16, hot_spot_x u16, hot_spot_y u16. So pixels start at byte 24.
        let Some(f) = parse_cursor_set(data) else {
            warn!("CURSOR_SET too short: {} bytes", data.len());
            return Ok(());
        };
        self.cursor_position = f.pos;
        self.cursor_visible = f.visible;

        if f.flags & SPICE_CURSOR_FLAGS_NONE != 0 {
            // No shape — empty/hidden cursor.
            self.current_cursor = None;
            self.notify_cursor_update();
            return Ok(());
        }
        if data.len() < 24 {
            warn!("CURSOR_SET truncated header: {} bytes", data.len());
            return Ok(());
        }

        let shape = if f.flags & SPICE_CURSOR_FLAGS_FROM_CACHE != 0 {
            match self.cursor_cache.get(&f.unique) {
                Some(s) => s.clone(),
                None => {
                    warn!("CURSOR_SET FROM_CACHE miss for unique {}", f.unique);
                    return Ok(());
                }
            }
        } else {
            match decode_cursor_shape(f.type_, f.width, f.height, &data[24..]) {
                Some(rgba) => {
                    let s = CursorShape {
                        width: f.width,
                        height: f.height,
                        hot_spot_x: f.hot_spot_x,
                        hot_spot_y: f.hot_spot_y,
                        data: rgba,
                        mask: None,
                    };
                    if f.flags & SPICE_CURSOR_FLAGS_CACHE_ME != 0 {
                        self.cursor_cache.insert(f.unique, s.clone());
                    }
                    s
                }
                None => {
                    warn!(
                        "CURSOR_SET: type {} ({}x{}) not decoded, skipping",
                        f.type_, f.width, f.height
                    );
                    return Ok(());
                }
            }
        };

        info!(
            "Set cursor - {}x{} type {}, hotspot ({}, {})",
            shape.width, shape.height, f.type_, shape.hot_spot_x, shape.hot_spot_y
        );
        self.current_cursor = Some(shape);
        self.notify_cursor_update();
        Ok(())
    }

    async fn handle_cursor_move(&mut self, data: &[u8]) -> Result<()> {
        if data.len() >= 4 {
            let x = i16::from_le_bytes([data[0], data[1]]) as i32;
            let y = i16::from_le_bytes([data[2], data[3]]) as i32;

            self.cursor_position = (x, y);
            debug!("Cursor moved to ({}, {})", x, y);

            self.notify_cursor_update();
        }
        Ok(())
    }

    async fn handle_cursor_hide(&mut self) -> Result<()> {
        self.cursor_visible = false;
        debug!("Cursor hidden");

        self.notify_cursor_update();
        Ok(())
    }

    async fn handle_cursor_trail(&mut self, data: &[u8]) -> Result<()> {
        if data.len() >= 4 {
            let length = u16::from_le_bytes([data[0], data[1]]);
            let frequency = u16::from_le_bytes([data[2], data[3]]);

            debug!(
                "Cursor trail - length: {}, frequency: {}",
                length, frequency
            );
        }
        Ok(())
    }

    async fn handle_cursor_inval_one(&mut self, data: &[u8]) -> Result<()> {
        if data.len() >= 8 {
            let cache_id = u64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);

            self.cursor_cache.remove(&cache_id);
            debug!("Invalidated cursor cache entry: {}", cache_id);
        }
        Ok(())
    }

    async fn handle_cursor_inval_all(&mut self) -> Result<()> {
        self.cursor_cache.clear();
        debug!("Invalidated all cursor cache entries");
        Ok(())
    }
}

impl Channel for CursorChannel {
    async fn handle_message(&mut self, header: &SpiceDataHeader, data: &[u8]) -> Result<()> {
        match header.msg_type {
            SPICE_MSG_SET_ACK => {
                debug!("Received SET_ACK message");
                // Parse the generation number
                if data.len() >= 4 {
                    let generation = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                    debug!("Cursor channel: SET_ACK generation: {}", generation);

                    // Send ACK_SYNC response
                    let ack_data = generation.to_le_bytes();
                    self.connection
                        .send_message(SPICE_MSGC_ACK_SYNC, &ack_data)
                        .await?;
                    debug!("Cursor channel: Sent ACK_SYNC response");
                }
            }
            SPICE_MSG_CURSOR_INIT => {
                debug!("Received cursor init");
                self.handle_cursor_init(data).await?;
            }
            SPICE_MSG_CURSOR_SET => {
                debug!("Received cursor set");
                self.handle_cursor_set(data).await?;
            }
            SPICE_MSG_CURSOR_MOVE => {
                debug!("Received cursor move");
                self.handle_cursor_move(data).await?;
            }
            SPICE_MSG_CURSOR_HIDE => {
                debug!("Received cursor hide");
                self.handle_cursor_hide().await?;
            }
            SPICE_MSG_CURSOR_TRAIL => {
                debug!("Received cursor trail");
                self.handle_cursor_trail(data).await?;
            }
            SPICE_MSG_CURSOR_INVAL_ONE => {
                debug!("Received cursor inval one");
                self.handle_cursor_inval_one(data).await?;
            }
            SPICE_MSG_CURSOR_INVAL_ALL => {
                debug!("Received cursor inval all");
                self.handle_cursor_inval_all().await?;
            }
            SPICE_MSG_CURSOR_RESET => {
                debug!("Received cursor reset");
                // Reset cursor to default state
                self.current_cursor = None;
                self.cursor_position = (0, 0);
                self.cursor_visible = false;
                self.cursor_cache.clear();
            }
            x if x == SPICE_MSG_PING => {
                debug!("Received PING message in cursor channel");
                if data.len() >= 4 {
                    let id = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                    let time = if data.len() >= 12 {
                        u64::from_le_bytes([
                            data[4], data[5], data[6], data[7], data[8], data[9], data[10],
                            data[11],
                        ])
                    } else {
                        0
                    };
                    // Send PONG response
                    let mut pong_data = Vec::with_capacity(12);
                    pong_data.extend_from_slice(&id.to_le_bytes());
                    pong_data.extend_from_slice(&time.to_le_bytes());
                    self.connection
                        .send_message(SPICE_MSGC_PONG, &pong_data)
                        .await?;
                }
            }
            x if x == SPICE_MSG_NOTIFY => {
                if data.len() >= 12 {
                    let severity = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
                    let msg_len =
                        u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
                    if data.len() >= 12 + msg_len {
                        let message = String::from_utf8_lossy(&data[12..12 + msg_len]);
                        match severity {
                            0 => info!("Cursor server info: {}", message),
                            1 => warn!("Cursor server warning: {}", message),
                            2 => error!("Cursor server error: {}", message),
                            _ => debug!("Cursor server notification: {}", message),
                        }
                    }
                }
            }
            x if x == SPICE_MSG_DISCONNECTING => {
                info!("Cursor channel: Server is disconnecting");
                return Err(SpiceError::ConnectionClosed);
            }
            x if x == SPICE_MSG_MIGRATE => {
                warn!("Cursor channel: Migration not implemented");
            }
            x if x == SPICE_MSG_MIGRATE_DATA => {
                warn!("Cursor channel: Migration data not implemented");
            }
            x if x == SPICE_MSG_WAIT_FOR_CHANNELS => {
                warn!("Cursor channel: Wait for channels not implemented");
            }
            _ => {
                warn!("Unknown cursor message type: {}", header.msg_type);
            }
        }

        Ok(())
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Cursor
    }
}

#[derive(Debug, Clone)]
pub struct SpiceCursorHeader {
    pub unique: u64,
    pub type_: u8,
    pub width: u16,
    pub height: u16,
    pub hot_spot_x: u16,
    pub hot_spot_y: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cursor_shape_creation() {
        let cursor = CursorShape {
            width: 32,
            height: 32,
            hot_spot_x: 16,
            hot_spot_y: 16,
            data: vec![0xFF; 32 * 32 * 4],
            mask: None,
        };

        assert_eq!(cursor.width, 32);
        assert_eq!(cursor.height, 32);
        assert_eq!(cursor.hot_spot_x, 16);
        assert_eq!(cursor.hot_spot_y, 16);
        assert_eq!(cursor.data.len(), 32 * 32 * 4);
    }

    #[tokio::test]
    async fn test_cursor_position() {
        // Test cursor position parsing
        let move_data = vec![
            0x10, 0x00, // x = 16
            0x20, 0x00, // y = 32
        ];

        let x = i16::from_le_bytes([move_data[0], move_data[1]]) as i32;
        let y = i16::from_le_bytes([move_data[2], move_data[3]]) as i32;

        assert_eq!(x, 16);
        assert_eq!(y, 32);
    }

    #[test]
    fn decode_alpha_cursor_bgra_to_rgba() {
        // 1x1 ALPHA cursor; wire is premultiplied BGRA -> expect RGBA.
        let raw = [0x10u8, 0x20, 0x30, 0x40]; // B=0x10 G=0x20 R=0x30 A=0x40
        let out = decode_cursor_shape(SPICE_CURSOR_TYPE_ALPHA, 1, 1, &raw).unwrap();
        assert_eq!(out, vec![0x30, 0x20, 0x10, 0x40]);
    }

    #[test]
    fn decode_alpha_cursor_truncated_is_none() {
        // 2x2 needs 16 bytes; give 8 -> None, not a panic.
        assert!(decode_cursor_shape(SPICE_CURSOR_TYPE_ALPHA, 2, 2, &[0u8; 8]).is_none());
    }

    #[test]
    fn decode_mono_cursor_and_xor_semantics() {
        // 8x1 MONO. stride=1, AND byte then XOR byte.
        // AND=0b1100_0000, XOR=0b1010_0000 -> per pixel (x=0..7):
        //  x0: AND1 XOR1 -> invert(black opaque)
        //  x1: AND1 XOR0 -> transparent
        //  x2: AND0 XOR1 -> white
        //  x3..7: AND0 XOR0 -> black
        let raw = [0b1100_0000u8, 0b1010_0000u8];
        let out = decode_cursor_shape(SPICE_CURSOR_TYPE_MONO, 8, 1, &raw).unwrap();
        let px = |x: usize| &out[x * 4..x * 4 + 4];
        assert_eq!(px(0), &[0, 0, 0, 255]); // invert -> black opaque
        assert_eq!(px(1), &[0, 0, 0, 0]); // transparent
        assert_eq!(px(2), &[255, 255, 255, 255]); // white
        assert_eq!(px(3), &[0, 0, 0, 255]); // black
    }

    #[test]
    fn decode_color_cursor_unsupported_is_none() {
        assert!(decode_cursor_shape(SPICE_CURSOR_TYPE_COLOR32, 4, 4, &[0u8; 64]).is_none());
    }

    #[test]
    fn parse_cursor_set_alpha_offsets_then_decode() {
        // Full CURSOR_SET ALPHA body for a 2x1 cursor; checks every wire offset
        // (pos@0, visible@4, flags@5, unique@7, type@15, w@16, h@18, hot@20,
        // pixels@24) and that the tail decodes BGRA->RGBA.
        let mut d = Vec::new();
        d.extend_from_slice(&7i16.to_le_bytes()); // x
        d.extend_from_slice(&9i16.to_le_bytes()); // y
        d.push(1); // visible
        d.extend_from_slice(&SPICE_CURSOR_FLAGS_CACHE_ME.to_le_bytes());
        d.extend_from_slice(&0xABCDu64.to_le_bytes()); // unique
        d.push(SPICE_CURSOR_TYPE_ALPHA); // type
        d.extend_from_slice(&2u16.to_le_bytes()); // width
        d.extend_from_slice(&1u16.to_le_bytes()); // height
        d.extend_from_slice(&1u16.to_le_bytes()); // hot_x
        d.extend_from_slice(&0u16.to_le_bytes()); // hot_y
        d.extend_from_slice(&[0x10, 0x20, 0x30, 0x40, 0x11, 0x22, 0x33, 0x44]); // 2px BGRA
        assert_eq!(d.len(), 32);

        let f = parse_cursor_set(&d).unwrap();
        assert_eq!(f.pos, (7, 9));
        assert!(f.visible);
        assert_eq!(f.flags, SPICE_CURSOR_FLAGS_CACHE_ME);
        assert_eq!(f.unique, 0xABCD);
        assert_eq!(f.type_, SPICE_CURSOR_TYPE_ALPHA);
        assert_eq!((f.width, f.height), (2, 1));
        assert_eq!((f.hot_spot_x, f.hot_spot_y), (1, 0));

        let rgba = decode_cursor_shape(f.type_, f.width, f.height, &d[24..]).unwrap();
        assert_eq!(rgba, vec![0x30, 0x20, 0x10, 0x40, 0x33, 0x22, 0x11, 0x44]);
    }

    #[test]
    fn parse_cursor_set_none_flag_prefix_only() {
        let mut d = Vec::new();
        d.extend_from_slice(&0i16.to_le_bytes());
        d.extend_from_slice(&0i16.to_le_bytes());
        d.push(0); // not visible
        d.extend_from_slice(&SPICE_CURSOR_FLAGS_NONE.to_le_bytes());
        let f = parse_cursor_set(&d).unwrap();
        assert_eq!(f.flags, SPICE_CURSOR_FLAGS_NONE);
        assert!(!f.visible);
        assert_eq!(f.width, 0); // header absent -> zeroed
    }

    #[test]
    fn parse_cursor_set_too_short_is_none() {
        assert!(parse_cursor_set(&[0, 0, 0]).is_none());
    }
}
