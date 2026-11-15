//! Last known value of every register, per central address, with the time of its last change.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub addr: u8,
    pub reg: u16,
    pub old: Option<u16>,
    pub new: u16,
}

#[derive(Debug, Default, Clone)]
pub struct Registry {
    regs: BTreeMap<(u8, u16), (u16, u64)>,
}

impl Registry {
    /// Store a block read from `addr` starting at `start`; returns the registers whose value changed
    /// (or that were unknown before).
    pub fn update(&mut self, addr: u8, start: u16, values: &[u16], now_ms: u64) -> Vec<Change> {
        let mut out = Vec::new();
        for (i, &v) in values.iter().enumerate() {
            let reg = start.wrapping_add(i as u16);
            let old = self.regs.get(&(addr, reg)).map(|e| e.0);
            if old != Some(v) {
                self.regs.insert((addr, reg), (v, now_ms));
                out.push(Change { addr, reg, old, new: v });
            }
        }
        out
    }

    /// Mirror the changes reported by another registry's `update` (no full copy needed).
    pub fn apply(&mut self, changes: &[Change], now_ms: u64) {
        for c in changes {
            self.regs.insert((c.addr, c.reg), (c.new, now_ms));
        }
    }

    pub fn get(&self, addr: u8, reg: u16) -> Option<u16> {
        self.regs.get(&(addr, reg)).map(|e| e.0)
    }

    /// `len` consecutive registers, `None` if any is unknown.
    pub fn get_range(&self, addr: u8, start: u16, len: u16) -> Option<Vec<u16>> {
        (0..len).map(|i| self.get(addr, start.wrapping_add(i))).collect()
    }

    pub fn changed_at(&self, addr: u8, reg: u16) -> Option<u64> {
        self.regs.get(&(addr, reg)).map(|e| e.1)
    }

    /// (address, register, value) of every register changed at or after `ms`.
    pub fn changed_since(&self, ms: u64) -> Vec<(u8, u16, u16)> {
        self.regs.iter().filter(|(_, (_, t))| *t >= ms).map(|((a, r), (v, _))| (*a, *r, *v)).collect()
    }

    /// Every known (register, value) of `addr`, ordered by register.
    pub fn dump(&self, addr: u8) -> Vec<(u16, u16)> {
        self.regs.range((addr, 0)..=(addr, u16::MAX)).map(|(k, v)| (k.1, v.0)).collect()
    }
}
