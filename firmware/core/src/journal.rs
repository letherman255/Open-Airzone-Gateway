//! Journal of configuration changes made through the gateway, with undo (the backup the stock tool lacks).

use std::collections::VecDeque;

use crate::frame::Request;

pub const CAPACITY: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub addr: u8,
    pub reg: u16,
    /// Value before the change; `None` when unknown (the register had not been read yet).
    pub old: Option<u16>,
    pub new: u16,
    /// Bits the change touched (0xFFFF = whole register).
    pub mask: u16,
    pub t_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Journal {
    entries: VecDeque<Entry>,
}

impl Journal {
    #[allow(clippy::too_many_arguments)]
    pub fn record(&mut self, id: &str, addr: u8, reg: u16, old: Option<u16>, new: u16, mask: u16, t_ms: u64) {
        if self.entries.len() >= CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry { id: id.to_string(), addr, reg, old, new, mask, t_ms });
    }

    pub fn list(&self) -> Vec<Entry> {
        self.entries.iter().cloned().collect()
    }

    pub fn get(&self, index: usize) -> Option<&Entry> {
        self.entries.get(index)
    }

    /// Requests that put back the old value of entry `index` (oldest = 0); `None` when there is no such
    /// entry or its old value is unknown.
    pub fn undo(&self, index: usize) -> Option<Vec<Request>> {
        let e = self.entries.get(index)?;
        let old = e.old?;
        Some(vec![if e.mask == 0xFFFF {
            Request::WriteSingle { addr: e.addr, reg: e.reg, value: old }
        } else {
            Request::MaskWrite { addr: e.addr, reg: e.reg, and: !e.mask, or: old & e.mask }
        }])
    }
}
