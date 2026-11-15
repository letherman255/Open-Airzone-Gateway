//! What is installed: centrals, their zones, the IAQ sensor. Built from registers, never from counts.

use crate::model::text_of;
use crate::registry::Registry;

pub const ZONE_REGS: u16 = 65;
pub const REG_ZONES_1_16: u16 = 0x0009;
pub const REG_ZONES_17_32: u16 = 0x000A;
pub const REG_IAQ_FW: u16 = 0x2100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    pub n: u8,
    pub name: String,
    pub master: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Central {
    pub addr: u8,
    pub zones: Vec<Zone>,
    pub iaq: bool,
}

impl Central {
    pub fn master_zone(&self) -> Option<u8> {
        self.zones.iter().find(|z| z.master).map(|z| z.n)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Topology {
    pub centrals: Vec<Central>,
}

/// Zone numbers present: bit n-1 of 0x0009 for zones 1-16, bit n-17 of 0x000A for zones 17-32.
pub fn zones_present(r0009: u16, r000a: u16) -> Vec<u8> {
    let mut out = Vec::new();
    for (word, first) in [(r0009, 1u8), (r000a, 17u8)] {
        for bit in 0..16u8 {
            if word & (1 << bit) != 0 {
                out.push(first + bit);
            }
        }
    }
    out
}

/// Zone name from registers +14..+19.
pub fn zone_name(regs: &[u16]) -> String {
    text_of(regs)
}

pub fn iaq_present(fw: u16) -> bool {
    fw != 0 && fw != 0xFFFF
}

/// Central `addr` as far as the registry knows it; `None` until 0x0009, 0x000A and every present
/// zone's 65 registers are known.
pub fn build_central(addr: u8, reg: &Registry) -> Option<Central> {
    let ns = zones_present(reg.get(addr, REG_ZONES_1_16)?, reg.get(addr, REG_ZONES_17_32)?);
    let mut zones = Vec::new();
    for n in ns {
        let block = reg.get_range(addr, (n as u16) << 8, ZONE_REGS)?;
        zones.push(Zone { n, name: zone_name(&block[14..20]), master: block[4] & 1 != 0 });
    }
    let iaq = reg.get(addr, REG_IAQ_FW).is_some_and(iaq_present);
    Some(Central { addr, zones, iaq })
}
