//! Host smoke driver for the SPICE transport — validates the protocol against
//! a real SPICE server (e.g. `qemu ... -spice port=5930,disable-ticketing=on`)
//! without building the Android AAR. Linux/macOS only (host-cli feature).
//!
//!   cargo run --features host-cli --bin spice-cli -- 127.0.0.1 5930 [password]
//!
//! Reports each display-surface update: dimensions, byte length, and the
//! fraction of non-black pixels (a quick "did anything actually render?"
//! signal for the Stage-1 render gate).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use spice_client::{DisplaySurface, SpiceClientShared};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The spice-client decoder emits diagnostics via `tracing`, not `log`;
    // install a subscriber so warn!/debug! actually print. RUST_LOG controls it.
    // (tracing-subscriber also bridges the `log` crate, so env_logger is unneeded.)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let mut args = std::env::args().skip(1);
    let host = args.next().unwrap_or_else(|| "127.0.0.1".to_string());
    let port: u16 = args.next().unwrap_or_else(|| "5930".to_string()).parse()?;
    let password = args.next();

    println!("spice-cli: connecting to {host}:{port} (password: {})", password.is_some());

    let mut client = SpiceClientShared::new(host, port);
    if let Some(pw) = password {
        client.set_password(pw).await;
    }
    let client = Arc::new(client);

    client.connect().await.map_err(|e| format!("connect: {e}"))?;
    println!("spice-cli: connected; wiring frame callback");

    let frames = Arc::new(AtomicU64::new(0));
    let frames_cb = frames.clone();
    // Save the LATEST frame each update so post-activity (e.g. GLZ incremental)
    // content is captured, not just the initial full paint.
    client
        .set_display_update_callback(0, move |surface: &DisplaySurface| {
            let n = frames_cb.fetch_add(1, Ordering::Relaxed) + 1;
            report_surface(n, surface);
            save_ppm("/tmp/spice-frame.ppm", surface);
        })
        .await
        .map_err(|e| format!("set_display_update_callback: {e}"))?;

    client.start_event_loop().await.map_err(|e| format!("start_event_loop: {e}"))?;

    let observe_secs: u64 = std::env::var("SPICE_OBSERVE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    println!("spice-cli: event loop started; observing for {observe_secs}s");

    // Optionally wiggle the mouse to provoke incremental repaints (GLZ uses the
    // global dictionary across updates, so it only appears once the screen
    // changes within a live connection). SPICE_WIGGLE=1 to enable.
    if std::env::var_os("SPICE_WIGGLE").is_some() {
        let mover = client.clone();
        tokio::spawn(async move {
            let pts = [
                (500, 300),
                (520, 350),
                (540, 400),
                (560, 450),
                (560, 500),
                (540, 256),
                (520, 304),
                (500, 352),
            ];
            let mut first = true;
            for _ in 0..200 {
                for (x, y) in pts {
                    let r = mover.send_mouse_motion(0, x, y).await;
                    if first {
                        first = false;
                        println!("spice-cli: first mouse_motion result: {r:?}");
                    }
                    tokio::time::sleep(Duration::from_millis(120)).await;
                }
            }
        });
    }

    tokio::time::sleep(Duration::from_secs(observe_secs)).await;

    let total = frames.load(Ordering::Relaxed);
    println!("spice-cli: observed {total} display update(s)");
    client.disconnect().await;

    if total == 0 {
        // Also probe the polled accessor in case the callback path is the gap.
        if let Some(surface) = client.get_display_surface(0).await {
            println!("spice-cli: polled primary surface present:");
            report_surface(0, &surface);
        } else {
            return Err("RENDER GATE FAILED: no frames and no primary surface".into());
        }
    }
    Ok(())
}

/// Write an RGBA surface to a binary PPM (P6, RGB) for visual inspection.
fn save_ppm(path: &str, s: &DisplaySurface) {
    let (w, h) = (s.width as usize, s.height as usize);
    let mut out = Vec::with_capacity(w * h * 3 + 32);
    out.extend_from_slice(format!("P6\n{} {}\n255\n", w, h).as_bytes());
    for px in s.data.chunks_exact(4) {
        out.push(px[0]);
        out.push(px[1]);
        out.push(px[2]);
    }
    match std::fs::write(path, &out) {
        Ok(_) => println!("spice-cli: wrote {} ({}x{})", path, w, h),
        Err(e) => eprintln!("spice-cli: failed to write {}: {}", path, e),
    }
}

fn report_surface(n: u64, s: &DisplaySurface) {
    let bytes = s.data.len();
    let expected = s.width as usize * s.height as usize * 4;
    let non_black = s
        .data
        .chunks_exact(4)
        .filter(|px| px[0] != 0 || px[1] != 0 || px[2] != 0)
        .count();
    let total_px = (bytes / 4).max(1);
    let pct = non_black * 100 / total_px;
    println!(
        "  frame #{n}: {}x{} format={} bytes={} (expected {}) non-black={}%",
        s.width, s.height, s.format, bytes, expected, pct
    );
}
