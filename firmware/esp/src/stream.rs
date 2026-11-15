//! Raw frame stream on TCP 7000, same line format as the sniffer, so tools/bus_receiver.py keeps working:
//!   `<t_us> <len> <hex bytes>` per received chunk, `# …` for comments and the 5 s heartbeat.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::Duration;

pub const STREAM_PORT: u16 = 7000;
pub static FRAMES: AtomicU32 = AtomicU32::new(0);
pub static BYTES: AtomicU32 = AtomicU32::new(0);
/// Lines lost because no stream client keeps up (normal when nobody is connected).
pub static STREAM_DROPPED: AtomicU32 = AtomicU32::new(0);
/// Chunks the listen-mode decoder could not take (should stay 0).
pub static LISTEN_DROPPED: AtomicU32 = AtomicU32::new(0);

pub fn now_us() -> i64 {
    unsafe { esp_idf_svc::sys::esp_timer_get_time() }
}

pub fn chunk_line(t_us: i64, frame: &[u8]) -> String {
    let mut line = String::with_capacity(24 + frame.len() * 3);
    line.push_str(&format!("{t_us} {}", frame.len()));
    for b in frame {
        line.push_str(&format!(" {b:02X}"));
    }
    line.push('\n');
    line
}

/// Last frames for the WebSocket `frames` topic (taken by the WebSocket thread).
pub static RECENT: std::sync::Mutex<std::collections::VecDeque<String>> = std::sync::Mutex::new(std::collections::VecDeque::new());

/// A stream client is connected. Without one, lines are not queued at all: a full queue of frame lines
/// (up to ~500 bytes each) held tens of KB of heap for nobody (2026-11-06).
pub static CLIENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// A WebSocket client is connected: keep the last frames for its `frames` topic (else nothing is kept).
pub static RECENT_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Queue a line for the stream client; never blocks (drops are counted).
pub fn send(tx: &SyncSender<String>, line: String) {
    if !line.starts_with('#') && RECENT_ON.load(Ordering::Relaxed) {
        if let Ok(mut r) = RECENT.try_lock() {
            if r.len() >= 30 {
                r.pop_front();
            }
            r.push_back(line.trim_end().to_string());
        }
    }
    if !CLIENT.load(Ordering::Relaxed) {
        return;
    }
    if let Err(TrySendError::Full(_)) = tx.try_send(line) {
        STREAM_DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn server(rx: Receiver<String>, banner: String) {
    let listener = loop {
        match TcpListener::bind(("0.0.0.0", STREAM_PORT)) {
            Ok(l) => break l,
            Err(e) => {
                log::warn!("stream bind failed: {e}");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    };
    log::info!("frame stream on port {STREAM_PORT}");
    for conn in listener.incoming() {
        if let Ok(s) = conn {
            CLIENT.store(true, Ordering::Relaxed);
            if let Err(e) = serve(s, &rx, &banner) {
                log::info!("stream client gone: {e}");
            }
            CLIENT.store(false, Ordering::Relaxed);
            while rx.try_recv().is_ok() {}
        }
    }
}

fn serve(mut s: TcpStream, rx: &Receiver<String>, banner: &str) -> std::io::Result<()> {
    s.set_nodelay(true)?;
    s.set_write_timeout(Some(Duration::from_secs(5)))?;
    // lines queued while nobody listened are stale
    while rx.try_recv().is_ok() {}
    writeln!(s, "# {banner} t_us={}", now_us())?;
    let mut last_alive = std::time::Instant::now();
    loop {
        if last_alive.elapsed() >= Duration::from_secs(5) {
            last_alive = std::time::Instant::now();
            writeln!(
                s,
                "# alive t_us={} frames={} bytes={} dropped={} listen_dropped={}",
                now_us(),
                FRAMES.load(Ordering::Relaxed),
                BYTES.load(Ordering::Relaxed),
                STREAM_DROPPED.load(Ordering::Relaxed),
                LISTEN_DROPPED.load(Ordering::Relaxed)
            )?;
        }
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(line) => s.write_all(line.as_bytes())?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}
