//! The gateway as bus master: refuse to start next to another master, discover centrals and zones,
//! identify, poll, apply writes (never retried, result taken from a re-read), heartbeat.

use std::collections::{BTreeMap, VecDeque};

use crate::bus::Bus;
use crate::frame::{parse_response, Request, Response};
use crate::poll::{block_of, poll_plan};
use crate::registry::{Change, Registry};
use crate::rules::{heartbeat, identification, identity_requests, Identity};
use crate::topology::{build_central, zones_present, Central, Topology, ZONE_REGS};

pub const LISTEN_BEFORE_START_MS: u32 = 5_000;
pub const HEARTBEAT_MS: u64 = 45_000;
pub const IDENTIFICATION_MS: u64 = 60_000;
pub const READ_RETRIES: u32 = 2;
pub const MISSED_CYCLES_UNAVAILABLE: u32 = 3;
pub const MAX_CENTRAL_ADDR: u8 = 32;

#[derive(Debug, Clone)]
pub struct Config {
    pub identity: Identity,
    pub heartbeat: bool,
    pub identification: bool,
    pub cycle_ms: u32,
    pub rediscover_ms: u64,
    pub timeout_ms: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config { identity: Identity::default(), heartbeat: true, identification: true, cycle_ms: 2_300, rediscover_ms: 600_000, timeout_ms: 300 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    ForeignMaster,
    NoCentral,
}

pub type WriteId = u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteResult {
    /// Value of each written register after the re-read, in request order.
    Applied(Vec<u16>),
    /// Refused by the central after `sent` earlier requests of the batch were accepted.
    Rejected { code: u8, sent: usize },
    /// No valid answer after `sent` earlier requests of the batch were accepted.
    NoAnswer { sent: usize },
    /// Accepted, but the re-read failed, the register is not polled, or it does not hold the written value.
    Unverified,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CycleReport {
    pub changes: Vec<Change>,
    /// A periodic rediscovery ran: its reads are not in `changes`.
    pub rediscovered: bool,
    pub writes: Vec<(WriteId, WriteResult)>,
    pub unavailable: Vec<u8>,
    pub topology_changed: bool,
    /// A poll round ended in this call (always true for `cycle`).
    pub round_done: bool,
}

pub struct Master<B: Bus> {
    bus: B,
    cfg: Config,
    reg: Registry,
    topo: Topology,
    plan: Vec<Request>,
    queue: VecDeque<(WriteId, Vec<Request>)>,
    next_id: WriteId,
    misses: BTreeMap<u8, u32>,
    last_heartbeat: u64,
    last_identification: u64,
    last_discovery: u64,
    /// Next block of the plan for `poll_step`.
    cursor: usize,
    /// Centrals that answered during the current round.
    answered: BTreeMap<u8, bool>,
    /// Reads that never got a valid answer (after retries), since start.
    read_failures: u32,
}

impl<B: Bus> Master<B> {
    pub fn new(bus: B, cfg: Config) -> Self {
        Master {
            bus, cfg, reg: Registry::default(), topo: Topology::default(), plan: Vec::new(), queue: VecDeque::new(),
            next_id: 1, misses: BTreeMap::new(), last_heartbeat: 0, last_identification: 0, last_discovery: 0,
            cursor: 0, answered: BTreeMap::new(), read_failures: 0,
        }
    }

    /// Reads that never got a valid answer (after retries), since start.
    pub fn read_failures(&self) -> u32 {
        self.read_failures
    }

    pub fn registry(&self) -> &Registry {
        &self.reg
    }
    pub fn topology(&self) -> &Topology {
        &self.topo
    }
    pub fn plan(&self) -> &[Request] {
        &self.plan
    }
    pub fn bus(&self) -> &B {
        &self.bus
    }
    pub fn bus_mut(&mut self) -> &mut B {
        &mut self.bus
    }
    pub fn config(&self) -> &Config {
        &self.cfg
    }
    /// Give the bus back (for example to keep listening after `start` refused).
    pub fn into_bus(self) -> B {
        self.bus
    }

    /// Discover again now; true when the topology changed (the poll plan follows).
    pub fn rediscover(&mut self, now_ms: u64) -> bool {
        let t = self.discover(now_ms);
        if t.centrals.is_empty() || t == self.topo {
            return false;
        }
        self.topo = t;
        self.plan = poll_plan(&self.topo);
        true
    }

    fn ask(&mut self, req: &Request) -> Option<Response> {
        let reply = self.bus.transact(&req.encode(), self.cfg.timeout_ms).ok()?;
        parse_response(req, &reply).ok()
    }

    /// A read, retried on timeout / bad frame; stores the values. `None` if it never succeeded.
    fn read(&mut self, req: &Request, now_ms: u64, changes: &mut Vec<Change>) -> Option<Vec<u16>> {
        let Request::Read { addr, reg, .. } = *req else { return None };
        for _ in 0..=READ_RETRIES {
            match self.ask(req) {
                Some(Response::Registers(v)) => {
                    changes.extend(self.reg.update(addr, reg, &v, now_ms));
                    return Some(v);
                }
                Some(Response::Exception { .. }) => break,
                _ => continue,
            }
        }
        self.read_failures = self.read_failures.saturating_add(1);
        None
    }

    fn discover(&mut self, now_ms: u64) -> Topology {
        let mut centrals = Vec::new();
        let mut sink = Vec::new();
        for addr in 1..=MAX_CENTRAL_ADDR {
            // probe with the read the stock webserver uses (the system block); no retry for absent addresses
            let probe = Request::Read { addr, reg: 0x0000, count: 0x50 };
            match self.ask(&probe) {
                Some(Response::Registers(v)) => {
                    self.reg.update(addr, 0x0000, &v, now_ms);
                }
                _ => continue,
            }
            for (reg, count) in [(0x0050, 80), (0x2100, 33)] {
                self.read(&Request::Read { addr, reg, count }, now_ms, &mut sink);
            }
            let (Some(w9), Some(wa)) = (self.reg.get(addr, 0x0009), self.reg.get(addr, 0x000A)) else { continue };
            for z in zones_present(w9, wa) {
                self.read(&Request::Read { addr, reg: (z as u16) << 8, count: ZONE_REGS }, now_ms, &mut sink);
            }
            if let Some(c) = build_central(addr, &self.reg) {
                centrals.push(c);
            }
        }
        self.last_discovery = now_ms;
        Topology { centrals }
    }

    fn send_once(&mut self, req: &Request) {
        let _ = self.ask(req);
    }

    /// Listen first (another master → refuse), discover, send the identity sequence.
    pub fn start(&mut self, now_ms: u64) -> Result<&Topology, StartError> {
        if !self.bus.listen(LISTEN_BEFORE_START_MS).is_empty() {
            return Err(StartError::ForeignMaster);
        }
        self.topo = self.discover(now_ms);
        if self.topo.centrals.is_empty() {
            return Err(StartError::NoCentral);
        }
        self.plan = poll_plan(&self.topo);
        let addrs: Vec<u8> = self.topo.centrals.iter().map(|c| c.addr).collect();
        for addr in addrs {
            if self.cfg.identification {
                for r in identity_requests(addr, &self.cfg.identity) {
                    self.send_once(&r);
                }
            }
            if self.cfg.heartbeat {
                self.send_once(&heartbeat(addr));
            }
        }
        self.last_heartbeat = now_ms;
        self.last_identification = now_ms;
        Ok(&self.topo)
    }

    /// Queue requests that are applied together at the start of the next cycle.
    pub fn submit(&mut self, requests: Vec<Request>) -> WriteId {
        let id = self.next_id;
        self.next_id += 1;
        self.queue.push_back((id, requests));
        id
    }

    /// (addr, reg, bits that must match, expected value of those bits) of a write request.
    fn target_bits(r: &Request) -> Option<(u8, u16, u16, u16)> {
        match *r {
            Request::WriteSingle { addr, reg, value } => Some((addr, reg, 0xFFFF, value)),
            Request::MaskWrite { addr, reg, and, or } => Some((addr, reg, !and, or & !and)),
            _ => None,
        }
    }

    /// Re-read the block of `r` and tell whether the central holds the written value.
    fn holds_after_reread(&mut self, r: &Request, now_ms: u64, changes: &mut Vec<Change>) -> bool {
        let Some((addr, reg, mask, want)) = Self::target_bits(r) else { return false };
        let Some(b) = block_of(&self.plan, addr, reg) else { return false };
        self.read(&b, now_ms, changes).is_some() && self.reg.get(addr, reg).is_some_and(|v| v & mask == want)
    }

    /// Send a batch, then re-read every block it touched — also after a partial batch — and report `Applied`
    /// only if each written register holds the written value after the re-read.
    ///
    /// A write without a valid answer is checked by re-reading its block: if the value is in place it counts
    /// as done (lost echo); if not, it is sent once more (the real central ignores roughly one write in ten,
    /// 2026-11-06; writes are absolute values, so a second send cannot double anything), then checked again.
    fn apply(&mut self, reqs: &[Request], now_ms: u64, changes: &mut Vec<Change>) -> WriteResult {
        let mut sent = 0;
        let mut failure = None;
        'batch: for r in reqs {
            let echo_ok = |resp: &Response| match (r, resp) {
                (Request::WriteSingle { reg, value, .. }, Response::WriteSingle { reg: r2, value: v2 }) => reg == r2 && value == v2,
                (Request::MaskWrite { reg, and, .. }, Response::MaskAck { reg: r2, and: a2 }) => reg == r2 && and == a2,
                (Request::Diag { .. }, Response::Diag { .. }) => true,
                _ => false,
            };
            for attempt in 0..2 {
                match self.ask(r) {
                    Some(Response::Exception { code, .. }) => {
                        failure = Some(WriteResult::Rejected { code, sent });
                        break 'batch;
                    }
                    Some(resp) if echo_ok(&resp) => {
                        sent += 1;
                        continue 'batch;
                    }
                    _ => {
                        if self.holds_after_reread(r, now_ms, changes) {
                            sent += 1;
                            continue 'batch;
                        }
                        if attempt == 1 || Self::target_bits(r).is_none() {
                            failure = Some(WriteResult::NoAnswer { sent });
                            break 'batch;
                        }
                    }
                }
            }
        }
        let written: Vec<(u8, u16, u16, u16)> = reqs[..sent].iter().filter_map(Self::target_bits).collect();
        let mut blocks: Vec<Request> = Vec::new();
        let mut verified = true;
        for &(addr, reg, _, _) in &written {
            match block_of(&self.plan, addr, reg) {
                Some(b) if !blocks.contains(&b) => blocks.push(b),
                Some(_) => {}
                None => verified = false,
            }
        }
        for b in &blocks {
            if self.read(b, now_ms, changes).is_none() {
                verified = false;
            }
        }
        if let Some(f) = failure {
            return f;
        }
        let mut values = Vec::new();
        for &(addr, reg, mask, want) in &written {
            match self.reg.get(addr, reg).filter(|v| v & mask == want) {
                Some(v) => values.push(v),
                None => verified = false,
            }
        }
        if verified {
            WriteResult::Applied(values)
        } else {
            WriteResult::Unverified
        }
    }

    /// Only the queued writes, each followed by the re-read of its block (no polling): lets a command go out
    /// as soon as it arrives instead of waiting for the next cycle.
    pub fn apply_queued(&mut self, now_ms: u64) -> CycleReport {
        let mut rep = CycleReport::default();
        while let Some((id, reqs)) = self.queue.pop_front() {
            let res = self.apply(&reqs, now_ms, &mut rep.changes);
            rep.writes.push((id, res));
        }
        rep
    }

    /// Start of a poll round: periodic heartbeat and identification.
    fn begin_round(&mut self, now_ms: u64) {
        let addrs: Vec<u8> = self.topo.centrals.iter().map(|c| c.addr).collect();
        if self.cfg.heartbeat && now_ms >= self.last_heartbeat + HEARTBEAT_MS {
            for &a in &addrs {
                self.send_once(&heartbeat(a));
            }
            self.last_heartbeat = now_ms;
        }
        if self.cfg.identification && now_ms >= self.last_identification + IDENTIFICATION_MS {
            for &a in &addrs {
                let r = identification(a, &self.cfg.identity);
                self.send_once(&r);
            }
            self.last_identification = now_ms;
        }
        self.answered = addrs.iter().map(|&a| (a, false)).collect();
    }

    /// One block of the poll plan.
    fn read_step(&mut self, index: usize, now_ms: u64, changes: &mut Vec<Change>) {
        let Some(r) = self.plan.get(index).cloned() else { return };
        if self.read(&r, now_ms, changes).is_some() {
            self.answered.insert(r.addr(), true);
        }
    }

    /// End of a poll round: unavailable centrals, periodic rediscovery, topology refresh.
    fn end_round(&mut self, now_ms: u64, rep: &mut CycleReport) {
        for (a, ok) in std::mem::take(&mut self.answered) {
            let m = self.misses.entry(a).or_insert(0);
            *m = if ok { 0 } else { *m + 1 };
            if *m >= MISSED_CYCLES_UNAVAILABLE {
                rep.unavailable.push(a);
            }
        }
        let rediscover = now_ms >= self.last_discovery + self.cfg.rediscover_ms;
        rep.rediscovered = rediscover;
        let new_topo = if rediscover {
            self.discover(now_ms)
        } else {
            let centrals: Vec<Central> = self.topo.centrals.iter()
                .map(|c| build_central(c.addr, &self.reg).unwrap_or_else(|| c.clone()))
                .collect();
            Topology { centrals }
        };
        if !new_topo.centrals.is_empty() && new_topo != self.topo {
            self.topo = new_topo;
            self.plan = poll_plan(&self.topo);
            rep.topology_changed = true;
        }
        rep.round_done = true;
    }

    /// One cycle: queued writes (each followed by a re-read of its block), periodic heartbeat and
    /// identification, then the whole poll plan.
    pub fn cycle(&mut self, now_ms: u64) -> CycleReport {
        let mut rep = self.apply_queued(now_ms);
        self.begin_round(now_ms);
        for i in 0..self.plan.len() {
            self.read_step(i, now_ms, &mut rep.changes);
        }
        self.end_round(now_ms, &mut rep);
        self.cursor = 0;
        rep
    }

    /// The same work as `cycle`, one block read per call: queued writes first, then the next block of the
    /// plan; the round ends (heartbeat, identification, unavailable, rediscovery) after the last block. A
    /// command then waits one block read (~0.1 s) instead of a whole cycle (~2 s on a 6-zone home).
    pub fn poll_step(&mut self, now_ms: u64) -> CycleReport {
        let mut rep = self.apply_queued(now_ms);
        if self.cursor == 0 {
            self.begin_round(now_ms);
        }
        self.read_step(self.cursor, now_ms, &mut rep.changes);
        self.cursor += 1;
        if self.cursor >= self.plan.len() {
            self.end_round(now_ms, &mut rep);
            self.cursor = 0;
        }
        rep
    }
}
