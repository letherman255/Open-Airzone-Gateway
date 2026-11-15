//! Which blocks to read, in the stock webserver's order.

use crate::frame::Request;
use crate::topology::{Topology, ZONE_REGS};

pub const OUTLETS: u16 = 8;
pub const OUTLET_REGS: u16 = 12;

/// Per central: system 0x0000 ×80 and 0x0050 ×80, IAQ 0x2100 ×33 (if present), 8 outlet blocks ×12,
/// then every present zone ×65.
pub fn poll_plan(t: &Topology) -> Vec<Request> {
    let mut out = Vec::new();
    for c in &t.centrals {
        let addr = c.addr;
        out.push(Request::Read { addr, reg: 0x0000, count: 80 });
        out.push(Request::Read { addr, reg: 0x0050, count: 80 });
        if c.iaq {
            out.push(Request::Read { addr, reg: 0x2100, count: 33 });
        }
        for k in 0..OUTLETS {
            out.push(Request::Read { addr, reg: 0x2900 + 0x100 * k, count: OUTLET_REGS });
        }
        for z in &c.zones {
            out.push(Request::Read { addr, reg: (z.n as u16) << 8, count: ZONE_REGS });
        }
    }
    out
}

/// The read of `plan` that covers register `reg` of `addr`.
pub fn block_of(plan: &[Request], addr: u8, reg: u16) -> Option<Request> {
    plan.iter()
        .find(|r| matches!(r, Request::Read { addr: a, reg: s, count } if *a == addr && reg >= *s && reg < s + count))
        .cloned()
}
