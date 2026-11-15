//! UART receive loop. Receive only: the driver is created without a TX pin.

use std::sync::atomic::Ordering;
use std::sync::mpsc::{SyncSender, TrySendError};

use esp_idf_svc::hal::uart::UartRxDriver;
use esp_idf_svc::sys;

use crate::stream::{self, now_us, BYTES, FRAMES, LISTEN_DROPPED};

/// A chunk this long was probably cut by the FIFO-full threshold, not by an idle gap.
const FIFO_SPLIT_LEN: usize = 120;

pub struct Chunk {
    pub t_us: i64,
    pub bytes: Vec<u8>,
}

fn emit(to_stream: &SyncSender<String>, to_listener: &SyncSender<Chunk>, t_us: i64, frame: &[u8]) {
    FRAMES.fetch_add(1, Ordering::Relaxed);
    BYTES.fetch_add(frame.len() as u32, Ordering::Relaxed);
    stream::send(to_stream, stream::chunk_line(t_us, frame));
    if let Err(TrySendError::Full(_)) = to_listener.try_send(Chunk { t_us, bytes: frame.to_vec() }) {
        LISTEN_DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn run(uart: UartRxDriver<'static>, to_stream: SyncSender<String>, to_listener: SyncSender<Chunk>) {
    let mut buf = [0u8; 256];
    let mut frame: Vec<u8> = Vec::with_capacity(512);
    let mut t_frame = 0i64;
    loop {
        let n = match uart.read(&mut buf, 2) {
            Ok(n) => n,
            Err(e) if e.code() == sys::ESP_ERR_TIMEOUT => 0,
            Err(e) => {
                log::warn!("uart read error: {e}");
                0
            }
        };
        if n == 0 {
            if !frame.is_empty() {
                emit(&to_stream, &to_listener, t_frame, &frame);
                frame.clear();
            }
            continue;
        }
        if frame.is_empty() {
            t_frame = now_us();
        }
        frame.extend_from_slice(&buf[..n]);
        if n < FIFO_SPLIT_LEN || frame.len() >= 1024 {
            emit(&to_stream, &to_listener, t_frame, &frame);
            frame.clear();
        }
    }
}
