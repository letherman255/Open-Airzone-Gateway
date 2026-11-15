//! Listen mode: decode the traffic of another master (the stock webserver) without ever sending.
//! Frames are paired request → reply; read replies fill the registry, accepted writes are reported
//! with the catalogue parameters they changed.

use crate::catalogue::Kind;
use crate::deframe::Deframer;
use crate::frame::{parse_response, Request, Response};
use crate::model::{decode, mask_of, params_at, Value};
use crate::registry::{Change, Registry};

#[derive(Debug, Clone, PartialEq)]
pub enum ListenEvent {
    /// A write the central accepted, with the parameters it touched: (id, zone or outlet number, new value).
    Write { request: Request, params: Vec<(&'static str, Option<u8>, Value)> },
    Changes(Vec<Change>),
    /// A frame that is neither a request nor the reply to the pending request.
    Unpaired(Vec<u8>),
}

#[derive(Debug, Default)]
pub struct Listener {
    deframer: Deframer,
    pending: Option<(Request, Vec<u8>)>,
    reg: Registry,
}

fn is_reply(req: &Request, raw: &[u8], frame: &[u8]) -> Option<Response> {
    let r = parse_response(req, frame).ok()?;
    match (&r, req) {
        // 06 and 08 replies are echoes; a 16 ack carries the request's AND mask
        (Response::WriteSingle { .. } | Response::Diag { .. }, _) if frame != raw => None,
        (Response::MaskAck { reg, and }, Request::MaskWrite { reg: r0, and: a0, .. }) if reg != r0 || and != a0 => None,
        _ => Some(r),
    }
}

impl Listener {
    pub fn registry(&self) -> &Registry {
        &self.reg
    }

    pub fn feed(&mut self, chunk: &[u8], gap_before_us: u64, now_ms: u64) -> Vec<ListenEvent> {
        let mut out = Vec::new();
        self.deframer.feed(chunk, gap_before_us);
        while let Some(frame) = self.deframer.next_frame() {
            if let Some((req, raw)) = self.pending.take() {
                if let Some(resp) = is_reply(&req, &raw, &frame) {
                    self.on_reply(req, resp, now_ms, &mut out);
                    continue;
                }
            }
            match Request::parse(&frame) {
                Ok(req) => {
                    // a read reply announces its length: make the deframer wait for exactly that
                    if let Request::Read { count, .. } = req {
                        self.deframer.expect(Some(5 + 2 * count as usize));
                    }
                    self.pending = Some((req, frame));
                }
                Err(_) => out.push(ListenEvent::Unpaired(frame)),
            }
        }
        out
    }

    fn on_reply(&mut self, req: Request, resp: Response, now_ms: u64, out: &mut Vec<ListenEvent>) {
        match (&req, resp) {
            (Request::Read { addr, reg, .. }, Response::Registers(v)) => {
                let ch = self.reg.update(*addr, *reg, &v, now_ms);
                if !ch.is_empty() {
                    out.push(ListenEvent::Changes(ch));
                }
            }
            (Request::WriteSingle { addr, reg, value }, Response::WriteSingle { .. }) => {
                let params = self.written(*addr, *reg, *value, 0xFFFF);
                self.reg.update(*addr, *reg, &[*value], now_ms);
                out.push(ListenEvent::Write { request: req.clone(), params });
            }
            (Request::MaskWrite { addr, reg, and, or }, Response::MaskAck { .. }) => {
                let old = self.reg.get(*addr, *reg).unwrap_or(0);
                let new = (old & and) | (or & !and);
                let params = self.written(*addr, *reg, new, !and);
                self.reg.update(*addr, *reg, &[new], now_ms);
                out.push(ListenEvent::Write { request: req.clone(), params });
            }
            _ => {}
        }
    }

    /// Parameters whose field overlaps `changed_bits` of register `reg`, decoded from the new value.
    fn written(&self, addr: u8, reg: u16, new: u16, changed_bits: u16) -> Vec<(&'static str, Option<u8>, Value)> {
        params_at(reg)
            .into_iter()
            .filter(|(p, _)| p.kind == Kind::Text || mask_of(p) & changed_bits != 0)
            .map(|(p, n)| {
                let v = if p.kind == Kind::Text {
                    let base = reg - (reg & 0xFF) + p.reg;
                    let regs: Vec<u16> = (0..p.length as u16)
                        .map(|i| if base + i == reg { new } else { self.reg.get(addr, base + i).unwrap_or(0) })
                        .collect();
                    decode(p, &regs)
                } else {
                    decode(p, &[new])
                };
                (p.id, n, v)
            })
            .collect()
    }
}
