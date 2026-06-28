use binrw::binrw;
use serde::{Deserialize, Serialize};

pub const SPICE_MAGIC: u32 = 0x51444552; // "REDQ" - official SPICE protocol magic
pub const SPICE_VERSION_MAJOR: u32 = 2;
pub const SPICE_VERSION_MINOR: u32 = 2;

// Link error codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum LinkError {
    Ok = 0,
    Error = 1,
    InvalidMagic = 2,
    InvalidData = 3,
    VersionMismatch = 4,
    NeedSecured = 5,
    NeedUnsecured = 6,
    PermissionDenied = 7,
    BadConnectionId = 8,
    ChannelNotAvailable = 9,
}

// Notification severity
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NotifySeverity {
    Info = 0,
    Warn = 1,
    Error = 2,
}

// Notification visibility
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NotifyVisibility {
    Low = 0,
    Medium = 1,
    High = 2,
}

// Clip type
#[binrw]
#[brw(repr = u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ClipType {
    None = 0,
    Rects = 1,
}

// Brush type
#[binrw]
#[brw(repr = u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BrushType {
    None = 0,
    Solid = 1,
    Pattern = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ChannelType {
    Main = 1,
    Display = 2,
    Inputs = 3,
    Cursor = 4,
    Playback = 5,
    Record = 6,
    Tunnel = 7,
    SmartCard = 8,
    UsbreDirect = 9,
    Port = 10,
    WebDav = 11,
}

impl From<u8> for ChannelType {
    fn from(value: u8) -> Self {
        match value {
            1 => ChannelType::Main,
            2 => ChannelType::Display,
            3 => ChannelType::Inputs,
            4 => ChannelType::Cursor,
            5 => ChannelType::Playback,
            6 => ChannelType::Record,
            7 => ChannelType::Tunnel,
            8 => ChannelType::SmartCard,
            9 => ChannelType::UsbreDirect,
            10 => ChannelType::Port,
            11 => ChannelType::WebDav,
            _ => ChannelType::Main, // Default fallback
        }
    }
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceDataHeader {
    pub serial: u64,   // 8 bytes
    pub msg_type: u16, // 2 bytes
    // NO PADDING - binrw handles this correctly
    pub msg_size: u32, // 4 bytes
    pub sub_list: u32, // 4 bytes
} // Total: 18 bytes

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceLinkHeader {
    pub magic: u32,
    pub major_version: u32,
    pub minor_version: u32,
    pub size: u32,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceLinkMess {
    pub connection_id: u32,
    pub channel_type: u8,
    pub channel_id: u8,
    // HAVEN: SPICE wire SpiceLinkMess is PACKED (18 bytes, no alignment
    // padding) — verified against remote-viewer's link (caps_offset=18). The
    // earlier pad_before=2 made it 20 bytes; with 0 advertised caps the server
    // tolerated it, but any advertised cap shifted num_channel_caps by one word
    // so the server read a garbage count (65536) and rejected the link with
    // INVALID_DATA. caps_offset must equal this struct's packed size.
    pub num_common_caps: u32,
    pub num_channel_caps: u32,
    pub caps_offset: u32,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceLinkReply {
    pub magic: u32,
    pub major_version: u32,
    pub minor_version: u32,
    pub size: u32,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone)]
pub struct SpiceLinkReplyData {
    pub error: u32,         // LinkError enum value
    pub pub_key: [u8; 162], // RSA 1024-bit public key (162 bytes)
    pub num_common_caps: u32,
    pub num_channel_caps: u32,
    pub caps_offset: u32,
}

// Authentication mechanism selection structure
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceLinkAuthMechanism {
    pub auth_mechanism: u32, // SPICE_COMMON_CAP_AUTH_* value
}

// Mini header structure (for future implementation)
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceMiniDataHeader {
    pub msg_type: u16,
    pub msg_size: u32,
}

// Wait for channels message structure
// Server sends this to request client to wait until specific channels are ready
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceMsgWaitForChannels {
    pub wait_count: u8,
    #[br(count = wait_count)]
    pub wait_list: Vec<ChannelId>,
}

// Main channel messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum MainChannelMessage {
    MigrateBegin = 101,
    MigrateCancel = 102,
    Init = 103,
    ChannelsList = 104,
    MouseMode = 105,
    MultiMediaTime = 106,
    AgentConnected = 107,
    AgentDisconnected = 108,
    AgentData = 109,
    AgentToken = 110,
    MigrateSwitchHost = 111,
    MigrateEnd = 112,
    Name = 113,
    Uuid = 114,
    AgentConnectedTokens = 115,
    MigrateBeginSeamless = 116,
    MigrateDstSeamlessAck = 117,
    MigrateDstSeamlessNack = 118,
}

// Display channel messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum DisplayChannelMessage {
    Mode = 101,
    Mark = 102,
    Reset = 103,
    CopyBits = 104,
    InvalList = 105,
    InvalAllPixmaps = 106,
    InvalAllPalettes = 107,
    StreamCreate = 122,
    StreamData = 123,
    StreamClip = 124,
    StreamDestroy = 125,
    StreamDestroyAll = 126,
    DrawFill = 302,
    DrawOpaque = 303,
    DrawCopy = 304,
    DrawBlend = 305,
    DrawBlackness = 306,
    DrawWhiteness = 307,
    DrawInvers = 308,
    DrawRop3 = 309,
    DrawStroke = 310,
    DrawText = 311,
    DrawTransparent = 312,
    DrawAlphaBlend = 317,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceRect {
    // SPICE wire order is top, left, bottom, right (verified against QEMU).
    pub top: i32,
    pub left: i32,
    pub bottom: i32,
    pub right: i32,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpicePoint {
    pub x: i32,
    pub y: i32,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpicePoint16 {
    pub x: i16,
    pub y: i16,
}

// Fixed-point 28.4 format (28 bits integer, 4 bits fraction)
pub type Fixed28_4 = i32;

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpicePointFix {
    pub x: Fixed28_4,
    pub y: Fixed28_4,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceTransform {
    pub t00: u32,
    pub t01: u32,
    pub t02: u32,
    pub t10: u32,
    pub t11: u32,
    pub t12: u32,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceSize {
    pub width: u32,
    pub height: u32,
}

// Common message type constants (all channels)
pub const SPICE_MSG_MIGRATE: u16 = 1;
pub const SPICE_MSG_MIGRATE_DATA: u16 = 2;
pub const SPICE_MSG_SET_ACK: u16 = 3;
pub const SPICE_MSG_PING: u16 = 4;
pub const SPICE_MSG_WAIT_FOR_CHANNELS: u16 = 5;
pub const SPICE_MSG_DISCONNECTING: u16 = 6;
pub const SPICE_MSG_NOTIFY: u16 = 7;

// Client to server common messages
pub const SPICE_MSGC_ACK_SYNC: u16 = 1;
pub const SPICE_MSGC_ACK: u16 = 2;
pub const SPICE_MSGC_PONG: u16 = 3;
pub const SPICE_MSGC_MIGRATE_FLUSH_MARK: u16 = 4;
pub const SPICE_MSGC_MIGRATE_DATA: u16 = 5;
pub const SPICE_MSGC_DISCONNECTING: u16 = 6;

// Client to server main channel messages
pub const SPICE_MSGC_MAIN_CLIENT_INFO: u16 = 101;
pub const SPICE_MSGC_MAIN_MIGRATE_CONNECTED: u16 = 102;
pub const SPICE_MSGC_MAIN_MIGRATE_CONNECT_ERROR: u16 = 103;
pub const SPICE_MSGC_MAIN_ATTACH_CHANNELS: u16 = 104;

// Common channel capabilities
pub const SPICE_COMMON_CAP_PROTOCOL_AUTH_SELECTION: u32 = 0;
pub const SPICE_COMMON_CAP_AUTH_SPICE: u32 = 1;
pub const SPICE_COMMON_CAP_AUTH_SASL: u32 = 2;
pub const SPICE_COMMON_CAP_MINI_HEADER: u32 = 3;

// Display channel capabilities
pub const SPICE_DISPLAY_CAP_SIZED_STREAM: u32 = 0;
pub const SPICE_DISPLAY_CAP_MONITORS_CONFIG: u32 = 1;
pub const SPICE_DISPLAY_CAP_COMPOSITE: u32 = 2;
pub const SPICE_DISPLAY_CAP_A8_SURFACE: u32 = 3;
pub const SPICE_DISPLAY_CAP_STREAM_REPORT: u32 = 4;
pub const SPICE_DISPLAY_CAP_LZ4_COMPRESSION: u32 = 5;
pub const SPICE_DISPLAY_CAP_PREF_COMPRESSION_SETTING: u32 = 6;
pub const SPICE_DISPLAY_CAP_GL_SCANOUT: u32 = 7;
pub const SPICE_DISPLAY_CAP_MULTI_CODEC: u32 = 8;
pub const SPICE_DISPLAY_CAP_CODEC_MJPEG: u32 = 9;
pub const SPICE_DISPLAY_CAP_CODEC_VP8: u32 = 10;
pub const SPICE_DISPLAY_CAP_CODEC_H264: u32 = 11;
pub const SPICE_DISPLAY_CAP_PREF_VIDEO_CODEC_TYPE: u32 = 12;
pub const SPICE_DISPLAY_CAP_CODEC_VP9: u32 = 13;
pub const SPICE_DISPLAY_CAP_CODEC_H265: u32 = 14;

// Main channel capabilities
pub const SPICE_MAIN_CAP_SEMI_SEAMLESS_MIGRATE: u32 = 0;
pub const SPICE_MAIN_CAP_NAME_AND_UUID: u32 = 1;
pub const SPICE_MAIN_CAP_AGENT_CONNECTED_TOKENS: u32 = 2;
pub const SPICE_MAIN_CAP_SEAMLESS_MIGRATE: u32 = 3;

// Client to server display channel messages
pub const SPICE_MSGC_DISPLAY_INIT: u16 = 101;

// Display init message structure
// Based on spice-protocol/spice/protocol.h
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceMsgcDisplayInit {
    // HAVEN: SPICE wire structs are PACKED (no C alignment padding) — same as
    // SpiceLinkMess. The earlier pad_before=7 made this 17 bytes and shifted
    // every field, and the struct was missing glz_dictionary_window_size
    // entirely, so the server saw a zero GLZ window and could only ever reply
    // with LZ_RGB, never GLZ_RGB. Packed layout = 14 bytes.
    pub cache_id: u8,
    pub cache_size: i64,
    pub glz_dict_id: u8,
    pub glz_dictionary_window_size: i32,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceMsgDisplayMark {
    pub mark: u32, // Mark value for synchronization
}

// Main channel message type constants
pub const SPICE_MSG_MAIN_MIGRATE_BEGIN: u16 = 101;
pub const SPICE_MSG_MAIN_MIGRATE_CANCEL: u16 = 102;
pub const SPICE_MSG_MAIN_INIT: u16 = 103;
pub const SPICE_MSG_MAIN_CHANNELS_LIST: u16 = 104;
pub const SPICE_MSG_MAIN_MOUSE_MODE: u16 = 105;
pub const SPICE_MSG_MAIN_MULTI_MEDIA_TIME: u16 = 106;
pub const SPICE_MSG_MAIN_AGENT_CONNECTED: u16 = 107;
pub const SPICE_MSG_MAIN_AGENT_DISCONNECTED: u16 = 108;
pub const SPICE_MSG_MAIN_AGENT_DATA: u16 = 109;
pub const SPICE_MSG_MAIN_AGENT_TOKEN: u16 = 110;
pub const SPICE_MSG_MAIN_MIGRATE_SWITCH_HOST: u16 = 111;
pub const SPICE_MSG_MAIN_MIGRATE_END: u16 = 112;
pub const SPICE_MSG_MAIN_NAME: u16 = 113;
pub const SPICE_MSG_MAIN_UUID: u16 = 114;
pub const SPICE_MSG_MAIN_AGENT_CONNECTED_TOKENS: u16 = 115;
pub const SPICE_MSG_MAIN_MIGRATE_BEGIN_SEAMLESS: u16 = 116;
pub const SPICE_MSG_MAIN_MIGRATE_DST_SEAMLESS_ACK: u16 = 117;
pub const SPICE_MSG_MAIN_MIGRATE_DST_SEAMLESS_NACK: u16 = 118;

pub const SPICE_MSG_DISPLAY_MODE: u16 = 101;
pub const SPICE_MSG_DISPLAY_MARK: u16 = 102;
pub const SPICE_MSG_DISPLAY_RESET: u16 = 103;
pub const SPICE_MSG_DISPLAY_COPY_BITS: u16 = 104;
pub const SPICE_MSG_DISPLAY_INVAL_LIST: u16 = 105;
pub const SPICE_MSG_DISPLAY_INVAL_ALL_PIXMAPS: u16 = 106;
pub const SPICE_MSG_DISPLAY_INVAL_PALETTE: u16 = 107;
pub const SPICE_MSG_DISPLAY_INVAL_ALL_PALETTES: u16 = 108;
pub const SPICE_MSG_DISPLAY_STREAM_CREATE: u16 = 122;
pub const SPICE_MSG_DISPLAY_STREAM_DATA: u16 = 123;
pub const SPICE_MSG_DISPLAY_STREAM_CLIP: u16 = 124;
pub const SPICE_MSG_DISPLAY_STREAM_DESTROY: u16 = 125;
pub const SPICE_MSG_DISPLAY_STREAM_DESTROY_ALL: u16 = 126;
pub const SPICE_MSG_DISPLAY_DRAW_FILL: u16 = 302;
pub const SPICE_MSG_DISPLAY_DRAW_OPAQUE: u16 = 303;
pub const SPICE_MSG_DISPLAY_DRAW_COPY: u16 = 304;
pub const SPICE_MSG_DISPLAY_DRAW_BLEND: u16 = 305;
pub const SPICE_MSG_DISPLAY_DRAW_BLACKNESS: u16 = 306;
pub const SPICE_MSG_DISPLAY_DRAW_WHITENESS: u16 = 307;
pub const SPICE_MSG_DISPLAY_DRAW_INVERS: u16 = 308;
pub const SPICE_MSG_DISPLAY_DRAW_ROP3: u16 = 309;
pub const SPICE_MSG_DISPLAY_DRAW_STROKE: u16 = 310;
pub const SPICE_MSG_DISPLAY_DRAW_TEXT: u16 = 311;
pub const SPICE_MSG_DISPLAY_DRAW_TRANSPARENT: u16 = 312;
pub const SPICE_MSG_DISPLAY_DRAW_ALPHA_BLEND: u16 = 317;
pub const SPICE_MSG_DISPLAY_SURFACE_CREATE: u16 = 318;
pub const SPICE_MSG_DISPLAY_SURFACE_DESTROY: u16 = 319;
pub const SPICE_MSG_DISPLAY_MONITORS_CONFIG: u16 = 320;
pub const SPICE_MSG_DISPLAY_DRAW_COMPOSITE: u16 = 321;

// Cursor channel messages
pub const SPICE_MSG_CURSOR_INIT: u16 = 101;
pub const SPICE_MSG_CURSOR_RESET: u16 = 102;
pub const SPICE_MSG_CURSOR_SET: u16 = 103;
pub const SPICE_MSG_CURSOR_MOVE: u16 = 104;
pub const SPICE_MSG_CURSOR_HIDE: u16 = 105;
pub const SPICE_MSG_CURSOR_TRAIL: u16 = 106;
pub const SPICE_MSG_CURSOR_INVAL_ONE: u16 = 107;
pub const SPICE_MSG_CURSOR_INVAL_ALL: u16 = 108;

// Display channel messages - additional constants
// Note: SPICE_MSG_DISPLAY_COPY_BITS is defined above as 104
// Note: SPICE_MSG_DISPLAY_INVAL_PALETTE and SPICE_MSG_DISPLAY_INVAL_ALL_PALETTES
// are already defined above in the display messages section

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceMsgMainInit {
    pub session_id: u32,
    pub display_channels_hint: u32,
    pub supported_mouse_modes: u32,
    pub current_mouse_mode: u32,
    pub agent_connected: u32,
    pub agent_tokens: u32,
    pub multi_media_time: u32,
    pub ram_hint: u32,
}

// Channel ID structure
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ChannelId {
    pub type_: u8,
    pub id: u8,
}

// Main channel structures
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceMsgMainMouseMode {
    pub mode: u32,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceMsgMainMultiMediaTime {
    pub time: u32,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceMsgMainAgentConnected {
    pub error_code: u32,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceMsgMainAgentData {
    pub protocol: u32,
    pub type_: u32,
    pub opaque: u64,
    pub size: u32,
    #[br(count = size)]
    pub data: Vec<u8>,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceMsgMainNotify {
    pub time_stamp: u64,
    pub severity: u32,
    pub visibility: u32,
    pub what: u32,
    pub message_len: u32,
    #[br(count = message_len)]
    pub message: Vec<u8>,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceMsgMainAgentTokens {
    pub num_tokens: u32,
}

// Display drawing structures
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceBrush {
    pub brush_type: u8,
    #[br(pad_before = 3)] // 3 bytes padding for alignment
    #[bw(pad_before = 3)]
    pub color: u32, // Only valid for SOLID type
                    // TODO: Pattern support needs to be added
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceClip {
    pub clip_type: u8, // ClipType enum
    #[br(pad_before = 3)] // 3 bytes padding for alignment before u64
    #[bw(pad_before = 3)]
    pub data: SpiceAddress, // Address to clip data (RectList or Path)
}

// SPICE_ADDRESS is a 64-bit offset from the beginning of the message body
pub type SpiceAddress = u64;

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawBase {
    pub surface_id: u32,
    pub box_: SpiceRect, // Note: named box_ to avoid keyword conflict
    pub clip: SpiceClip,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawFill {
    pub base: SpiceDrawBase,
    pub data: SpiceDrawFillData,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawFillData {
    pub brush: SpiceBrush,
    pub rop_descriptor: u16,
    pub mask: SpiceQMask,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceQMask {
    pub flags: u8,
    #[br(pad_before = 3)] // 3 bytes padding for alignment before SpicePoint
    #[bw(pad_before = 3)]
    pub pos: SpicePoint,
    pub bitmap: SpiceAddress, // Address to the mask bitmap
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawCopy {
    pub base: SpiceDrawBase,
    pub data: SpiceDrawCopyData,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawCopyData {
    pub src_image: SpiceAddress,
    pub src_area: SpiceRect,
    pub rop_descriptor: u16,
    pub scale_mode: u8,
    #[br(pad_after = 1)] // Padding for alignment
    #[bw(pad_after = 1)]
    pub mask: SpiceQMask,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawOpaque {
    pub base: SpiceDrawBase,
    pub data: SpiceDrawOpaqueData,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawOpaqueData {
    pub src_image: SpiceAddress,
    pub src_area: SpiceRect,
    pub brush: SpiceBrush,
    pub rop_descriptor: u16,
    pub scale_mode: u8,
    #[br(pad_after = 1)] // Padding for alignment
    #[bw(pad_after = 1)]
    pub mask: SpiceQMask,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawBlend {
    pub base: SpiceDrawBase,
    pub data: SpiceDrawBlendData,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawBlendData {
    pub src_image: SpiceAddress,
    pub src_area: SpiceRect,
    pub rop_descriptor: u16,
    pub scale_mode: u8,
    #[br(pad_after = 1)] // Padding for alignment
    #[bw(pad_after = 1)]
    pub mask: SpiceQMask,
}

// Additional draw operations

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawBlackness {
    pub base: SpiceDrawBase,
    pub mask: SpiceQMask,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawWhiteness {
    pub base: SpiceDrawBase,
    pub mask: SpiceQMask,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawInvers {
    pub base: SpiceDrawBase,
    pub mask: SpiceQMask,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawRop3 {
    pub base: SpiceDrawBase,
    pub src_bitmap: SpiceAddress,
    pub src_area: SpiceRect,
    pub brush: SpiceBrush,
    pub rop3: u8,
    pub scale_mode: u8,
    #[br(pad_after = 2)] // Padding for alignment
    #[bw(pad_after = 2)]
    pub mask: SpiceQMask,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawTransparent {
    pub base: SpiceDrawBase,
    pub src_bitmap: SpiceAddress,
    pub src_area: SpiceRect,
    pub src_color: u32,
    pub true_color: u32,
}

// Complex draw operations - Path and String structures needed for Stroke and Text

// Simplified Path structure - full implementation would need PathSegment support
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpicePath {
    pub segments_size: u32,
    // PathSegment data follows but is variable-length
    // For now, we'll handle this as raw data in the decoder
}

// Simplified LineAttr structure
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceLineAttr {
    pub flags: u8,
    pub join_style: u8,
    pub end_style: u8,
    pub style_nseg: u8,
    pub width: u32, // fixed28_4 format
    pub miter_limit: u32, // fixed28_4 format
                    // Style data follows based on style_nseg
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawStroke {
    pub base: SpiceDrawBase,
    pub path: SpiceAddress, // Address to SpicePath
    pub attr: SpiceLineAttr,
    pub brush: SpiceBrush,
    pub fore_mode: u16,
    pub back_mode: u16,
}

// Simplified String structure - full implementation would need glyph support
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceString {
    pub length: u16,
    pub flags: u16, // string_flags
                    // Glyph data follows based on flags and length
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceDrawText {
    pub base: SpiceDrawBase,
    pub str_: SpiceAddress, // Address to SpiceString (str is keyword)
    pub back_area: SpiceRect,
    pub fore_brush: SpiceBrush,
    pub back_brush: SpiceBrush,
    pub fore_mode: u16,
    pub back_mode: u16,
}

// Stream structures
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceStreamCreate {
    pub id: u32,
    pub flags: u8,
    pub codec_type: u8,
    #[br(pad_before = 2)] // 2 bytes padding for alignment
    #[bw(pad_before = 2)]
    pub stamp: u64,
    pub stream_width: u32,
    pub stream_height: u32,
    pub src_width: u32,
    pub src_height: u32,
    pub dest: SpiceRect,
    pub clip: SpiceClip,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceStreamData {
    pub id: u32,
    pub multi_media_time: u32,
    pub data_size: u32,
    #[br(count = data_size)]
    pub data: Vec<u8>,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceStreamDestroy {
    pub id: u32,
}

// Copy bits structure - for SPICE_MSG_DISPLAY_COPY_BITS
// This message copies a region from one position to another on a surface
// Unlike DrawCopy which copies from an image, this copies from existing surface data
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceCopyBits {
    pub base: SpiceDrawBase, // Base drawing info with dest surface, dest box and clip
    pub src_pos: SpicePoint, // Source position to copy from (on same surface)
}

// Image structures
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceImageDescriptor {
    pub id: u64,
    pub type_: u8, // ImageType enum (BITMAP, QUIC, LZ, GLZ, etc.)
    pub flags: u8, // ImageFlags
    #[br(pad_before = 2)] // 2 bytes padding for alignment
    #[bw(pad_before = 2)]
    pub width: u32,
    pub height: u32,
}

// Image types — values per spice-protocol spice/enums.h (authoritative).
// HAVEN: upstream 0.2.0 had these shifted by one from 100 up (LZ=100,GLZ=101,...),
// which mis-dispatched every compressed image. Corrected to the canonical enum.
pub const SPICE_IMAGE_TYPE_BITMAP: u8 = 0;
pub const SPICE_IMAGE_TYPE_QUIC: u8 = 1;
pub const SPICE_IMAGE_TYPE_LZ_PLT: u8 = 100;
pub const SPICE_IMAGE_TYPE_LZ_RGB: u8 = 101;
pub const SPICE_IMAGE_TYPE_GLZ_RGB: u8 = 102;
pub const SPICE_IMAGE_TYPE_FROM_CACHE: u8 = 103;
pub const SPICE_IMAGE_TYPE_SURFACE: u8 = 104;
pub const SPICE_IMAGE_TYPE_JPEG: u8 = 105;
pub const SPICE_IMAGE_TYPE_FROM_CACHE_LOSSLESS: u8 = 106;
pub const SPICE_IMAGE_TYPE_ZLIB_GLZ_RGB: u8 = 107;
pub const SPICE_IMAGE_TYPE_JPEG_ALPHA: u8 = 108;
pub const SPICE_IMAGE_TYPE_LZ4: u8 = 109;

// Bitmap format
pub const SPICE_BITMAP_FMT_1BIT_LE: u8 = 1;
pub const SPICE_BITMAP_FMT_1BIT_BE: u8 = 2;
pub const SPICE_BITMAP_FMT_4BIT_LE: u8 = 3;
pub const SPICE_BITMAP_FMT_4BIT_BE: u8 = 4;
pub const SPICE_BITMAP_FMT_8BIT: u8 = 5;
pub const SPICE_BITMAP_FMT_16BIT: u8 = 6;
pub const SPICE_BITMAP_FMT_24BIT: u8 = 7;
pub const SPICE_BITMAP_FMT_32BIT: u8 = 8;
pub const SPICE_BITMAP_FMT_RGBA: u8 = 9;
pub const SPICE_BITMAP_FMT_8BIT_A: u8 = 10;

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceImage {
    pub descriptor: SpiceImageDescriptor,
    // Data follows based on descriptor.type_
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceBitmap {
    pub format: u8,
    pub flags: u8,
    #[br(pad_before = 2)] // 2 bytes padding
    #[bw(pad_before = 2)]
    pub x: u32,
    pub y: u32,
    pub stride: u32,
    pub palette: SpiceAddress, // Address to palette data if needed
    pub data: SpiceAddress,    // Address to bitmap data
}

// Surface structures
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceMsgSurfaceCreate {
    pub surface_id: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub flags: u32,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceMsgSurfaceDestroy {
    pub surface_id: u32,
}

// Multi-display support structures
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpiceHead {
    pub id: u32,
    pub surface_id: u32,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub flags: u32,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpiceMonitorsConfig {
    pub count: u16,
    pub max_allowed: u16,
    #[br(count = count)]
    pub heads: Vec<SpiceHead>,
}

// Monitor flags
pub const SPICE_HEAD_FLAGS_NONE: u32 = 0;
pub const SPICE_HEAD_FLAGS_PRIMARY: u32 = 1 << 0;

#[cfg(test)]
mod tests;
