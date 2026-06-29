//! Inputs channel implementation for keyboard and mouse events

use crate::channels::{Channel, ChannelConnection, InputEvent, KeyCode, MouseButton};
use crate::error::{Result, SpiceError};
use crate::protocol::*;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Mouse operation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseMode {
    Server,
    Client,
}

/// Inputs channel for sending keyboard and mouse events
pub struct InputsChannel {
    pub(crate) connection: ChannelConnection,
    mouse_mode: MouseMode,
    modifiers: KeyModifiers,
    /// Outgoing input messages are queued here and drained by `run()` — the sole
    /// reader/writer of the socket — so external senders never contend the
    /// run-loop's channel lock (which previously deadlocked every input).
    outgoing_tx: mpsc::UnboundedSender<(u16, Vec<u8>)>,
    outgoing_rx: Option<mpsc::UnboundedReceiver<(u16, Vec<u8>)>>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl InputsChannel {
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
            ChannelConnection::new(host, port, ChannelType::Inputs, channel_id).await?;
        if let Some(conn_id) = connection_id {
            connection.set_connection_id(conn_id);
        }
        connection.handshake().await?;

        Ok(Self::build(connection))
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
            ChannelType::Inputs,
            channel_id,
            auth_token,
        )
        .await?;
        connection.handshake().await?;

        Ok(Self::build(connection))
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
            ChannelType::Inputs,
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

        Ok(Self::build(connection))
    }

    /// Builds an inputs channel around an established connection, wiring the
    /// outgoing queue that `run()` drains.
    fn build(connection: ChannelConnection) -> Self {
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel();
        Self {
            connection,
            mouse_mode: MouseMode::Server,
            modifiers: KeyModifiers::default(),
            outgoing_tx,
            outgoing_rx: Some(outgoing_rx),
        }
    }

    /// Clonable handle to enqueue outgoing input messages without touching the
    /// run-loop's channel lock.
    pub(crate) fn outgoing_sender(&self) -> mpsc::UnboundedSender<(u16, Vec<u8>)> {
        self.outgoing_tx.clone()
    }

    fn enqueue(&self, msg: (u16, Vec<u8>)) -> Result<()> {
        self.outgoing_tx
            .send(msg)
            .map_err(|_| SpiceError::Protocol("inputs channel closed".into()))
    }

    pub async fn initialize(&mut self) -> Result<()> {
        info!("Inputs channel {} initialized", self.connection.channel_id);
        Ok(())
    }

    pub fn get_mouse_mode(&self) -> MouseMode {
        self.mouse_mode
    }

    pub fn get_modifiers(&self) -> KeyModifiers {
        self.modifiers
    }

    /// Sends an input event to the server
    pub async fn send_event(&mut self, event: InputEvent) -> Result<()> {
        match event {
            InputEvent::KeyDown(key) => {
                let scancode = key_to_scancode(key);
                self.update_modifiers(&key, true);
                self.send_key_down(scancode).await?
            }
            InputEvent::KeyUp(key) => {
                let scancode = key_to_scancode(key);
                self.update_modifiers(&key, false);
                self.send_key_up(scancode).await?
            }
            InputEvent::MouseMove { x, y } => self.send_mouse_motion(x, y).await?,
            InputEvent::MouseButton { button, pressed } => {
                self.send_mouse_button(button, pressed).await?
            }
        }
        Ok(())
    }

    /// Queues a key-down event (scancode); the run loop sends it.
    pub async fn send_key_down(&mut self, scancode: u32) -> Result<()> {
        self.enqueue(encode_key(scancode, true))
    }

    /// Queues a key-up event (scancode).
    pub async fn send_key_up(&mut self, scancode: u32) -> Result<()> {
        self.enqueue(encode_key(scancode, false))
    }

    /// Queues an absolute pointer position (client/tablet mouse mode).
    pub async fn send_mouse_motion(&mut self, x: i32, y: i32) -> Result<()> {
        self.enqueue(encode_mouse_position(x, y, 0))
    }

    /// Queues a mouse button press/release.
    pub async fn send_mouse_button(&mut self, button: MouseButton, pressed: bool) -> Result<()> {
        self.enqueue(encode_mouse_button(button, pressed))
    }

    /// Updates modifier keys state
    fn update_modifiers(&mut self, key: &KeyCode, pressed: bool) {
        match key {
            KeyCode::Other(scancode) => {
                match *scancode {
                    0x2A | 0x36 => self.modifiers.shift = pressed, // Left/Right Shift
                    0x1D | 0x9D => self.modifiers.ctrl = pressed,  // Left/Right Ctrl
                    0x38 | 0xB8 => self.modifiers.alt = pressed,   // Left/Right Alt
                    0x5B | 0x5C => self.modifiers.meta = pressed,  // Left/Right Meta
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Runs the inputs channel: the sole owner of the socket. It both reads
    /// server messages and drains the outgoing queue, so external senders never
    /// need the channel lock this loop holds.
    pub async fn run(&mut self) -> Result<()> {
        let mut outgoing = self
            .outgoing_rx
            .take()
            .ok_or_else(|| SpiceError::Protocol("inputs run loop already started".into()))?;
        loop {
            tokio::select! {
                queued = outgoing.recv() => match queued {
                    Some((msg_type, data)) => {
                        self.connection.send_message(msg_type, &data).await?;
                    }
                    None => return Ok(()), // all senders dropped → shutting down
                },
                incoming = self.connection.read_message() => {
                    let (header, data) = incoming?;
                    self.handle_message(&header, &data).await?;
                }
            }
        }
    }

    async fn handle_init_message(&mut self, data: &[u8]) -> Result<()> {
        if data.len() >= 2 {
            let modifiers = u16::from_le_bytes([data[0], data[1]]);
            info!("Inputs init - modifiers: 0x{:04X}", modifiers);

            // Update modifier state based on init message
            self.modifiers.shift = (modifiers & SPICE_KEYBOARD_MODIFIER_SHIFT) != 0;
            self.modifiers.ctrl = (modifiers & SPICE_KEYBOARD_MODIFIER_CTRL) != 0;
            self.modifiers.alt = (modifiers & SPICE_KEYBOARD_MODIFIER_ALT) != 0;
        }
        Ok(())
    }

    async fn handle_modifiers_message(&mut self, data: &[u8]) -> Result<()> {
        if data.len() >= 2 {
            let modifiers = u16::from_le_bytes([data[0], data[1]]);
            debug!("Modifiers update: 0x{:04X}", modifiers);

            self.modifiers.shift = (modifiers & SPICE_KEYBOARD_MODIFIER_SHIFT) != 0;
            self.modifiers.ctrl = (modifiers & SPICE_KEYBOARD_MODIFIER_CTRL) != 0;
            self.modifiers.alt = (modifiers & SPICE_KEYBOARD_MODIFIER_ALT) != 0;
        }
        Ok(())
    }
}

impl Channel for InputsChannel {
    async fn handle_message(&mut self, header: &SpiceDataHeader, data: &[u8]) -> Result<()> {
        match header.msg_type {
            SPICE_MSG_INPUTS_INIT => {
                debug!("Received inputs init");
                self.handle_init_message(data).await?;
            }
            SPICE_MSG_INPUTS_KEY_MODIFIERS => {
                debug!("Received key modifiers");
                self.handle_modifiers_message(data).await?;
            }
            x if x == SPICE_MSG_SET_ACK => {
                debug!("Received SET_ACK message in inputs channel");
                if data.len() >= 4 {
                    let generation = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                    debug!("Inputs channel: SET_ACK generation: {}", generation);
                    // Send ACK_SYNC response
                    let ack_data = generation.to_le_bytes();
                    self.connection
                        .send_message(SPICE_MSGC_ACK_SYNC, &ack_data)
                        .await?;
                }
            }
            x if x == SPICE_MSG_PING => {
                debug!("Received PING message in inputs channel");
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
                            0 => info!("Inputs server info: {}", message),
                            1 => warn!("Inputs server warning: {}", message),
                            2 => error!("Inputs server error: {}", message),
                            _ => debug!("Inputs server notification: {}", message),
                        }
                    }
                }
            }
            x if x == SPICE_MSG_DISCONNECTING => {
                info!("Inputs channel: Server is disconnecting");
                return Err(SpiceError::ConnectionClosed);
            }
            x if x == SPICE_MSG_MIGRATE => {
                warn!("Inputs channel: Migration not implemented");
            }
            x if x == SPICE_MSG_MIGRATE_DATA => {
                warn!("Inputs channel: Migration data not implemented");
            }
            x if x == SPICE_MSG_WAIT_FOR_CHANNELS => {
                warn!("Inputs channel: Wait for channels not implemented");
            }
            _ => {
                warn!("Unknown inputs message type: {}", header.msg_type);
            }
        }

        Ok(())
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Inputs
    }
}

// Server -> client inputs messages
pub const SPICE_MSG_INPUTS_INIT: u16 = 101;
pub const SPICE_MSG_INPUTS_KEY_MODIFIERS: u16 = 102;

// Client -> server inputs messages (SpiceMsgcInputs). These differ from the
// server->client numbers above — the channel direction disambiguates the wire.
pub const SPICE_MSGC_INPUTS_KEY_DOWN: u16 = 101;
pub const SPICE_MSGC_INPUTS_KEY_UP: u16 = 102;
pub const SPICE_MSGC_INPUTS_MOUSE_MOTION: u16 = 111;
pub const SPICE_MSGC_INPUTS_MOUSE_POSITION: u16 = 112;
pub const SPICE_MSGC_INPUTS_MOUSE_PRESS: u16 = 113;
pub const SPICE_MSGC_INPUTS_MOUSE_RELEASE: u16 = 114;

// Mouse button numbers (SpiceMouseButton), used in PRESS/RELEASE.
const SPICE_MOUSE_BUTTON_LEFT: u8 = 1;
const SPICE_MOUSE_BUTTON_MIDDLE: u8 = 2;
const SPICE_MOUSE_BUTTON_RIGHT: u8 = 3;
const SPICE_MOUSE_BUTTON_UP: u8 = 4;
const SPICE_MOUSE_BUTTON_DOWN: u8 = 5;

// Mouse button-state mask bits (buttons currently held).
const SPICE_MOUSE_BUTTON_MASK_LEFT: u16 = 1 << 0;
const SPICE_MOUSE_BUTTON_MASK_MIDDLE: u16 = 1 << 1;
const SPICE_MOUSE_BUTTON_MASK_RIGHT: u16 = 1 << 2;
const SPICE_MOUSE_BUTTON_MASK_UP: u16 = 1 << 3;
const SPICE_MOUSE_BUTTON_MASK_DOWN: u16 = 1 << 4;

fn button_number(b: MouseButton) -> u8 {
    match b {
        MouseButton::Left => SPICE_MOUSE_BUTTON_LEFT,
        MouseButton::Middle => SPICE_MOUSE_BUTTON_MIDDLE,
        MouseButton::Right => SPICE_MOUSE_BUTTON_RIGHT,
        MouseButton::WheelUp => SPICE_MOUSE_BUTTON_UP,
        MouseButton::WheelDown => SPICE_MOUSE_BUTTON_DOWN,
    }
}

fn button_mask(b: MouseButton) -> u16 {
    match b {
        MouseButton::Left => SPICE_MOUSE_BUTTON_MASK_LEFT,
        MouseButton::Middle => SPICE_MOUSE_BUTTON_MASK_MIDDLE,
        MouseButton::Right => SPICE_MOUSE_BUTTON_MASK_RIGHT,
        MouseButton::WheelUp => SPICE_MOUSE_BUTTON_MASK_UP,
        MouseButton::WheelDown => SPICE_MOUSE_BUTTON_MASK_DOWN,
    }
}

/// Encodes a key down/up message: `{ u32 scancode }`.
pub(crate) fn encode_key(scancode: u32, pressed: bool) -> (u16, Vec<u8>) {
    let ty = if pressed {
        SPICE_MSGC_INPUTS_KEY_DOWN
    } else {
        SPICE_MSGC_INPUTS_KEY_UP
    };
    (ty, scancode.to_le_bytes().to_vec())
}

/// Encodes an absolute pointer position (client/tablet mouse mode):
/// `{ u32 x; u32 y; u16 buttons_state; u8 display_id }`.
pub(crate) fn encode_mouse_position(x: i32, y: i32, buttons_state: u16) -> (u16, Vec<u8>) {
    let mut data = Vec::with_capacity(11);
    data.extend_from_slice(&(x.max(0) as u32).to_le_bytes());
    data.extend_from_slice(&(y.max(0) as u32).to_le_bytes());
    data.extend_from_slice(&buttons_state.to_le_bytes());
    data.push(0u8); // display_id
    (SPICE_MSGC_INPUTS_MOUSE_POSITION, data)
}

/// Encodes a mouse press/release: `{ u8 button; u16 buttons_state }`.
pub(crate) fn encode_mouse_button(button: MouseButton, pressed: bool) -> (u16, Vec<u8>) {
    let ty = if pressed {
        SPICE_MSGC_INPUTS_MOUSE_PRESS
    } else {
        SPICE_MSGC_INPUTS_MOUSE_RELEASE
    };
    // ponytail: single-button click model — pressed reports just this button in
    // the state mask, released reports none. Button chords aren't tracked.
    let buttons_state = if pressed { button_mask(button) } else { 0 };
    let mut data = Vec::with_capacity(3);
    data.push(button_number(button));
    data.extend_from_slice(&buttons_state.to_le_bytes());
    (ty, data)
}

// Keyboard modifier masks
pub const SPICE_KEYBOARD_MODIFIER_SHIFT: u16 = 1 << 0;
pub const SPICE_KEYBOARD_MODIFIER_CTRL: u16 = 1 << 1;
pub const SPICE_KEYBOARD_MODIFIER_ALT: u16 = 1 << 2;

/// Converts a KeyCode to a PC scancode
fn key_to_scancode(key: KeyCode) -> u32 {
    match key {
        KeyCode::Escape => 0x01,
        KeyCode::Enter => 0x1C,
        KeyCode::Space => 0x39,
        KeyCode::Tab => 0x0F,
        KeyCode::Backspace => 0x0E,
        KeyCode::Function(n) => match n {
            1 => 0x3B,
            2 => 0x3C,
            3 => 0x3D,
            4 => 0x3E,
            5 => 0x3F,
            6 => 0x40,
            7 => 0x41,
            8 => 0x42,
            9 => 0x43,
            10 => 0x44,
            11 => 0x57,
            12 => 0x58,
            _ => 0x00,
        },
        KeyCode::ArrowUp => 0x48,
        KeyCode::ArrowDown => 0x50,
        KeyCode::ArrowLeft => 0x4B,
        KeyCode::ArrowRight => 0x4D,
        KeyCode::Char(c) => char_to_scancode(c),
        KeyCode::Other(scancode) => scancode,
    }
}

/// Converts a character to a PC scancode
fn char_to_scancode(c: char) -> u32 {
    match c.to_ascii_uppercase() {
        'A' => 0x1E,
        'B' => 0x30,
        'C' => 0x2E,
        'D' => 0x20,
        'E' => 0x12,
        'F' => 0x21,
        'G' => 0x22,
        'H' => 0x23,
        'I' => 0x17,
        'J' => 0x24,
        'K' => 0x25,
        'L' => 0x26,
        'M' => 0x32,
        'N' => 0x31,
        'O' => 0x18,
        'P' => 0x19,
        'Q' => 0x10,
        'R' => 0x13,
        'S' => 0x1F,
        'T' => 0x14,
        'U' => 0x16,
        'V' => 0x2F,
        'W' => 0x11,
        'X' => 0x2D,
        'Y' => 0x15,
        'Z' => 0x2C,
        '1' => 0x02,
        '2' => 0x03,
        '3' => 0x04,
        '4' => 0x05,
        '5' => 0x06,
        '6' => 0x07,
        '7' => 0x08,
        '8' => 0x09,
        '9' => 0x0A,
        '0' => 0x0B,
        '-' => 0x0C,
        '=' => 0x0D,
        '[' => 0x1A,
        ']' => 0x1B,
        ';' => 0x27,
        '\'' => 0x28,
        ',' => 0x33,
        '.' => 0x34,
        '/' => 0x35,
        '\\' => 0x2B,
        '`' => 0x29,
        _ => 0x00, // Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_to_scancode() {
        assert_eq!(key_to_scancode(KeyCode::Escape), 0x01);
        assert_eq!(key_to_scancode(KeyCode::Enter), 0x1C);
        assert_eq!(key_to_scancode(KeyCode::Space), 0x39);
        assert_eq!(key_to_scancode(KeyCode::Char('A')), 0x1E);
        assert_eq!(key_to_scancode(KeyCode::Char('a')), 0x1E);
        assert_eq!(key_to_scancode(KeyCode::Other(0x42)), 0x42);
    }

    #[test]
    fn test_modifiers() {
        let mut modifiers = KeyModifiers::default();
        assert!(!modifiers.shift);
        assert!(!modifiers.ctrl);
        assert!(!modifiers.alt);
        assert!(!modifiers.meta);

        modifiers.shift = true;
        modifiers.ctrl = true;
        assert!(modifiers.shift);
        assert!(modifiers.ctrl);
    }

    #[test]
    fn test_encode_key_uses_msgc_numbers() {
        let (ty, data) = encode_key(0x1E, true);
        assert_eq!(ty, SPICE_MSGC_INPUTS_KEY_DOWN); // 101, not the old 103
        assert_eq!(data, 0x1Eu32.to_le_bytes().to_vec());
        assert_eq!(encode_key(0x1E, false).0, SPICE_MSGC_INPUTS_KEY_UP); // 102
    }

    #[test]
    fn test_encode_mouse_position_layout() {
        let (ty, data) = encode_mouse_position(100, 200, 0);
        assert_eq!(ty, SPICE_MSGC_INPUTS_MOUSE_POSITION); // 112
        assert_eq!(data.len(), 11);
        assert_eq!(&data[0..4], &100u32.to_le_bytes());
        assert_eq!(&data[4..8], &200u32.to_le_bytes());
        assert_eq!(&data[8..10], &0u16.to_le_bytes());
        assert_eq!(data[10], 0); // display_id
        // Negative coords clamp to 0 (avoids u32 wraparound).
        assert_eq!(&encode_mouse_position(-5, 7, 0).1[0..4], &0u32.to_le_bytes());
    }

    #[test]
    fn test_encode_mouse_button_layout() {
        // press left → button number 1, state mask LEFT(0x01)
        let (ty, data) = encode_mouse_button(MouseButton::Left, true);
        assert_eq!(ty, SPICE_MSGC_INPUTS_MOUSE_PRESS); // 113
        assert_eq!(data, vec![1u8, 0x01, 0x00]);
        // release right → button number 3, no buttons held
        let (ty2, data2) = encode_mouse_button(MouseButton::Right, false);
        assert_eq!(ty2, SPICE_MSGC_INPUTS_MOUSE_RELEASE); // 114
        assert_eq!(data2, vec![3u8, 0x00, 0x00]);
    }
}
