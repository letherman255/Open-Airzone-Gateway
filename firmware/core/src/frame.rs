//! Modbus RTU frames as used on the Airzone domotic bus (functions 03, 06, 16 = mask write, 08).
//!
//! Airzone's reply to a 0x16 mask write is not the standard echo: it is
//! `<addr> 16 <reg> 00 00 <and mask>` + CRC.

/// CRC-16/Modbus (poly 0xA001 reflected, init 0xFFFF). Sent low byte first.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}

/// True if the last two bytes are the CRC of the rest.
pub fn crc_ok(frame: &[u8]) -> bool {
    if frame.len() < 4 {
        return false;
    }
    let n = frame.len();
    let c = crc16(&frame[..n - 2]);
    frame[n - 2] == (c & 0xFF) as u8 && frame[n - 1] == (c >> 8) as u8
}

fn finish(mut v: Vec<u8>) -> Vec<u8> {
    let c = crc16(&v);
    v.push((c & 0xFF) as u8);
    v.push((c >> 8) as u8);
    v
}

fn be(b: &[u8], i: usize) -> u16 {
    u16::from_be_bytes([b[i], b[i + 1]])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    TooShort,
    BadCrc,
    WrongAddress,
    WrongFunction,
    BadLength,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Read { addr: u8, reg: u16, count: u16 },
    WriteSingle { addr: u8, reg: u16, value: u16 },
    MaskWrite { addr: u8, reg: u16, and: u16, or: u16 },
    /// Function 08; `data` is everything after the sub-function word.
    Diag { addr: u8, sub: u16, data: Vec<u8> },
}

impl Request {
    pub fn addr(&self) -> u8 {
        match self {
            Request::Read { addr, .. }
            | Request::WriteSingle { addr, .. }
            | Request::MaskWrite { addr, .. }
            | Request::Diag { addr, .. } => *addr,
        }
    }

    pub fn function(&self) -> u8 {
        match self {
            Request::Read { .. } => 0x03,
            Request::WriteSingle { .. } => 0x06,
            Request::MaskWrite { .. } => 0x16,
            Request::Diag { .. } => 0x08,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = vec![self.addr(), self.function()];
        match self {
            Request::Read { reg, count, .. } => {
                v.extend(reg.to_be_bytes());
                v.extend(count.to_be_bytes());
            }
            Request::WriteSingle { reg, value, .. } => {
                v.extend(reg.to_be_bytes());
                v.extend(value.to_be_bytes());
            }
            Request::MaskWrite { reg, and, or, .. } => {
                v.extend(reg.to_be_bytes());
                v.extend(and.to_be_bytes());
                v.extend(or.to_be_bytes());
            }
            Request::Diag { sub, data, .. } => {
                v.extend(sub.to_be_bytes());
                v.extend(data);
            }
        }
        finish(v)
    }

    /// Parse a complete request frame (CRC included).
    pub fn parse(b: &[u8]) -> Result<Request, FrameError> {
        if b.len() < 6 {
            return Err(FrameError::TooShort);
        }
        if !crc_ok(b) {
            return Err(FrameError::BadCrc);
        }
        let addr = b[0];
        let n = b.len();
        match b[1] {
            0x03 if n == 8 => Ok(Request::Read { addr, reg: be(b, 2), count: be(b, 4) }),
            0x06 if n == 8 => Ok(Request::WriteSingle { addr, reg: be(b, 2), value: be(b, 4) }),
            0x16 if n == 10 => Ok(Request::MaskWrite { addr, reg: be(b, 2), and: be(b, 4), or: be(b, 6) }),
            0x08 => Ok(Request::Diag { addr, sub: be(b, 2), data: b[4..n - 2].to_vec() }),
            0x03 | 0x06 | 0x16 => Err(FrameError::BadLength),
            _ => Err(FrameError::WrongFunction),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Registers(Vec<u16>),
    WriteSingle { reg: u16, value: u16 },
    MaskAck { reg: u16, and: u16 },
    Diag { sub: u16, data: Vec<u8> },
    Exception { function: u8, code: u8 },
}

/// Parse the central's reply to `req`.
pub fn parse_response(req: &Request, b: &[u8]) -> Result<Response, FrameError> {
    if b.len() < 5 {
        return Err(FrameError::TooShort);
    }
    if !crc_ok(b) {
        return Err(FrameError::BadCrc);
    }
    if b[0] != req.addr() {
        return Err(FrameError::WrongAddress);
    }
    let f = req.function();
    if b[1] == f | 0x80 {
        return if b.len() == 5 {
            Ok(Response::Exception { function: f, code: b[2] })
        } else {
            Err(FrameError::BadLength)
        };
    }
    if b[1] != f {
        return Err(FrameError::WrongFunction);
    }
    let n = b.len();
    match req {
        Request::Read { count, .. } => {
            let bytes = b[2] as usize;
            if n != 5 + bytes || bytes != 2 * *count as usize {
                return Err(FrameError::BadLength);
            }
            Ok(Response::Registers((0..bytes / 2).map(|i| be(b, 3 + 2 * i)).collect()))
        }
        Request::WriteSingle { .. } => {
            if n != 8 {
                return Err(FrameError::BadLength);
            }
            Ok(Response::WriteSingle { reg: be(b, 2), value: be(b, 4) })
        }
        Request::MaskWrite { .. } => {
            if n != 10 {
                return Err(FrameError::BadLength);
            }
            Ok(Response::MaskAck { reg: be(b, 2), and: be(b, 6) })
        }
        Request::Diag { .. } => {
            if n < 6 {
                return Err(FrameError::BadLength);
            }
            Ok(Response::Diag { sub: be(b, 2), data: b[4..n - 2].to_vec() })
        }
    }
}
