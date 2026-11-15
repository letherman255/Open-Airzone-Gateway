//! Behaviour the catalogue cannot express: global mode, VMC calendar, gateway identity, heartbeat.

use crate::catalogue::param;
use crate::frame::Request;
use crate::model::EncodeError;
use crate::registry::Registry;
use crate::topology::Central;

/// Change the machine mode of a central. The mode is global: it is written on the master zone only and
/// the central copies it to every zone. The master zone is switched on first if it is off (stock order).
pub fn set_mode(c: &Central, reg: &Registry, mode: u16) -> Result<Vec<Request>, EncodeError> {
    let p = param("zone.mode").expect("catalogue has zone.mode");
    if !p.values.iter().any(|(k, _)| *k == mode) {
        return Err(EncodeError::UnknownChoice);
    }
    let master = c.master_zone().ok_or(EncodeError::WrongTarget)?;
    let r0 = (master as u16) << 8;
    let mut out = Vec::new();
    if reg.get(c.addr, r0).is_some_and(|w| w & 0x0004 == 0) {
        out.push(Request::MaskWrite { addr: c.addr, reg: r0, and: 0xFFFB, or: 0x0004 });
    }
    out.push(Request::MaskWrite { addr: c.addr, reg: r0, and: 0xF0FF, or: mode << 8 });
    Ok(out)
}

/// VMC calendar bits the gateway must drive (the central does not store the lists):
/// 0x2101 bit 13 = air renewal allowed today, bit 15 = free cooling allowed this month.
/// Days: 0 = Monday … 6 = Sunday; months: 0 = January … 11 = December.
pub fn calendar_writes(addr: u8, renew_days: &[u8], fcool_months: &[u8], weekday_mon0: u8, month_jan0: u8) -> [Request; 2] {
    let renew = renew_days.contains(&weekday_mon0) as u16;
    let fcool = fcool_months.contains(&month_jan0) as u16;
    [
        Request::MaskWrite { addr, reg: 0x2101, and: 0xDFFF, or: renew << 13 },
        Request::MaskWrite { addr, reg: 0x2101, and: 0x7FFF, or: fcool << 15 },
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// 10 ASCII characters, sent in the 08 identification frame.
    pub name: [u8; 10],
    pub mac: [u8; 6],
    pub ip: [u8; 4],
    pub gateway_ip: [u8; 4],
    /// Firmware version word written to 0x0059 (stock: 0x0423 = 4.23).
    pub fw: u16,
}

impl Default for Identity {
    fn default() -> Self {
        Identity { name: *b"OPENAZWS01", mac: [0; 6], ip: [0; 4], gateway_ip: [0; 4], fw: 0x0100 }
    }
}

/// Bytes of the stock 08 identification frame between the sub-function and the name (meaning unknown).
const IDENT_PREFIX: [u8; 10] = [0x00, 0x12, 0x01, 0x00, 0x81, 0x00, 0x20, 0x00, 0x00, 0x30];

fn words(b: &[u8]) -> impl Iterator<Item = u16> + '_ {
    b.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]]))
}

/// The stock start-up identity sequence, with our values.
pub fn identity_requests(addr: u8, id: &Identity) -> Vec<Request> {
    let mut data = IDENT_PREFIX.to_vec();
    data.extend_from_slice(&id.name);
    let mut out = vec![Request::Diag { addr, sub: 0x0003, data }];
    for (i, w) in words(&id.mac).enumerate() {
        out.push(Request::WriteSingle { addr, reg: 0x004E + i as u16, value: w });
    }
    for (i, w) in words(&id.ip).enumerate() {
        out.push(Request::WriteSingle { addr, reg: 0x002E + i as u16, value: w });
    }
    out.push(Request::WriteSingle { addr, reg: 0x0059, value: id.fw });
    for (i, w) in words(&id.gateway_ip).enumerate() {
        out.push(Request::WriteSingle { addr, reg: 0x004C + i as u16, value: w });
    }
    out.push(Request::MaskWrite { addr, reg: 0x008B, and: 0xF8FF, or: 0x0100 });
    out.push(Request::MaskWrite { addr, reg: 0x008B, and: 0xF8FF, or: 0x0500 });
    out
}

/// Periodic identification (stock: every 60 s).
pub fn identification(addr: u8, id: &Identity) -> Request {
    identity_requests(addr, id).remove(0)
}

/// Gateway alive bit (stock: every ~45 s; the central clears it).
pub fn heartbeat(addr: u8) -> Request {
    Request::MaskWrite { addr, reg: 0x0001, and: 0xFFEF, or: 0x0010 }
}
