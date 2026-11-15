//! Minimal WebSocket (RFC 6455) framing for the host dev server: server text frames out, masked client
//! frames in. The ESP uses its HTTP server's WebSocket support instead.

/// Unmasked text frame (server → client).
pub fn text_frame(s: &str) -> Vec<u8> {
    let p = s.as_bytes();
    let mut f = vec![0x81];
    match p.len() {
        n if n < 126 => f.push(n as u8),
        n if n <= 0xFFFF => {
            f.push(126);
            f.extend((n as u16).to_be_bytes());
        }
        n => {
            f.push(127);
            f.extend((n as u64).to_be_bytes());
        }
    }
    f.extend_from_slice(p);
    f
}

/// One client frame from the start of `buf`: (opcode, unmasked payload, bytes used); `None` if incomplete.
pub fn client_frame(buf: &[u8]) -> Option<(u8, Vec<u8>, usize)> {
    if buf.len() < 2 {
        return None;
    }
    let op = buf[0] & 0x0F;
    let masked = buf[1] & 0x80 != 0;
    let (len, mut i) = match buf[1] & 0x7F {
        126 => (u16::from_be_bytes([*buf.get(2)?, *buf.get(3)?]) as usize, 4),
        127 => (u64::from_be_bytes(buf.get(2..10)?.try_into().ok()?) as usize, 10),
        n => (n as usize, 2),
    };
    if len > 64 * 1024 {
        return Some((0x8, Vec::new(), buf.len()));
    }
    let key = if masked {
        let k = buf.get(i..i + 4)?.to_vec();
        i += 4;
        k
    } else {
        vec![0; 4]
    };
    let data = buf.get(i..i + len)?;
    Some((op, data.iter().enumerate().map(|(j, b)| b ^ key[j % 4]).collect(), i + len))
}

/// Where a WebSocket client is in the event / frame streams and the register changes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WsCursor {
    pub events: u64,
    pub frames: u64,
    pub ms: u64,
}

/// Messages for one WebSocket tick: `values` always, `events` when new ones exist, and in developer mode
/// `frames` (new bus frames) and `registers` (registers changed since the last tick).
pub fn live_messages(g: &mut crate::gateway::Gateway, c: &mut WsCursor, values: String, now_ms: u64) -> Vec<String> {
    use crate::ha::js;
    let mut out = vec![values];
    let ev: Vec<&str> = g.events.iter().filter(|(s, _)| *s > c.events).map(|(_, j)| j.as_str()).collect();
    if !ev.is_empty() {
        out.push(format!(r#"{{"topic":"events","events":[{}]}}"#, ev.join(",")));
    }
    c.events = g.events.back().map(|(s, _)| *s).unwrap_or(c.events);
    if g.dev_mode {
        let fr: Vec<String> = g.frames.iter().filter(|(s, _)| *s > c.frames).map(|(_, l)| js(l)).collect();
        if !fr.is_empty() {
            out.push(format!(r#"{{"topic":"frames","frames":[{}]}}"#, fr.join(",")));
        }
        let regs = g.registry.changed_since(c.ms);
        if !regs.is_empty() {
            let items: Vec<String> = regs.iter().map(|(a, r, v)| format!("[{a},{r},{v}]")).collect();
            out.push(format!(r#"{{"topic":"registers","registers":[{}]}}"#, items.join(",")));
        }
    }
    c.frames = g.frames.back().map(|(s, _)| *s).unwrap_or(c.frames);
    c.ms = now_ms + 1;
    out
}
