use crate::channels::{Channel, ChannelConnection};
use crate::error::{Result, SpiceError};
use crate::protocol::*;
use binrw::BinRead;
use std::collections::HashMap;
use tracing::{debug, error, info, warn};

// Integration tests moved to tests/display_integration.rs

#[path = "video_tests.rs"]
mod video_tests;

// Cache for images referenced by FROM_CACHE types
struct ImageCache {
    entries: HashMap<u64, CachedImage>,
}

struct CachedImage {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

impl ImageCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn get(&self, id: u64) -> Option<&CachedImage> {
        self.entries.get(&id)
    }

    fn insert(&mut self, id: u64, data: Vec<u8>, width: u32, height: u32) {
        self.entries.insert(
            id,
            CachedImage {
                data,
                width,
                height,
            },
        );
    }

    fn remove(&mut self, id: u64) -> bool {
        self.entries.remove(&id).is_some()
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Debug, Clone)]
pub struct DisplaySurface {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub id: u32,
    pub codec_type: u8,
    pub width: u32,
    pub height: u32,
    pub dest_rect: SpiceRect,
}

pub struct DisplayChannel {
    pub(crate) connection: ChannelConnection,
    surfaces: HashMap<u32, DisplaySurface>,
    monitors: Vec<SpiceHead>,
    active_streams: HashMap<u32, StreamInfo>,
    #[cfg(not(target_arch = "wasm32"))]
    update_callback: Option<Box<dyn Fn(&DisplaySurface) + Send + Sync>>,
    #[cfg(target_arch = "wasm32")]
    update_callback: Option<Box<dyn Fn(&DisplaySurface)>>,
    image_cache: ImageCache,
    last_mark: Option<u32>, // Track last received mark for synchronization
}

impl DisplayChannel {
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
            ChannelConnection::new(host, port, ChannelType::Display, channel_id).await?;
        if let Some(conn_id) = connection_id {
            connection.set_connection_id(conn_id);
        }
        connection.handshake().await?;

        // Send display init message after handshake
        info!("Sending SPICE_MSGC_DISPLAY_INIT");

        // Create the display init message
        // For now, disable cache until we properly handle cache negotiation
        // The server is sending cached references without first sending the images
        let display_init = SpiceMsgcDisplayInit {
            cache_id: 0,
            cache_size: 0, // Disabled - server sending cached refs without images
            glz_dict_id: 0,
        };

        // Serialize the message
        use binrw::BinWrite;
        let mut cursor = std::io::Cursor::new(Vec::new());
        display_init
            .write(&mut cursor)
            .map_err(|e| SpiceError::Protocol(format!("Failed to write display init: {e}")))?;
        let init_data = cursor.into_inner();

        connection
            .send_message(SPICE_MSGC_DISPLAY_INIT, &init_data)
            .await?;

        // Create default primary surface (ID 0) with a reasonable default size
        // Many SPICE servers assume surface 0 exists after display init
        let mut surfaces = HashMap::new();
        let default_width = 1024;
        let default_height = 768;
        let default_format = 32; // 32-bit RGBA

        info!(
            "Creating default primary surface (ID 0) - {}x{}",
            default_width, default_height
        );
        surfaces.insert(
            0,
            DisplaySurface {
                width: default_width,
                height: default_height,
                format: default_format,
                data: vec![0; (default_width * default_height * 4) as usize],
            },
        );

        Ok(Self {
            connection,
            surfaces,
            monitors: Vec::new(),
            active_streams: HashMap::new(),
            update_callback: None,
            image_cache: ImageCache::new(),
            last_mark: None,
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
        let mut connection = ChannelConnection::new_websocket_with_auth(
            websocket_url,
            ChannelType::Display,
            channel_id,
            auth_token,
        )
        .await?;
        connection.handshake().await?;

        // Send display init message after handshake
        info!("Sending SPICE_MSGC_DISPLAY_INIT");

        // Create the display init message
        // For now, disable cache until we properly handle cache negotiation
        // The server is sending cached references without first sending the images
        let display_init = SpiceMsgcDisplayInit {
            cache_id: 0,
            cache_size: 0, // Disabled - server sending cached refs without images
            glz_dict_id: 0,
        };

        // Serialize the message
        use binrw::BinWrite;
        let mut cursor = std::io::Cursor::new(Vec::new());
        display_init
            .write(&mut cursor)
            .map_err(|e| SpiceError::Protocol(format!("Failed to write display init: {e}")))?;
        let init_data = cursor.into_inner();

        connection
            .send_message(SPICE_MSGC_DISPLAY_INIT, &init_data)
            .await?;

        // Create default primary surface (ID 0) with a reasonable default size
        // Many SPICE servers assume surface 0 exists after display init
        let mut surfaces = HashMap::new();
        let default_width = 1024;
        let default_height = 768;
        let default_format = 32; // 32-bit RGBA

        info!(
            "Creating default primary surface (ID 0) - {}x{}",
            default_width, default_height
        );
        surfaces.insert(
            0,
            DisplaySurface {
                width: default_width,
                height: default_height,
                format: default_format,
                data: vec![0; (default_width * default_height * 4) as usize],
            },
        );

        Ok(Self {
            connection,
            surfaces,
            monitors: Vec::new(),
            active_streams: HashMap::new(),
            update_callback: None,
            image_cache: ImageCache::new(),
            last_mark: None,
        })
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
            ChannelType::Display,
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

        // Send display init message after handshake
        info!("Sending SPICE_MSGC_DISPLAY_INIT");

        // Create the display init message
        // For now, disable cache until we properly handle cache negotiation
        // The server is sending cached references without first sending the images
        let display_init = SpiceMsgcDisplayInit {
            cache_id: 0,
            cache_size: 0, // Disabled - server sending cached refs without images
            glz_dict_id: 0,
        };

        // Serialize the message
        use binrw::BinWrite;
        let mut cursor = std::io::Cursor::new(Vec::new());
        display_init
            .write(&mut cursor)
            .map_err(|e| SpiceError::Protocol(format!("Failed to write display init: {e}")))?;
        let init_data = cursor.into_inner();

        connection
            .send_message(SPICE_MSGC_DISPLAY_INIT, &init_data)
            .await?;

        // Create default primary surface (ID 0) with a reasonable default size
        // Many SPICE servers assume surface 0 exists after display init
        let mut surfaces = HashMap::new();
        let default_width = 1024;
        let default_height = 768;
        let default_format = 32; // 32-bit RGBA

        info!(
            "Creating default primary surface (ID 0) - {}x{}",
            default_width, default_height
        );
        surfaces.insert(
            0,
            DisplaySurface {
                width: default_width,
                height: default_height,
                format: default_format,
                data: vec![0; (default_width * default_height * 4) as usize],
            },
        );

        Ok(Self {
            connection,
            surfaces,
            monitors: Vec::new(),
            active_streams: HashMap::new(),
            update_callback: None,
            image_cache: ImageCache::new(),
            last_mark: None,
        })
    }

    pub async fn initialize(&mut self) -> Result<()> {
        info!("Display channel {} initialized", self.connection.channel_id);

        // According to SPICE protocol, display channels might need to send an init message
        // or wait for the server to send display configuration
        // For now, we just wait for server messages

        Ok(())
    }

    pub fn get_surface(&self, surface_id: u32) -> Option<&DisplaySurface> {
        self.surfaces.get(&surface_id)
    }

    pub fn get_primary_surface(&self) -> Option<&DisplaySurface> {
        let surface = self.surfaces.get(&0);
        if surface.is_none() {
            eprintln!(
                "DisplayChannel: No primary surface available. Total surfaces: {}",
                self.surfaces.len()
            );
        }
        surface
    }

    pub fn get_monitors(&self) -> &[SpiceHead] {
        &self.monitors
    }

    pub fn get_surfaces(&self) -> &HashMap<u32, DisplaySurface> {
        &self.surfaces
    }

    pub fn get_last_mark(&self) -> Option<u32> {
        self.last_mark
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_update_callback<F>(&mut self, callback: F)
    where
        F: Fn(&DisplaySurface) + Send + Sync + 'static,
    {
        self.update_callback = Some(Box::new(callback));
    }

    #[cfg(target_arch = "wasm32")]
    pub fn set_update_callback<F>(&mut self, callback: F)
    where
        F: Fn(&DisplaySurface) + 'static,
    {
        self.update_callback = Some(Box::new(callback));
    }

    /// Process a single message from the server
    /// This is primarily for testing purposes
    pub async fn process_next_message(&mut self) -> Result<()> {
        let (header, data) = self.connection.read_message().await?;
        self.handle_message(&header, &data).await
    }

    fn notify_update(&self, surface_id: u32) {
        if let Some(ref callback) = self.update_callback {
            if let Some(surface) = self.surfaces.get(&surface_id) {
                callback(surface);
            }
        }
    }

    /// Resolve a SpiceAddress to get data from the message buffer
    /// SpiceAddress is an offset from the beginning of the message data
    fn resolve_address<'a>(&self, address: SpiceAddress, data: &'a [u8]) -> Option<&'a [u8]> {
        if address == 0 {
            return None;
        }

        // Check if this is an encoded address (upper 32 bits non-zero)
        if address > 0xFFFFFFFF {
            let surface_id = (address >> 32) as u32;
            let offset = (address & 0xFFFFFFFF) as u32;

            warn!(
                "SpiceAddress 0x{:x} is encoded: surface_id=0x{:x} ({}) offset=0x{:x} ({})",
                address, surface_id, surface_id, offset, offset
            );

            // For now, we don't have a surface cache implemented
            // In a full implementation, we would look up the surface/cache entry
            // and return the data from that surface at the given offset
            warn!("Encoded SpiceAddress not supported yet - no surface cache system");
            return None;
        }

        // Simple offset from message body
        let offset = address as usize;
        if offset >= data.len() {
            warn!(
                "SpiceAddress 0x{:x} ({}) is out of bounds (data len: {})",
                address,
                address,
                data.len()
            );
            return None;
        }

        Some(&data[offset..])
    }

    /// HAVEN: decode a SPICE image embedded at `offset` within the message body.
    /// SPICE wire layout (packed, little-endian), verified against QEMU:
    ///   SpiceImageDescriptor = id u64, type u8, flags u8, width u32, height u32
    /// (18 bytes) followed by type-specific data. Replaces the upstream
    /// `decode_image`, which mis-parsed the descriptor (spurious padding) and
    /// fabricated placeholder pixels on a bogus "cached address" heuristic.
    fn decode_image_at(
        &mut self,
        data: &[u8],
        offset: usize,
    ) -> Result<Option<(Vec<u8>, u32, u32)>> {
        if offset + 18 > data.len() {
            warn!(
                "Image descriptor offset {} out of bounds (len {})",
                offset,
                data.len()
            );
            return Ok(None);
        }
        let img_type = data[offset + 8];
        let width = u32::from_le_bytes([
            data[offset + 10],
            data[offset + 11],
            data[offset + 12],
            data[offset + 13],
        ]);
        let height = u32::from_le_bytes([
            data[offset + 14],
            data[offset + 15],
            data[offset + 16],
            data[offset + 17],
        ]);
        match img_type {
            SPICE_IMAGE_TYPE_BITMAP => self.decode_bitmap_inline(data, offset + 18, width, height),
            SPICE_IMAGE_TYPE_LZ_RGB => {
                // BinaryData wrapper: data_size u32 LE @desc+18, LZ stream @desc+22.
                let ds_off = offset + 18;
                if ds_off + 4 > data.len() {
                    warn!("LZ_RGB data_size out of bounds");
                    return Ok(None);
                }
                let data_size = u32::from_le_bytes([
                    data[ds_off],
                    data[ds_off + 1],
                    data[ds_off + 2],
                    data[ds_off + 3],
                ]) as usize;
                let s = offset + 22;
                if s > data.len() {
                    return Ok(None);
                }
                let e = (s + data_size).min(data.len());
                self.decode_lz(&data[s..e])
            }
            other => {
                warn!(
                    "HAVEN: image type {} ({}x{}) not yet supported",
                    other, width, height
                );
                Ok(None)
            }
        }
    }

    /// HAVEN: decode a SPICE_IMAGE_TYPE_BITMAP whose pixel data is inline
    /// immediately after the bitmap header. Header (packed, little-endian):
    ///   format u8, flags u8, x u32, y u32, stride u32, palette u32  (18 bytes)
    /// then `stride * height` bytes of pixels. Returns RGBA, top-down.
    fn decode_bitmap_inline(
        &self,
        data: &[u8],
        hdr_off: usize,
        width: u32,
        height: u32,
    ) -> Result<Option<(Vec<u8>, u32, u32)>> {
        if hdr_off + 18 > data.len() {
            warn!("Bitmap header offset {} out of bounds", hdr_off);
            return Ok(None);
        }
        let format = data[hdr_off];
        let flags = data[hdr_off + 1];
        let stride = u32::from_le_bytes([
            data[hdr_off + 10],
            data[hdr_off + 11],
            data[hdr_off + 12],
            data[hdr_off + 13],
        ]) as usize;
        // SPICE_BITMAP_FLAGS_TOP_DOWN = 1 << 2; bottom-up otherwise.
        let top_down = flags & 0x04 != 0;
        let bpp: usize = match format {
            SPICE_BITMAP_FMT_32BIT | SPICE_BITMAP_FMT_RGBA => 4,
            SPICE_BITMAP_FMT_24BIT => 3,
            _ => {
                warn!("HAVEN: bitmap format {} not yet supported", format);
                return Ok(None);
            }
        };
        let pix_off = hdr_off + 18;
        let needed = stride.saturating_mul(height as usize);
        if pix_off + needed > data.len() {
            warn!(
                "Bitmap pixels out of bounds: need {} from {}, have {}",
                needed,
                pix_off,
                data.len()
            );
            return Ok(None);
        }
        let w = width as usize;
        let h = height as usize;
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            let sy = if top_down { row } else { h - 1 - row };
            let ro = pix_off + sy * stride;
            for x in 0..w {
                let po = ro + x * bpp;
                match format {
                    // SPICE 32BIT is BGRx (opaque); force alpha to 255.
                    SPICE_BITMAP_FMT_32BIT => {
                        rgba.push(data[po + 2]);
                        rgba.push(data[po + 1]);
                        rgba.push(data[po]);
                        rgba.push(255);
                    }
                    SPICE_BITMAP_FMT_RGBA => {
                        rgba.extend_from_slice(&data[po..po + 4]);
                    }
                    SPICE_BITMAP_FMT_24BIT => {
                        rgba.push(data[po + 2]);
                        rgba.push(data[po + 1]);
                        rgba.push(data[po]);
                        rgba.push(255);
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(Some((rgba, width, height)))
    }

    /// Decode LZ4 compressed image
    fn decode_lz4(
        &self,
        compressed_data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Option<(Vec<u8>, u32, u32)>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use lz4::Decoder;
            use std::io::Read;

            let mut decoder = Decoder::new(compressed_data)
                .map_err(|e| SpiceError::Protocol(format!("Failed to create LZ4 decoder: {e}")))?;

            let mut decompressed = Vec::new();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|e| SpiceError::Protocol(format!("Failed to decompress LZ4: {e}")))?;

            // Assume decompressed data is RGBA
            Ok(Some((decompressed, width, height)))
        }

        #[cfg(target_arch = "wasm32")]
        {
            // LZ4 is not supported in WASM builds (requires C compilation)
            let _ = (compressed_data, width, height); // Suppress unused warnings
            Err(SpiceError::Protocol(
                "LZ4 decompression is not supported in WebAssembly builds".to_string(),
            ))
        }
    }

    /// Decode JPEG image
    fn decode_jpeg(&self, jpeg_data: &[u8]) -> Result<Option<(Vec<u8>, u32, u32)>> {
        use jpeg_decoder::Decoder;

        let mut decoder = Decoder::new(jpeg_data);
        let pixels = decoder
            .decode()
            .map_err(|e| SpiceError::Protocol(format!("Failed to decode JPEG: {e}")))?;

        let info = decoder.info().unwrap();
        let width = info.width as u32;
        let height = info.height as u32;

        // Convert to RGBA if needed
        let rgba_data = match info.pixel_format {
            jpeg_decoder::PixelFormat::RGB24 => {
                let mut rgba = Vec::with_capacity(pixels.len() * 4 / 3);
                for chunk in pixels.chunks(3) {
                    rgba.push(chunk[0]); // R
                    rgba.push(chunk[1]); // G
                    rgba.push(chunk[2]); // B
                    rgba.push(255); // A
                }
                rgba
            }
            jpeg_decoder::PixelFormat::L8 => {
                let mut rgba = Vec::with_capacity(pixels.len() * 4);
                for &gray in &pixels {
                    rgba.push(gray); // R
                    rgba.push(gray); // G
                    rgba.push(gray); // B
                    rgba.push(255); // A
                }
                rgba
            }
            _ => {
                warn!("Unsupported JPEG pixel format: {:?}", info.pixel_format);
                return Ok(None);
            }
        };

        Ok(Some((rgba_data, width, height)))
    }

    /// Decode LZ compressed image (SPICE custom LZ format)
    /// This is a simplified implementation - full LZ support would require implementing the SPICE LZ algorithm
    /// Decode a SPICE LZ_RGB image stream (begins at the "LZ  " magic).
    /// Port of spice-common lz.c (header big-endian; LZSS body per
    /// lz_decompress_tmpl.c). Returns top-down RGBA. Verified against QEMU
    /// `image-compression=lz` (Ubuntu installer frame). RGB16 / PLT deferred.
    fn decode_lz(&self, stream: &[u8]) -> Result<Option<(Vec<u8>, u32, u32)>> {
        if stream.len() < 28 {
            warn!("LZ stream too short ({} bytes)", stream.len());
            return Ok(None);
        }
        let be = |o: usize| {
            u32::from_be_bytes([stream[o], stream[o + 1], stream[o + 2], stream[o + 3]])
        };
        let magic = be(0);
        if magic != 0x2020_5a4c {
            warn!("LZ bad magic {magic:#010x}");
            return Ok(None);
        }
        let lz_type = be(8);
        let width = be(12);
        let height = be(16);
        let top_down = be(24) != 0;
        let n_pixels = width as usize * height as usize;
        if n_pixels == 0 || n_pixels > 64 * 1024 * 1024 {
            warn!("LZ implausible size {width}x{height}");
            return Ok(None);
        }
        let body = &stream[28..];

        // LZ image sub-types (lz_common.h): RGB16=6, RGB24=7, RGB32=8, RGBA=9.
        // RGB24/RGB32 read 3 stream bytes/pixel (b,g,r); RGBA adds an alpha pass.
        let rgba_alpha = match lz_type {
            7 | 8 => false,
            9 => true,
            6 => {
                warn!("LZ RGB16 sub-type not yet implemented");
                return Ok(None);
            }
            other => {
                warn!("LZ image sub-type {other} not supported");
                return Ok(None);
            }
        };

        // Decompress the RGB pass into a BGRX (4 bytes/pixel) buffer.
        let mut bgrx = vec![0u8; n_pixels * 4];
        let consumed = match lz_decompress(body, &mut bgrx, n_pixels, false) {
            Some(c) => c,
            None => {
                warn!("LZ rgb decompress failed/truncated");
                return Ok(None);
            }
        };
        if rgba_alpha {
            // Second LZSS stream carries the alpha (pad) byte per pixel.
            if lz_decompress(&body[consumed..], &mut bgrx, n_pixels, true).is_none() {
                warn!("LZ alpha decompress failed");
                return Ok(None);
            }
        } else {
            for px in bgrx.chunks_exact_mut(4) {
                px[3] = 255; // opaque
            }
        }

        // BGRX -> top-down RGBA (matches decode_bitmap_inline output).
        let w = width as usize;
        let h = height as usize;
        let mut rgba = vec![0u8; n_pixels * 4];
        for y in 0..h {
            let sy = if top_down { y } else { h - 1 - y };
            for x in 0..w {
                let s = (sy * w + x) * 4;
                let d = (y * w + x) * 4;
                rgba[d] = bgrx[s + 2];
                rgba[d + 1] = bgrx[s + 1];
                rgba[d + 2] = bgrx[s];
                rgba[d + 3] = bgrx[s + 3];
            }
        }
        Ok(Some((rgba, width, height)))
    }

    /// Decode zlib compressed image
    fn decode_zlib(
        &self,
        compressed_data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Option<(Vec<u8>, u32, u32)>> {
        use flate2::read::ZlibDecoder;
        use std::io::Read;

        let mut decoder = ZlibDecoder::new(compressed_data);
        let mut decompressed = Vec::new();

        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| SpiceError::Protocol(format!("Failed to decompress zlib: {e}")))?;

        // Assume decompressed data is RGBA
        Ok(Some((decompressed, width, height)))
    }

    pub async fn run(&mut self) -> Result<()> {
        info!(
            "DisplayChannel: Starting event loop for channel {}",
            self.connection.channel_id
        );
        eprintln!("DisplayChannel: Entering message read loop");
        loop {
            eprintln!("DisplayChannel: Waiting for message...");
            match self.connection.read_message().await {
                Ok((header, data)) => {
                    eprintln!("DisplayChannel: Got message!");
                    self.handle_message(&header, &data).await?;
                }
                Err(e) => {
                    eprintln!("DisplayChannel: Error reading message: {e}");
                    return Err(e);
                }
            }
        }
    }

    async fn handle_mode_message(&mut self, data: &[u8]) -> Result<()> {
        // Parse mode message to get display dimensions and format
        // This is a simplified implementation
        if data.len() >= 12 {
            let width = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            let height = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
            let format = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);

            info!("Display mode: {}x{}, format: {}", width, height, format);

            // Create primary surface (ID 0)
            eprintln!(
                "DisplayChannel: Creating primary surface {}x{} format {}",
                width, height, format
            );
            self.surfaces.insert(
                0,
                DisplaySurface {
                    width,
                    height,
                    format,
                    data: vec![0; (width * height * 4) as usize], // Assuming 32-bit RGBA
                },
            );

            // Notify about primary surface
            self.notify_update(0);
        }

        Ok(())
    }

    async fn handle_draw_message(&mut self, msg_type: u16, data: &[u8]) -> Result<()> {
        match msg_type {
            x if x == DisplayChannelMessage::DrawFill as u16 => {
                // Implemented in Phase F (solid-brush rect fill). Skip rather
                // than paint a fake test pattern.
                debug!("DrawFill ({} bytes) — not yet implemented, skipping", data.len());
            }
            x if x == DisplayChannelMessage::DrawCopy as u16 => {
                // HAVEN: explicit wire parse. Upstream's binrw SpiceDrawCopy uses
                // 64-bit pointers, spurious padding and a fixed-size clip — none
                // match the SPICE wire (verified by hex capture against QEMU).
                // Layout (little-endian):
                //   surface_id u32 | box Rect{top,left,bottom,right} (16)
                //   | clip.type u8 [+ u32 clip-data ptr if RECTS]
                //   | src_image u32 (offset into this message body)
                //   | src_area Rect (16) | rop u16 | scale u8 | mask ...
                // SPICE wire pointers are 32-bit offsets into the body (@ptr32).
                if data.len() < 41 {
                    warn!("DrawCopy message too short: {} bytes", data.len());
                    return Ok(());
                }
                let rd_i32 = |o: usize| {
                    i32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
                };
                let rd_u32 = |o: usize| {
                    u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
                };
                let surface_id = rd_u32(0);
                let bbox = SpiceRect {
                    top: rd_i32(4),
                    left: rd_i32(8),
                    bottom: rd_i32(12),
                    right: rd_i32(16),
                };
                let clip_type = data[20];
                let mut off = 21usize;
                if clip_type == ClipType::Rects as u8 {
                    off += 4; // skip the 32-bit clip-data pointer
                }
                if off + 20 > data.len() {
                    warn!("DrawCopy truncated before src_area");
                    return Ok(());
                }
                let src_image = rd_u32(off) as usize;
                off += 4;
                let src_area = SpiceRect {
                    top: rd_i32(off),
                    left: rd_i32(off + 4),
                    bottom: rd_i32(off + 8),
                    right: rd_i32(off + 12),
                };

                let decoded_image = self.decode_image_at(data, src_image)?;

                if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                    match decoded_image {
                        Some((image_data, img_width, img_height)) => {
                            let bytes_per_pixel = 4;
                            let surface_stride = surface.width as usize * bytes_per_pixel;
                            let image_stride = img_width as usize * bytes_per_pixel;

                            let src_left = src_area.left.max(0) as usize;
                            let src_top = src_area.top.max(0) as usize;
                            let src_right = src_area.right.min(img_width as i32) as usize;
                            let src_bottom = src_area.bottom.min(img_height as i32) as usize;

                            let dst_left = bbox.left.max(0) as usize;
                            let dst_top = bbox.top.max(0) as usize;
                            let dst_right = bbox.right.min(surface.width as i32) as usize;
                            let dst_bottom = bbox.bottom.min(surface.height as i32) as usize;

                            // saturating_sub: dst/src rects may exceed the surface.
                            let copy_width = src_right
                                .saturating_sub(src_left)
                                .min(dst_right.saturating_sub(dst_left));
                            let copy_height = src_bottom
                                .saturating_sub(src_top)
                                .min(dst_bottom.saturating_sub(dst_top));

                            for y in 0..copy_height {
                                let src_y = src_top + y;
                                let dst_y = dst_top + y;
                                if src_y < img_height as usize && dst_y < surface.height as usize {
                                    let src_row_offset = src_y * image_stride;
                                    let dst_row_offset = dst_y * surface_stride;
                                    for x in 0..copy_width {
                                        let src_x = src_left + x;
                                        let dst_x = dst_left + x;
                                        if src_x < img_width as usize
                                            && dst_x < surface.width as usize
                                        {
                                            let so = src_row_offset + src_x * bytes_per_pixel;
                                            let dofs = dst_row_offset + dst_x * bytes_per_pixel;
                                            if so + 4 <= image_data.len()
                                                && dofs + 4 <= surface.data.len()
                                            {
                                                surface.data[dofs..dofs + 4]
                                                    .copy_from_slice(&image_data[so..so + 4]);
                                            }
                                        }
                                    }
                                }
                            }
                            self.notify_update(surface_id);
                        }
                        None => {
                            warn!(
                                "DrawCopy: could not decode source image at offset {} (surface {})",
                                src_image, surface_id
                            );
                        }
                    }
                }
            }
            _ => {
                debug!("Unhandled draw message type: {}", msg_type);
            }
        }

        Ok(())
    }

    async fn handle_stream_message(&mut self, msg_type: u16, data: &[u8]) -> Result<()> {
        match msg_type {
            x if x == DisplayChannelMessage::StreamCreate as u16 => {
                debug!("Handle stream create");
                let mut cursor = std::io::Cursor::new(data);
                let stream_create = SpiceStreamCreate::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse StreamCreate: {e}"))
                })?;

                info!(
                    "Created stream {} - codec: {}, dimensions: {}x{} -> {}x{}",
                    stream_create.id,
                    stream_create.codec_type,
                    stream_create.src_width,
                    stream_create.src_height,
                    stream_create.stream_width,
                    stream_create.stream_height
                );

                // Store stream info for processing subsequent data
                self.active_streams.insert(
                    stream_create.id,
                    StreamInfo {
                        id: stream_create.id,
                        codec_type: stream_create.codec_type,
                        width: stream_create.stream_width,
                        height: stream_create.stream_height,
                        dest_rect: stream_create.dest.clone(),
                    },
                );
            }
            x if x == DisplayChannelMessage::StreamData as u16 => {
                debug!("Handle stream data");
                let mut cursor = std::io::Cursor::new(data);
                let stream_data = SpiceStreamData::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse StreamData: {e}"))
                })?;

                debug!(
                    "Received {} bytes for stream {}",
                    stream_data.data_size, stream_data.id
                );

                // TODO: Decode stream data and apply to surface
                // This would involve:
                // 1. Finding the decoder for this stream ID
                // 2. Decoding the data based on codec type
                // 3. Applying decoded frame to the display surface
            }
            x if x == DisplayChannelMessage::StreamDestroy as u16 => {
                debug!("Handle stream destroy");
                let mut cursor = std::io::Cursor::new(data);
                let stream_destroy = SpiceStreamDestroy::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse StreamDestroy: {e}"))
                })?;

                info!("Destroyed stream {}", stream_destroy.id);

                // Clean up stream info
                self.active_streams.remove(&stream_destroy.id);
            }
            _ => {
                debug!("Unhandled stream message type: {}", msg_type);
            }
        }

        Ok(())
    }
}

impl Channel for DisplayChannel {
    async fn handle_message(&mut self, header: &SpiceDataHeader, data: &[u8]) -> Result<()> {
        eprintln!(
            "DisplayChannel: Received message type {} with {} bytes",
            header.msg_type,
            data.len()
        );

        // Log specific message types for debugging
        match header.msg_type {
            101 => eprintln!("  -> SPICE_MSG_DISPLAY_MODE"),
            318 => eprintln!("  -> SPICE_MSG_DISPLAY_SURFACE_CREATE"),
            319 => eprintln!("  -> SPICE_MSG_DISPLAY_SURFACE_DESTROY"),
            122 => eprintln!("  -> SPICE_MSG_DISPLAY_STREAM_CREATE"),
            123 => eprintln!("  -> SPICE_MSG_DISPLAY_STREAM_DATA"),
            125 => eprintln!("  -> SPICE_MSG_DISPLAY_STREAM_DESTROY"),
            302 => eprintln!("  -> SPICE_MSG_DISPLAY_DRAW_FILL"),
            304 => eprintln!("  -> SPICE_MSG_DISPLAY_DRAW_COPY"),
            _ => {}
        }

        match header.msg_type {
            x if x == DisplayChannelMessage::Mode as u16 => {
                debug!("Received display mode");
                self.handle_mode_message(data).await?;
            }
            x if x == DisplayChannelMessage::Mark as u16 => {
                debug!("Received display mark");

                // Parse the mark message
                let mut cursor = std::io::Cursor::new(data);
                let mark_msg = SpiceMsgDisplayMark::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse DisplayMark: {e}"))
                })?;

                info!("Display mark received: {}", mark_msg.mark);

                // Store the mark for synchronization tracking
                self.last_mark = Some(mark_msg.mark);

                // According to the protocol, we should expose the display content after receiving a mark
                // Notify any update callbacks about the current surface state
                if let Some(surface_id) = self.surfaces.keys().next() {
                    self.notify_update(*surface_id);
                }

                // The MARK message itself doesn't require a specific acknowledgment response
                // It's a synchronization point to indicate when display content should be exposed
            }
            x if x == DisplayChannelMessage::Reset as u16 => {
                debug!("Received display reset");
                // Reset display state - clear all surfaces and streams
                self.surfaces.clear();
                self.active_streams.clear();
                self.monitors.clear();
            }
            x if x == DisplayChannelMessage::InvalList as u16 => {
                debug!("Received invalidation list");

                // Parse the invalidation list: starts with a u16 count, followed by u64 IDs
                if data.len() >= 2 {
                    let count = u16::from_le_bytes([data[0], data[1]]) as usize;
                    let expected_size = 2 + (count * 8); // 2 bytes for count + count * 8 bytes for u64 IDs

                    if data.len() >= expected_size {
                        let mut removed_count = 0;
                        for i in 0..count {
                            let offset = 2 + (i * 8);
                            let id = u64::from_le_bytes([
                                data[offset],
                                data[offset + 1],
                                data[offset + 2],
                                data[offset + 3],
                                data[offset + 4],
                                data[offset + 5],
                                data[offset + 6],
                                data[offset + 7],
                            ]);

                            if self.image_cache.remove(id) {
                                removed_count += 1;
                                debug!("Invalidated cached resource ID: {}", id);
                            }
                        }

                        info!(
                            "Invalidated {} out of {} cached resources (cache size: {})",
                            removed_count,
                            count,
                            self.image_cache.len()
                        );
                    } else {
                        warn!(
                            "INVAL_LIST message too small: expected {} bytes, got {}",
                            expected_size,
                            data.len()
                        );
                    }
                } else {
                    warn!("INVAL_LIST message too small to contain count");
                }
            }
            x if x == DisplayChannelMessage::InvalAllPixmaps as u16 => {
                debug!("Received invalidate all pixmaps");

                // Clear the entire image cache
                let cache_size = self.image_cache.len();
                self.image_cache.clear();

                info!(
                    "Invalidated all pixmaps - cleared {} cached images",
                    cache_size
                );
            }
            x if (DisplayChannelMessage::DrawFill as u16
                ..=DisplayChannelMessage::DrawAlphaBlend as u16)
                .contains(&x) =>
            {
                self.handle_draw_message(header.msg_type, data).await?;
            }
            x if (DisplayChannelMessage::StreamCreate as u16
                ..=DisplayChannelMessage::StreamDestroyAll as u16)
                .contains(&x) =>
            {
                self.handle_stream_message(header.msg_type, data).await?;
            }
            x if x == SPICE_MSG_DISPLAY_SURFACE_CREATE => {
                debug!("Received surface create");
                let mut cursor = std::io::Cursor::new(data);
                let surface_create = SpiceMsgSurfaceCreate::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse SurfaceCreate: {e}"))
                })?;

                info!(
                    "Creating surface {} - {}x{} format: {}",
                    surface_create.surface_id,
                    surface_create.width,
                    surface_create.height,
                    surface_create.format
                );

                // Create new surface
                let data_size = (surface_create.width * surface_create.height * 4) as usize;
                self.surfaces.insert(
                    surface_create.surface_id,
                    DisplaySurface {
                        width: surface_create.width,
                        height: surface_create.height,
                        format: surface_create.format,
                        data: vec![0; data_size],
                    },
                );

                // Notify about new surface
                self.notify_update(surface_create.surface_id);
            }
            x if x == SPICE_MSG_DISPLAY_SURFACE_DESTROY => {
                debug!("Received surface destroy");
                let mut cursor = std::io::Cursor::new(data);
                let surface_destroy = SpiceMsgSurfaceDestroy::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse SurfaceDestroy: {e}"))
                })?;

                info!("Destroying surface {}", surface_destroy.surface_id);

                self.surfaces.remove(&surface_destroy.surface_id);
            }
            x if x == SPICE_MSG_DISPLAY_MONITORS_CONFIG => {
                debug!("Received monitors config");
                let mut cursor = std::io::Cursor::new(data);
                let monitors_config = SpiceMonitorsConfig::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse MonitorsConfig: {e}"))
                })?;

                info!(
                    "Monitors config: {} monitors (max {})",
                    monitors_config.count, monitors_config.max_allowed
                );

                self.monitors = monitors_config.heads;

                // Log monitor configurations
                for (i, head) in self.monitors.iter().enumerate() {
                    info!(
                        "Monitor {}: {}x{} at ({},{}) on surface {}",
                        i, head.width, head.height, head.x, head.y, head.surface_id
                    );
                }
            }
            3 => {
                // SPICE_MSG_SET_ACK
                eprintln!("DisplayChannel: Received SET_ACK message");
                // Parse the generation number
                if data.len() >= 4 {
                    let generation = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                    eprintln!("DisplayChannel: SET_ACK generation: {generation}");

                    // Send ACK_SYNC response
                    let ack_data = generation.to_le_bytes();
                    self.connection.send_message(1, &ack_data).await?; // SPICE_MSGC_ACK_SYNC
                    eprintln!("DisplayChannel: Sent ACK_SYNC response");
                }
            }
            x if x == SPICE_MSG_DISPLAY_COPY_BITS => {
                debug!("Received DISPLAY_COPY_BITS");

                // Parse the copy bits message
                let mut cursor = std::io::Cursor::new(data);
                match SpiceCopyBits::read(&mut cursor) {
                    Ok(copy_bits) => {
                        let surface_id = copy_bits.base.surface_id;
                        let dest_rect = &copy_bits.base.box_;
                        let src_pos = &copy_bits.src_pos;

                        debug!(
                            "COPY_BITS: surface {} from ({}, {}) to rect ({},{}) - ({},{})",
                            surface_id,
                            src_pos.x,
                            src_pos.y,
                            dest_rect.left,
                            dest_rect.top,
                            dest_rect.right,
                            dest_rect.bottom
                        );

                        // Perform the copy operation
                        if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                            // Calculate dimensions
                            let rect_width = (dest_rect.right - dest_rect.left) as usize;
                            let rect_height = (dest_rect.bottom - dest_rect.top) as usize;
                            let bytes_per_pixel = 4;
                            let stride = surface.width as usize * bytes_per_pixel;

                            // Validate bounds
                            let src_x = src_pos.x as usize;
                            let src_y = src_pos.y as usize;
                            let dest_x = dest_rect.left as usize;
                            let dest_y = dest_rect.top as usize;

                            if src_x + rect_width <= surface.width as usize
                                && src_y + rect_height <= surface.height as usize
                                && dest_x + rect_width <= surface.width as usize
                                && dest_y + rect_height <= surface.height as usize
                            {
                                // Create a temporary buffer to hold the source data
                                let mut temp_buffer =
                                    Vec::with_capacity(rect_width * rect_height * bytes_per_pixel);

                                // Copy source data to temp buffer
                                for y in 0..rect_height {
                                    let src_offset = (src_y + y) * stride + src_x * bytes_per_pixel;
                                    let row_data = &surface.data
                                        [src_offset..src_offset + rect_width * bytes_per_pixel];
                                    temp_buffer.extend_from_slice(row_data);
                                }

                                // Copy from temp buffer to destination
                                for y in 0..rect_height {
                                    let dest_offset =
                                        (dest_y + y) * stride + dest_x * bytes_per_pixel;
                                    let src_row_offset = y * rect_width * bytes_per_pixel;
                                    surface.data
                                        [dest_offset..dest_offset + rect_width * bytes_per_pixel]
                                        .copy_from_slice(
                                            &temp_buffer[src_row_offset
                                                ..src_row_offset + rect_width * bytes_per_pixel],
                                        );
                                }

                                // Notify that the surface was updated
                                self.notify_update(surface_id);

                                debug!("COPY_BITS completed successfully");
                            } else {
                                warn!(
                                    "COPY_BITS out of bounds: src ({},{}) dest ({},{}) rect {}x{} surface {}x{}",
                                    src_x, src_y, dest_x, dest_y, rect_width, rect_height,
                                    surface.width, surface.height
                                );
                            }
                        } else {
                            warn!("COPY_BITS: Surface {} not found", surface_id);
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse COPY_BITS message: {}", e);
                    }
                }
            }
            x if x == SPICE_MSG_DISPLAY_INVAL_PALETTE => {
                // This message tells the client to invalidate a specific cached palette.
                // Since we don't currently cache palettes (only full images), we can
                // safely acknowledge this message.
                if data.len() >= 8 {
                    let palette_id = u64::from_le_bytes([
                        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
                    ]);
                    debug!(
                        "Received DISPLAY_INVAL_PALETTE for palette_id: {} (no-op)",
                        palette_id
                    );
                } else {
                    debug!("Received DISPLAY_INVAL_PALETTE - palette cache entry cleared (no-op)");
                }
            }
            x if x == SPICE_MSG_DISPLAY_INVAL_ALL_PALETTES => {
                // This message tells the client to invalidate all cached palettes.
                // Since we don't currently cache palettes (only full images), we can
                // safely acknowledge this message. Palettes are used for indexed color
                // images which we convert to RGBA during decoding.
                debug!("Received DISPLAY_INVAL_ALL_PALETTES - palette cache cleared (no-op)");
            }
            x if x == SPICE_MSG_PING => {
                debug!("Received PING message in display channel");
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
                            0 => info!("Display server info: {}", message),
                            1 => warn!("Display server warning: {}", message),
                            2 => error!("Display server error: {}", message),
                            _ => debug!("Display server notification: {}", message),
                        }
                    }
                }
            }
            x if x == SPICE_MSG_DISCONNECTING => {
                info!("Display channel: Server is disconnecting");
                return Err(SpiceError::ConnectionClosed);
            }
            x if x == SPICE_MSG_MIGRATE => {
                warn!("Display channel: Migration not implemented");
            }
            x if x == SPICE_MSG_MIGRATE_DATA => {
                warn!("Display channel: Migration data not implemented");
            }
            x if x == SPICE_MSG_WAIT_FOR_CHANNELS => {
                warn!("Display channel: Wait for channels not implemented");
            }
            _ => {
                warn!("Unknown display message type: {}", header.msg_type);
            }
        }

        Ok(())
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Display
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use binrw::BinWrite;
    use std::io::Cursor;

    #[test]
    fn test_copy_bits_message_parsing() {
        // Create a test SpiceCopyBits message
        let copy_bits = SpiceCopyBits {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 100,
                    top: 50,
                    right: 200,
                    bottom: 150,
                },
                clip: SpiceClip {
                    clip_type: 0, // SPICE_CLIP_TYPE_NONE
                    data: 0,      // No clip data for NONE type
                },
            },
            src_pos: SpicePoint { x: 10, y: 20 },
        };

        // Serialize the message
        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        copy_bits.write(&mut cursor).unwrap();

        // Parse it back
        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceCopyBits::read(&mut read_cursor).unwrap();

        // Verify the parsed message matches
        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.base.box_.left, 100);
        assert_eq!(parsed.base.box_.top, 50);
        assert_eq!(parsed.base.box_.right, 200);
        assert_eq!(parsed.base.box_.bottom, 150);
        assert_eq!(parsed.src_pos.x, 10);
        assert_eq!(parsed.src_pos.y, 20);
    }

    #[test]
    fn test_copy_bits_bounds_validation() {
        // Create a surface for testing
        let mut surface = DisplaySurface {
            width: 800,
            height: 600,
            format: 32, // RGBA
            data: vec![0; 800 * 600 * 4],
        };

        // Fill a test pattern in the source area
        let bytes_per_pixel = 4;
        let stride = surface.width as usize * bytes_per_pixel;
        for y in 20..120 {
            for x in 10..110 {
                let offset = y * stride + x * bytes_per_pixel;
                surface.data[offset] = 255; // R
                surface.data[offset + 1] = 100; // G
                surface.data[offset + 2] = 50; // B
                surface.data[offset + 3] = 255; // A
            }
        }

        // Simulate a copy operation (from 10,20 to 200,100 with size 100x100)
        let src_x = 10;
        let src_y = 20;
        let dest_x = 200;
        let dest_y = 100;
        let rect_width = 100;
        let rect_height = 100;

        // Perform the copy
        let mut temp_buffer = Vec::with_capacity(rect_width * rect_height * bytes_per_pixel);

        // Copy source data to temp buffer
        for y in 0..rect_height {
            let src_offset = (src_y + y) * stride + src_x * bytes_per_pixel;
            let row_data = &surface.data[src_offset..src_offset + rect_width * bytes_per_pixel];
            temp_buffer.extend_from_slice(row_data);
        }

        // Copy from temp buffer to destination
        for y in 0..rect_height {
            let dest_offset = (dest_y + y) * stride + dest_x * bytes_per_pixel;
            let src_row_offset = y * rect_width * bytes_per_pixel;
            surface.data[dest_offset..dest_offset + rect_width * bytes_per_pixel].copy_from_slice(
                &temp_buffer[src_row_offset..src_row_offset + rect_width * bytes_per_pixel],
            );
        }

        // Verify the destination area has the copied data
        for y in 100..200 {
            for x in 200..300 {
                let offset = y * stride + x * bytes_per_pixel;
                assert_eq!(surface.data[offset], 255); // R
                assert_eq!(surface.data[offset + 1], 100); // G
                assert_eq!(surface.data[offset + 2], 50); // B
                assert_eq!(surface.data[offset + 3], 255); // A
            }
        }
    }

    #[test]
    fn test_copy_bits_out_of_bounds() {
        let surface = DisplaySurface {
            width: 800,
            height: 600,
            format: 32,
            data: vec![0; 800 * 600 * 4],
        };

        // Test source out of bounds
        let src_x = 750;
        let src_y = 550;
        let rect_width = 100; // Would exceed width
        let rect_height = 100; // Would exceed height

        // This should be caught by bounds checking
        assert!(src_x + rect_width > surface.width as usize);
        assert!(src_y + rect_height > surface.height as usize);

        // Test destination out of bounds
        let dest_x = 750;
        let dest_y = 550;

        assert!(dest_x + rect_width > surface.width as usize);
        assert!(dest_y + rect_height > surface.height as usize);
    }

    #[test]
    fn test_mark_message_parsing() {
        // Create a test SpiceMsgDisplayMark message
        let mark_msg = SpiceMsgDisplayMark {
            mark: 42, // Test mark value
        };

        // Serialize the message
        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        mark_msg.write(&mut cursor).unwrap();

        // Parse the serialized message
        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceMsgDisplayMark::read(&mut read_cursor).unwrap();

        // Verify the parsed message matches the original
        assert_eq!(parsed.mark, 42);
    }

    #[test]
    fn test_mark_value_updates() {
        // Test multiple mark value scenarios
        let mark_values = vec![0, 1, 100, 255, 65535, u32::MAX];

        for mark_value in mark_values {
            let mark_msg = SpiceMsgDisplayMark { mark: mark_value };

            // Serialize and deserialize
            let mut buffer = Vec::new();
            let mut cursor = Cursor::new(&mut buffer);
            mark_msg.write(&mut cursor).unwrap();

            let mut read_cursor = Cursor::new(&buffer);
            let parsed = SpiceMsgDisplayMark::read(&mut read_cursor).unwrap();

            // Verify the value is preserved
            assert_eq!(parsed.mark, mark_value);
        }
    }

    #[test]
    fn test_image_cache_operations() {
        let mut cache = ImageCache::new();

        // Test empty cache
        assert_eq!(cache.len(), 0);
        assert!(cache.get(1).is_none());

        // Test insert and get
        let test_data = vec![1, 2, 3, 4];
        cache.insert(1, test_data.clone(), 10, 10);
        assert_eq!(cache.len(), 1);

        let cached = cache.get(1).unwrap();
        assert_eq!(cached.data, test_data);
        assert_eq!(cached.width, 10);
        assert_eq!(cached.height, 10);

        // Test multiple inserts
        cache.insert(2, vec![5, 6, 7, 8], 20, 20);
        cache.insert(3, vec![9, 10, 11, 12], 30, 30);
        assert_eq!(cache.len(), 3);

        // Test remove
        assert!(cache.remove(2));
        assert_eq!(cache.len(), 2);
        assert!(cache.get(2).is_none());
        assert!(!cache.remove(2)); // Already removed

        // Test clear
        cache.clear();
        assert_eq!(cache.len(), 0);
        assert!(cache.get(1).is_none());
        assert!(cache.get(3).is_none());
    }

    #[test]
    fn test_inval_list_message_parsing() {
        // Create a mock INVAL_LIST message with 3 IDs to invalidate
        let count: u16 = 3;
        let ids = vec![100u64, 200u64, 300u64];

        let mut message = Vec::new();
        message.extend_from_slice(&count.to_le_bytes());
        for id in &ids {
            message.extend_from_slice(&id.to_le_bytes());
        }

        // Verify message structure
        assert_eq!(message.len(), 2 + (3 * 8)); // 2 bytes count + 3 * 8 bytes IDs

        // Parse count
        let parsed_count = u16::from_le_bytes([message[0], message[1]]);
        assert_eq!(parsed_count, 3);

        // Parse IDs
        for (i, expected_id) in ids.iter().enumerate() {
            let offset = 2 + (i * 8);
            let parsed_id = u64::from_le_bytes([
                message[offset],
                message[offset + 1],
                message[offset + 2],
                message[offset + 3],
                message[offset + 4],
                message[offset + 5],
                message[offset + 6],
                message[offset + 7],
            ]);
            assert_eq!(parsed_id, *expected_id);
        }
    }

    #[test]
    fn test_cache_invalidation_logic() {
        let mut cache = ImageCache::new();

        // Populate cache with test data
        for i in 1..=10 {
            cache.insert(i, vec![i as u8; 100], i as u32, i as u32);
        }
        assert_eq!(cache.len(), 10);

        // Simulate INVAL_LIST with some IDs
        let ids_to_remove = vec![2u64, 5u64, 8u64];
        let mut removed_count = 0;
        for id in ids_to_remove {
            if cache.remove(id) {
                removed_count += 1;
            }
        }

        assert_eq!(removed_count, 3);
        assert_eq!(cache.len(), 7);

        // Verify correct entries were removed
        assert!(cache.get(2).is_none());
        assert!(cache.get(5).is_none());
        assert!(cache.get(8).is_none());

        // Verify other entries still exist
        assert!(cache.get(1).is_some());
        assert!(cache.get(3).is_some());
        assert!(cache.get(10).is_some());
    }

    #[test]
    fn test_inval_list_with_nonexistent_ids() {
        let mut cache = ImageCache::new();

        // Add only a few entries
        cache.insert(100, vec![1; 100], 10, 10);
        cache.insert(200, vec![2; 100], 20, 20);

        // Try to remove IDs that don't exist
        let ids_to_remove = vec![100u64, 150u64, 200u64, 250u64];
        let mut removed_count = 0;
        for id in ids_to_remove {
            if cache.remove(id) {
                removed_count += 1;
            }
        }

        // Should only remove the 2 that existed
        assert_eq!(removed_count, 2);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_inval_all_pixmaps() {
        let mut cache = ImageCache::new();

        // Populate cache
        for i in 1..=100 {
            cache.insert(i, vec![i as u8; 100], i as u32, i as u32);
        }
        assert_eq!(cache.len(), 100);

        // Simulate INVAL_ALL_PIXMAPS
        cache.clear();

        // Verify all entries removed
        assert_eq!(cache.len(), 0);
        for i in 1..=100 {
            assert!(cache.get(i).is_none());
        }
    }

    #[test]
    fn test_empty_inval_list() {
        let mut cache = ImageCache::new();
        cache.insert(1, vec![1; 100], 10, 10);

        // Empty invalidation list (count = 0)
        let count: u16 = 0;
        let message = count.to_le_bytes().to_vec();

        // Parse and verify
        let parsed_count = u16::from_le_bytes([message[0], message[1]]);
        assert_eq!(parsed_count, 0);
        assert_eq!(message.len(), 2);

        // Cache should remain unchanged
        assert_eq!(cache.len(), 1);
        assert!(cache.get(1).is_some());
    }

    #[test]
    fn test_draw_blackness_parsing() {
        // Create a test SpiceDrawBlackness message
        let draw_blackness = SpiceDrawBlackness {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 10,
                    top: 20,
                    right: 100,
                    bottom: 80,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            mask: SpiceQMask {
                flags: 0,
                pos: SpicePoint { x: 0, y: 0 },
                bitmap: 0,
            },
        };

        // Serialize the message
        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_blackness.write(&mut cursor).unwrap();

        // Parse it back
        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawBlackness::read(&mut read_cursor).unwrap();

        // Verify the parsed message matches
        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.base.box_.left, 10);
        assert_eq!(parsed.base.box_.top, 20);
        assert_eq!(parsed.base.box_.right, 100);
        assert_eq!(parsed.base.box_.bottom, 80);
    }

    #[test]
    fn test_draw_whiteness_parsing() {
        let draw_whiteness = SpiceDrawWhiteness {
            base: SpiceDrawBase {
                surface_id: 1,
                box_: SpiceRect {
                    left: 50,
                    top: 50,
                    right: 150,
                    bottom: 100,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            mask: SpiceQMask {
                flags: 0,
                pos: SpicePoint { x: 0, y: 0 },
                bitmap: 0,
            },
        };

        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_whiteness.write(&mut cursor).unwrap();

        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawWhiteness::read(&mut read_cursor).unwrap();

        assert_eq!(parsed.base.surface_id, 1);
        assert_eq!(parsed.base.box_.left, 50);
        assert_eq!(parsed.base.box_.top, 50);
    }

    #[test]
    fn test_draw_invers_parsing() {
        let draw_invers = SpiceDrawInvers {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 0,
                    top: 0,
                    right: 800,
                    bottom: 600,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            mask: SpiceQMask {
                flags: 0,
                pos: SpicePoint { x: 0, y: 0 },
                bitmap: 0,
            },
        };

        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_invers.write(&mut cursor).unwrap();

        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawInvers::read(&mut read_cursor).unwrap();

        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.base.box_.right, 800);
        assert_eq!(parsed.base.box_.bottom, 600);
    }

    #[test]
    fn test_draw_rop3_parsing() {
        let draw_rop3 = SpiceDrawRop3 {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 10,
                    top: 10,
                    right: 110,
                    bottom: 110,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            src_bitmap: 0x1000,
            src_area: SpiceRect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 100,
            },
            brush: SpiceBrush {
                brush_type: 1,
                color: 0xFF0000, // Red
            },
            rop3: 0xCC, // SRCCOPY
            scale_mode: 0,
            mask: SpiceQMask {
                flags: 0,
                pos: SpicePoint { x: 0, y: 0 },
                bitmap: 0,
            },
        };

        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_rop3.write(&mut cursor).unwrap();

        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawRop3::read(&mut read_cursor).unwrap();

        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.rop3, 0xCC);
        assert_eq!(parsed.src_bitmap, 0x1000);
        assert_eq!(parsed.brush.color, 0xFF0000);
    }

    #[test]
    fn test_draw_transparent_parsing() {
        let draw_transparent = SpiceDrawTransparent {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 20,
                    top: 30,
                    right: 120,
                    bottom: 130,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            src_bitmap: 0x2000,
            src_area: SpiceRect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 100,
            },
            src_color: 0xFF00FF, // Magenta transparency key
            true_color: 0xFFFFFF,
        };

        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_transparent.write(&mut cursor).unwrap();

        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawTransparent::read(&mut read_cursor).unwrap();

        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.src_bitmap, 0x2000);
        assert_eq!(parsed.src_color, 0xFF00FF);
        assert_eq!(parsed.true_color, 0xFFFFFF);
    }

    #[test]
    fn test_draw_stroke_parsing() {
        let draw_stroke = SpiceDrawStroke {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 5,
                    top: 5,
                    right: 95,
                    bottom: 95,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            path: 0x3000,
            attr: SpiceLineAttr {
                flags: 0,
                join_style: 0,
                end_style: 0,
                style_nseg: 0,
                width: 1 << 4,       // fixed28_4 format: 1.0
                miter_limit: 4 << 4, // fixed28_4 format: 4.0
            },
            brush: SpiceBrush {
                brush_type: 1,
                color: 0x0000FF, // Blue
            },
            fore_mode: 0,
            back_mode: 0,
        };

        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_stroke.write(&mut cursor).unwrap();

        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawStroke::read(&mut read_cursor).unwrap();

        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.path, 0x3000);
        assert_eq!(parsed.brush.color, 0x0000FF);
        assert_eq!(parsed.attr.width, 1 << 4);
    }

    #[test]
    fn test_draw_text_parsing() {
        let draw_text = SpiceDrawText {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 10,
                    top: 10,
                    right: 200,
                    bottom: 30,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            str_: 0x4000,
            back_area: SpiceRect {
                left: 10,
                top: 10,
                right: 200,
                bottom: 30,
            },
            fore_brush: SpiceBrush {
                brush_type: 1,
                color: 0x000000, // Black text
            },
            back_brush: SpiceBrush {
                brush_type: 1,
                color: 0xFFFFFF, // White background
            },
            fore_mode: 0,
            back_mode: 0,
        };

        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_text.write(&mut cursor).unwrap();

        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawText::read(&mut read_cursor).unwrap();

        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.str_, 0x4000);
        assert_eq!(parsed.fore_brush.color, 0x000000);
        assert_eq!(parsed.back_brush.color, 0xFFFFFF);
    }
}

/// SPICE LZSS inner decompressor (port of lz_decompress_tmpl.c).
///
/// `out` holds 4-byte BGRX pixels (`n_pixels` of them). Two modes:
/// - RGB pass (`alpha_only == false`): each literal reads 3 stream bytes
///   (b,g,r), pad set to 0; references copy whole pixels. Covers RGB24/RGB32
///   and the colour pass of RGBA. Length bias +1.
/// - Alpha pass (`alpha_only == true`, RGBA only): each literal reads 1 byte
///   into the pad/alpha lane; references copy only that lane. Length bias +3.
///
/// Returns the number of `body` bytes consumed, or `None` on truncation /
/// an out-of-range back-reference (caller then leaves the surface untouched).
fn lz_decompress(body: &[u8], out: &mut [u8], n_pixels: usize, alpha_only: bool) -> Option<usize> {
    const MAX_COPY: u32 = 32;
    const MAX_DISTANCE: u32 = 8191;
    let len_bias: u32 = if alpha_only { 3 } else { 1 };
    let mut ip = 0usize;
    let mut op = 0usize; // pixel index

    while op < n_pixels {
        let ctrl = *body.get(ip)? as u32;
        ip += 1;
        if ctrl >= MAX_COPY {
            // back-reference / run
            let mut len = (ctrl >> 5) - 1;
            let mut ofs = (ctrl & 31) << 8;
            if len == 6 {
                loop {
                    let code = *body.get(ip)? as u32;
                    ip += 1;
                    len += code;
                    if code != 255 {
                        break;
                    }
                }
            }
            let code = *body.get(ip)? as u32;
            ip += 1;
            ofs += code;
            if code == 255 && (ofs - code) == (31 << 8) {
                ofs = (*body.get(ip)? as u32) << 8;
                ip += 1;
                ofs += *body.get(ip)? as u32;
                ip += 1;
                ofs += MAX_DISTANCE;
            }
            let len = (len + len_bias) as usize;
            let ofs = (ofs + 1) as usize;
            if ofs > op || op + len > n_pixels {
                return None; // ref underflow / output overflow
            }
            let mut refp = op - ofs;
            // Element-wise so the run case (ofs == 1) propagates correctly.
            for _ in 0..len {
                if alpha_only {
                    out[op * 4 + 3] = out[refp * 4 + 3];
                } else {
                    let (s, d) = (refp * 4, op * 4);
                    out[d] = out[s];
                    out[d + 1] = out[s + 1];
                    out[d + 2] = out[s + 2];
                    out[d + 3] = out[s + 3];
                }
                refp += 1;
                op += 1;
            }
        } else {
            // literal run, count biased by 1
            let count = (ctrl + 1) as usize;
            if op + count > n_pixels {
                return None;
            }
            for _ in 0..count {
                let o = op * 4;
                if alpha_only {
                    out[o + 3] = *body.get(ip)?;
                    ip += 1;
                } else {
                    out[o] = *body.get(ip)?;
                    out[o + 1] = *body.get(ip + 1)?;
                    out[o + 2] = *body.get(ip + 2)?;
                    out[o + 3] = 0;
                    ip += 3;
                }
                op += 1;
            }
        }
    }
    Some(ip)
}
