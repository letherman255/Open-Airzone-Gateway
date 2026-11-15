//! Simulated Airzone central, seeded from a real capture or built from scratch, for tests.
//!
//! Behaviours copied from the 2026-11-05/06 observations: 0x16 mask write and its non-standard reply,
//! global mode copied to every zone, fan mode forcing 15.0 °C and restoring the per-mode setpoint,
//! system limits copied to every zone, heartbeat bit cleared by the central.

use std::collections::{BTreeMap, HashMap};

use airzone_core::bus::{Bus, BusError};
use airzone_core::frame::{crc16, Request};

fn finish(mut v: Vec<u8>) -> Vec<u8> {
    let c = crc16(&v);
    v.push((c & 0xFF) as u8);
    v.push((c >> 8) as u8);
    v
}

const FAN: u16 = 3;
const FAN_SETPOINT: u16 = 150;

#[derive(Debug, Clone)]
pub struct SimCentral {
    pub addr: u8,
    regs: BTreeMap<u16, u16>,
    blocks: Vec<(u16, u16)>,
    saved_setpoints: HashMap<(u16, u16), u16>,
    /// Registers whose writes are refused with exception 03.
    pub refuse: Vec<u16>,
}

impl SimCentral {
    fn new(addr: u8, regs: BTreeMap<u16, u16>) -> Self {
        let mut s = SimCentral { addr, regs, blocks: Vec::new(), saved_setpoints: HashMap::new(), refuse: Vec::new() };
        s.rebuild_blocks();
        s
    }

    fn zones(&self) -> Vec<u16> {
        let w9 = self.reg(0x0009);
        let wa = self.reg(0x000A);
        (0..16).filter(|b| w9 & (1 << b) != 0).map(|b| b + 1)
            .chain((0..16).filter(|b| wa & (1 << b) != 0).map(|b| b + 17))
            .collect()
    }

    fn rebuild_blocks(&mut self) {
        let mut b = vec![(0x0000, 0xD0), (0x2100, 33)];
        for k in 0..8u16 {
            b.push((0x2900 + 0x100 * k, 12));
        }
        for z in self.zones() {
            b.push((z << 8, 65));
        }
        self.blocks = b;
    }

    /// Snapshot fixture (register values decoded from a real capture): `{"centrals": {"1": {"0x0000": 258, …}}}`.
    pub fn from_fixture(json: &str, addr: u8) -> SimCentral {
        let v: serde_json::Value = serde_json::from_str(json).expect("fixture json");
        let m = v["centrals"][addr.to_string()].as_object().expect("central in fixture");
        let regs = m.iter()
            .map(|(k, v)| (u16::from_str_radix(k.trim_start_matches("0x"), 16).unwrap(), v.as_u64().unwrap() as u16))
            .collect();
        SimCentral::new(addr, regs)
    }

    pub fn builder() -> SimBuilder {
        SimBuilder { addr: 1, zones: vec![1], iaq: true }
    }

    pub fn reg(&self, r: u16) -> u16 {
        *self.regs.get(&r).unwrap_or(&0xFFFF)
    }

    fn in_block(&self, reg: u16, count: u16) -> bool {
        self.blocks.iter().any(|&(s, n)| reg >= s && (reg as u32 + count as u32) <= (s as u32 + n as u32))
    }

    fn exception(&self, f: u8, code: u8) -> Vec<u8> {
        finish(vec![self.addr, f | 0x80, code])
    }

    fn write(&mut self, reg: u16, new: u16) {
        let old = self.reg(reg);
        self.regs.insert(reg, new);
        let zones = self.zones();
        let is_zone_word0 = reg & 0xFF == 0 && zones.contains(&(reg >> 8));
        if is_zone_word0 && (old >> 8) & 0xF != (new >> 8) & 0xF {
            let (from, to) = ((old >> 8) & 0xF, (new >> 8) & 0xF);
            for z in &zones {
                let r0 = z << 8;
                let sp = self.reg(r0 + 3);
                if from != FAN {
                    self.saved_setpoints.insert((*z, from), sp);
                }
                let w = (self.reg(r0) & 0xF0FF) | (to << 8);
                self.regs.insert(r0, w);
                let next = if to == FAN { Some(FAN_SETPOINT) } else { self.saved_setpoints.get(&(*z, to)).copied() };
                if let Some(v) = next {
                    self.regs.insert(r0 + 3, v);
                }
            }
            let s0 = (self.reg(0x0000) & 0xFFF0) | to;
            self.regs.insert(0x0000, s0);
        }
        if reg == 0x0019 || reg == 0x001A {
            for z in &zones {
                self.regs.insert((z << 8) + if reg == 0x0019 { 1 } else { 2 }, new);
            }
        }
        if reg == 0x0009 || reg == 0x000A {
            self.rebuild_blocks();
        }
    }

    /// Answer one request frame; `None` when this central does not answer (other address, broadcast, garbage).
    pub fn handle(&mut self, request: &[u8]) -> Option<Vec<u8>> {
        let req = Request::parse(request).ok()?;
        if req.addr() != self.addr {
            return None;
        }
        Some(match req {
            Request::Read { reg, count, .. } => {
                if count == 0 || !self.in_block(reg, count) {
                    return Some(self.exception(0x03, 2));
                }
                if reg <= 0x0001 && reg + count > 0x0001 {
                    let hb = self.reg(0x0001) & !0x0010;
                    self.regs.insert(0x0001, hb);
                }
                let mut v = vec![self.addr, 0x03, (count * 2) as u8];
                for i in 0..count {
                    v.extend(self.reg(reg + i).to_be_bytes());
                }
                finish(v)
            }
            Request::WriteSingle { reg, value, .. } => {
                if self.refuse.contains(&reg) {
                    return Some(self.exception(0x06, 3));
                }
                self.write(reg, value);
                request.to_vec()
            }
            Request::MaskWrite { reg, and, or, .. } => {
                if self.refuse.contains(&reg) {
                    return Some(self.exception(0x16, 3));
                }
                let new = (self.reg(reg) & and) | (or & !and);
                self.write(reg, new);
                let mut v = vec![self.addr, 0x16];
                v.extend(reg.to_be_bytes());
                v.extend([0, 0]);
                v.extend(and.to_be_bytes());
                finish(v)
            }
            Request::Diag { .. } => request.to_vec(),
        })
    }
}

pub struct SimBuilder {
    addr: u8,
    zones: Vec<u8>,
    iaq: bool,
}

impl SimBuilder {
    pub fn addr(mut self, a: u8) -> Self {
        self.addr = a;
        self
    }
    pub fn zones(mut self, z: &[u8]) -> Self {
        self.zones = z.to_vec();
        self
    }
    pub fn iaq(mut self, on: bool) -> Self {
        self.iaq = on;
        self
    }
    pub fn build(self) -> SimCentral {
        let mut regs = BTreeMap::new();
        let (mut w9, mut wa) = (0u16, 0u16);
        for &z in &self.zones {
            if z <= 16 { w9 |= 1 << (z - 1) } else { wa |= 1 << (z - 17) }
        }
        for r in 0..0xD0u16 {
            regs.insert(r, 0);
        }
        regs.insert(0x0000, 0x0005);
        regs.insert(0x0001, 0x0400);
        regs.insert(0x0009, w9);
        regs.insert(0x000A, wa);
        regs.insert(0x0019, 190);
        regs.insert(0x001A, 250);
        regs.insert(0x0030, 0xA079);
        for r in 0x2100..0x2121u16 {
            regs.insert(r, 0xFFFF);
        }
        if self.iaq {
            regs.insert(0x2100, 0x0105);
        }
        for k in 0..8u16 {
            let base = 0x2900 + 0x100 * k;
            for i in 0..12 {
                regs.insert(base + i, 0);
            }
            let outlet = k + 1;
            let linked = if self.zones.contains(&(outlet as u8)) { outlet } else { 0 };
            regs.insert(base, (outlet << 8) | linked);
            regs.insert(base + 1, if linked != 0 { 0x0031 } else { 0x0030 });
        }
        for (i, &z) in self.zones.iter().enumerate() {
            let b = (z as u16) << 8;
            for o in 0..65 {
                regs.insert(b + o, 0xFFFF);
            }
            regs.insert(b, 0x0504);
            regs.insert(b + 1, 190);
            regs.insert(b + 2, 250);
            regs.insert(b + 3, 210);
            regs.insert(b + 4, if i == 0 { 0x8807 } else { 0x8806 });
            regs.insert(b + 8, 200);
            regs.insert(b + 10, 200);
            let name = format!("Zone {z}");
            let nb = name.as_bytes();
            for j in 0..6u16 {
                let hi = *nb.get(2 * j as usize).unwrap_or(&0);
                let lo = *nb.get(2 * j as usize + 1).unwrap_or(&0);
                regs.insert(b + 14 + j, u16::from_be_bytes([hi, lo]));
            }
            regs.insert(b + 31, 45);
        }
        SimCentral::new(self.addr, regs)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Faults {
    /// Addresses that never answer.
    pub silent: Vec<u8>,
    /// Corrupt the CRC of every n-th reply.
    pub bad_crc_every: Option<u32>,
    /// Another master is talking on the bus (heard by `listen`).
    pub foreign_master: bool,
    /// Write requests (06 / 16) get no answer and are not applied.
    pub drop_writes: bool,
    /// Read requests get no answer.
    pub drop_reads: bool,
    /// Writes are acknowledged but not applied.
    pub ack_without_apply: bool,
    /// Writes are applied but their echo is lost.
    pub lose_write_replies: bool,
    /// The next n writes are ignored (not applied, no answer), as the real central does now and then.
    pub ignore_next_writes: u32,
}

#[derive(Debug, Default)]
pub struct SimBus {
    pub centrals: Vec<SimCentral>,
    pub faults: Faults,
    /// Every frame the master sent.
    pub sent: Vec<Vec<u8>>,
    /// Number of `listen` calls (to check nothing was sent before).
    pub listened_ms: u64,
    replies: u32,
}

impl SimBus {
    pub fn new(centrals: Vec<SimCentral>) -> Self {
        SimBus { centrals, ..Default::default() }
    }

    pub fn central(&self, addr: u8) -> &SimCentral {
        self.centrals.iter().find(|c| c.addr == addr).expect("central")
    }

    pub fn central_mut(&mut self, addr: u8) -> &mut SimCentral {
        self.centrals.iter_mut().find(|c| c.addr == addr).expect("central")
    }
}

impl Bus for SimBus {
    fn transact(&mut self, request: &[u8], _timeout_ms: u32) -> Result<Vec<u8>, BusError> {
        self.sent.push(request.to_vec());
        if request.len() < 2 || self.faults.silent.contains(&request[0]) {
            return Err(BusError::Timeout);
        }
        let is_write = matches!(request[1], 0x06 | 0x16);
        if self.faults.drop_writes && is_write {
            return Err(BusError::Timeout);
        }
        if is_write && self.faults.ignore_next_writes > 0 {
            self.faults.ignore_next_writes -= 1;
            return Err(BusError::Timeout);
        }
        if self.faults.drop_reads && request[1] == 0x03 {
            return Err(BusError::Timeout);
        }
        if self.faults.ack_without_apply && is_write {
            let addr = request[0];
            let Some(c) = self.centrals.iter_mut().find(|c| c.addr == addr) else { return Err(BusError::Timeout) };
            let mut copy = c.clone();
            return copy.handle(request).ok_or(BusError::Timeout);
        }
        let mut reply = None;
        for c in &mut self.centrals {
            if let Some(r) = c.handle(request) {
                reply = Some(r);
            }
        }
        let mut reply = reply.ok_or(BusError::Timeout)?;
        if self.faults.lose_write_replies && is_write {
            return Err(BusError::Timeout);
        }
        self.replies += 1;
        if let Some(n) = self.faults.bad_crc_every {
            if n > 0 && self.replies % n == 0 {
                let last = reply.len() - 1;
                reply[last] ^= 0xFF;
            }
        }
        Ok(reply)
    }

    fn listen(&mut self, duration_ms: u32) -> Vec<Vec<u8>> {
        self.listened_ms += duration_ms as u64;
        if self.faults.foreign_master {
            vec![Request::Read { addr: 1, reg: 0x0000, count: 80 }.encode()]
        } else {
            Vec::new()
        }
    }
}
