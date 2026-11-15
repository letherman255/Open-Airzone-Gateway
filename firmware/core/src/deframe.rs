//! Cut a byte stream from the bus into frames.
//!
//! Lengths come from the function code where they are known; a read reply announces its length and is
//! waited for even when a shorter prefix has a valid CRC by chance (seen 2026-11-05 22:01). Function 08
//! frames have no reliable length field: they are found by the shortest CRC-valid prefix.

use crate::frame::crc_ok;

/// Silence that ends a frame at 19200 8E1 (3.5 characters, rounded up as in the Modbus spec).
pub const GAP_US: u64 = 1_750;
const MAX_FRAME: usize = 260;

#[derive(Debug, Default)]
pub struct Deframer {
    buf: Vec<u8>,
    /// Length of the reply the caller is waiting for (a read reply), if any.
    expect: Option<usize>,
}

enum Cut {
    Frame(usize),
    Wait,
    Skip,
}

fn candidates(buf: &[u8]) -> Vec<usize> {
    let f = buf[1];
    match f {
        0x03 | 0x04 => vec![8, 5 + buf[2] as usize],
        0x05 | 0x06 => vec![8],
        0x16 => vec![10],
        0x10 if buf.len() >= 7 => vec![8, 9 + buf[6] as usize],
        0x10 => vec![8],
        f if f & 0x80 != 0 => vec![5],
        _ => vec![],
    }
}

fn cut(buf: &[u8], expect: Option<usize>) -> Cut {
    if buf.len() < 4 {
        return Cut::Wait;
    }
    // a read reply of known length: only that length (or a 5-byte exception) can end it
    if let Some(n) = expect {
        if matches!(buf[1], 0x03 | 0x04) && 5 + buf[2] as usize == n {
            if buf.len() < n {
                return Cut::Wait;
            }
            if crc_ok(&buf[..n]) {
                return Cut::Frame(n);
            }
        }
    }
    let cands = candidates(buf);
    for &n in &cands {
        if n <= buf.len() && crc_ok(&buf[..n]) {
            return Cut::Frame(n);
        }
    }
    if matches!(buf[1], 0x03 | 0x04) && cands.len() == 2 && cands[1] > buf.len() {
        return Cut::Wait;
    }
    for n in 4..=buf.len().min(MAX_FRAME) {
        if crc_ok(&buf[..n]) {
            return Cut::Frame(n);
        }
    }
    let unknown_function = cands.is_empty() && buf[1] != 0x08;
    if unknown_function || buf.len() >= MAX_FRAME || (!cands.is_empty() && cands.iter().all(|&n| n <= buf.len())) {
        return Cut::Skip;
    }
    Cut::Wait
}

impl Deframer {
    /// Feed received bytes and return every complete frame. `gap_before_us` is the silence before this
    /// chunk; it is informative only (lengths decide), because captured timestamps are not wire-accurate.
    pub fn push(&mut self, chunk: &[u8], gap_before_us: u64) -> Vec<Vec<u8>> {
        self.feed(chunk, gap_before_us);
        std::iter::from_fn(|| self.next_frame()).collect()
    }

    /// Append received bytes without cutting (use `next_frame` to take frames one by one).
    pub fn feed(&mut self, chunk: &[u8], gap_before_us: u64) {
        let _ = gap_before_us;
        self.buf.extend_from_slice(chunk);
    }

    /// The next complete frame, if any. Clears the expectation set by `expect`.
    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            if self.buf.is_empty() {
                return None;
            }
            match cut(&self.buf, self.expect) {
                Cut::Frame(n) => {
                    self.expect = None;
                    return Some(self.buf.drain(..n).collect());
                }
                Cut::Skip => {
                    self.buf.remove(0);
                }
                Cut::Wait => return None,
            }
        }
    }

    /// Tell the deframer that the next frame should be a read reply of `len` bytes (or `None`).
    /// Without it a reply whose first 8 bytes happen to carry a valid CRC is cut as a request.
    pub fn expect(&mut self, len: Option<usize>) {
        self.expect = len;
    }

    /// Bytes waiting for the rest of a frame.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }
}
