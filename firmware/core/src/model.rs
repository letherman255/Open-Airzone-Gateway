//! Registers ↔ typed values, driven by the catalogue.

use crate::catalogue::{Confidence, Kind, Param, Scope};
use crate::frame::Request;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f32),
    Enum(u16, &'static str),
    Bool(bool),
    Text(String),
    /// Raw field value that the catalogue cannot interpret (unknown enum value, unused 0xFFFF register).
    Unknown(u16),
}

/// Where a parameter lives. IAQ parameters use `System` (absolute registers on the central).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    System { addr: u8 },
    Zone { addr: u8, zone: u8 },
    Outlet { addr: u8, outlet: u8 },
}

impl Target {
    pub fn addr(&self) -> u8 {
        match *self {
            Target::System { addr } | Target::Zone { addr, .. } | Target::Outlet { addr, .. } => addr,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Num(f32),
    Choice(u16),
    Bool(bool),
    Text(String),
    Trigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    NotWritable,
    NotConfirmed,
    OutOfRange,
    /// Not a multiple of the parameter's step (the central would get a rounded value).
    OffStep,
    UnknownChoice,
    TextTooLong,
    ConfirmRequired,
    WrongTarget,
    WrongInput,
}

fn field_mask(p: &Param) -> u16 {
    let width = (p.hi - p.lo + 1) as u32;
    if width >= 16 {
        0xFFFF
    } else {
        (((1u32 << width) - 1) << p.lo) as u16
    }
}

fn field(p: &Param, raw: u16) -> u16 {
    (raw & field_mask(p)) >> p.lo
}

/// Highest zone number of a central (zones 1-32).
pub const MAX_ZONE: u8 = 32;
/// Outlets per central (1-8).
pub const MAX_OUTLET: u8 = 8;

/// Base register of the block the parameter is in, `None` when the target does not fit the scope.
pub fn block_base(p: &Param, t: &Target) -> Option<u16> {
    match (p.scope, t) {
        (Scope::System | Scope::Iaq, Target::System { .. }) => Some(0),
        (Scope::Zone, Target::Zone { zone, .. }) if (1..=MAX_ZONE).contains(zone) => Some((*zone as u16) << 8),
        (Scope::Outlet, Target::Outlet { outlet, .. }) if (1..=MAX_OUTLET).contains(outlet) => {
            Some(0x2900 + 0x100 * (*outlet as u16 - 1))
        }
        _ => None,
    }
}

/// Absolute register of `p` for `t` (panics if the target does not fit the scope: use `block_base` to check).
pub fn abs_reg(p: &Param, t: &Target) -> u16 {
    block_base(p, t).expect("target does not fit the parameter scope") + p.reg
}

/// Catalogue parameters stored in absolute register `abs` of a central, with the zone or outlet number
/// for zone / outlet parameters (`None` for system and IAQ).
pub fn params_at(abs: u16) -> Vec<(&'static Param, Option<u8>)> {
    let (scope, number, offset) = match abs {
        0x0000..=0x00FF => (Scope::System, None, abs),
        0x0100..=0x20FF => (Scope::Zone, Some((abs >> 8) as u8), abs & 0xFF),
        0x2100..=0x28FF => (Scope::Iaq, None, abs),
        0x2900..=0x30FF => (Scope::Outlet, Some((((abs - 0x2900) >> 8) + 1) as u8), abs & 0xFF),
        _ => return Vec::new(),
    };
    crate::catalogue::PARAMS
        .iter()
        .filter(|p| p.scope == scope && offset >= p.reg && offset < p.reg + p.length.max(1) as u16)
        .map(|p| (p, number))
        .collect()
}

/// Bit mask of `p`'s field (all 16 bits for whole-register parameters).
pub fn mask_of(p: &Param) -> u16 {
    field_mask(p)
}

/// Decode `p` from its `p.length` registers.
pub fn decode(p: &Param, raw: &[u16]) -> Value {
    // an unused register reads 0xFFFF: no field of it means anything
    if p.kind != Kind::Text && raw.first() == Some(&0xFFFF) {
        return Value::Unknown(0xFFFF);
    }
    match p.kind {
        Kind::Text => Value::Text(text_of(raw)),
        Kind::Bool | Kind::Command => Value::Bool(field(p, raw[0]) != 0),
        Kind::Enum => {
            let v = field(p, raw[0]);
            match p.values.iter().find(|(k, _)| *k == v) {
                Some((k, name)) => Value::Enum(*k, name),
                None => Value::Unknown(v),
            }
        }
        Kind::Number => {
            Value::Num(field(p, raw[0]) as f32 * p.scale + p.offset)
        }
    }
}

/// Zone-name style text: two characters per register, high byte first, ends at the first 0x00;
/// non-ASCII bytes become U+FFFD.
pub fn text_of(raw: &[u16]) -> String {
    let mut s = String::new();
    for &r in raw {
        for b in r.to_be_bytes() {
            if b == 0 {
                return s;
            }
            s.push(if b.is_ascii() && !b.is_ascii_control() { b as char } else { '\u{FFFD}' });
        }
    }
    s
}

/// Requests that write `input` to `p` at `t`. Refuses anything the catalogue does not allow.
pub fn encode(p: &Param, t: &Target, input: &Input, confirm: Option<&str>) -> Result<Vec<Request>, EncodeError> {
    if !p.writable {
        return Err(EncodeError::NotWritable);
    }
    if p.confidence != Confidence::Confirmed {
        return Err(EncodeError::NotConfirmed);
    }
    let base = block_base(p, t).ok_or(EncodeError::WrongTarget)?;
    if let Some(d) = p.danger {
        if confirm != Some(d.confirm) {
            return Err(EncodeError::ConfirmRequired);
        }
    }
    let addr = t.addr();
    let reg = base + p.reg;
    let width_max = (field_mask(p) >> p.lo) as u32;
    let raw: u32 = match (p.kind, input) {
        (Kind::Text, Input::Text(s)) => {
            let bytes = s.as_bytes();
            if bytes.len() > 2 * p.length as usize {
                return Err(EncodeError::TextTooLong);
            }
            if !s.is_ascii() {
                return Err(EncodeError::WrongInput);
            }
            return Ok((0..p.length as usize)
                .map(|i| {
                    let hi = *bytes.get(2 * i).unwrap_or(&0);
                    let lo = *bytes.get(2 * i + 1).unwrap_or(&0);
                    Request::WriteSingle { addr, reg: reg + i as u16, value: u16::from_be_bytes([hi, lo]) }
                })
                .collect());
        }
        (Kind::Bool, Input::Bool(b)) => *b as u32,
        (Kind::Command, Input::Trigger) => width_max,
        (Kind::Enum, Input::Choice(c)) => {
            if !p.values.iter().any(|(k, _)| k == c) {
                return Err(EncodeError::UnknownChoice);
            }
            *c as u32
        }
        (Kind::Number, Input::Num(x)) => {
            if !x.is_finite() || p.min.is_some_and(|m| *x < m - 1e-4) || p.max.is_some_and(|m| *x > m + 1e-4) {
                return Err(EncodeError::OutOfRange);
            }
            let exact = (*x - p.offset) / p.scale;
            let r = exact.round();
            if (exact - r).abs() > 0.01 {
                return Err(EncodeError::OffStep);
            }
            if r < 0.0 || r > width_max as f32 {
                return Err(EncodeError::OutOfRange);
            }
            r as u32
        }
        _ => return Err(EncodeError::WrongInput),
    };
    if raw > width_max {
        return Err(EncodeError::OutOfRange);
    }
    let raw = raw as u16;
    Ok(vec![if p.mask_write {
        let m = field_mask(p);
        Request::MaskWrite { addr, reg, and: !m, or: (raw << p.lo) & m }
    } else {
        Request::WriteSingle { addr, reg, value: raw }
    }])
}
