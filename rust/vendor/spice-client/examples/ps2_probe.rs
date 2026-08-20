//! Probe for #549: connect to a SPICE server fronting a PS/2-only guest and
//! report which pointer mode the client ends up holding, and what it sends.
//!
//! Run against `qemu-system-x86_64 -spice port=5930,disable-ticketing=on`
//! with NO usb-tablet device, then watch the log lines:
//!   "Server mouse mode: N"   -- the mode carried in MSG_MAIN_INIT
//!   "Mouse mode changed: .." -- the standalone MOUSE_MODE message, if any

use spice_client::SpiceClientShared;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let host = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "5930".into())
        .parse()?;

    let client = SpiceClientShared::new(host, port);
    client.connect().await?;
    client.start_event_loop().await?;

    // Give the channels time to settle, then send pointer traffic.
    tokio::time::sleep(Duration::from_secs(2)).await;
    for i in 0..5i32 {
        client.send_mouse_motion(0, 100 + i * 20, 100 + i * 10).await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    tokio::time::sleep(Duration::from_secs(1)).await;
    println!("PROBE DONE");
    Ok(())
}
