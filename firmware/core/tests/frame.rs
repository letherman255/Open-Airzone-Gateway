mod common;
use airzone_core::frame::*;
use common::{fixture, hex};

fn with_crc(mut v: Vec<u8>) -> Vec<u8> {
    let c = crc16(&v);
    v.push((c & 0xFF) as u8);
    v.push((c >> 8) as u8);
    v
}

#[test]
fn crc_known() {
    for name in ["setpoint_write", "mask_req", "mask_ack", "zone_read_reply", "ident", "broadcast_ident", "presence_reply"] {
        let f = fixture(name);
        let n = f.len();
        let c = crc16(&f[..n - 2]);
        assert_eq!([(c & 0xFF) as u8, (c >> 8) as u8], [f[n - 2], f[n - 1]], "{name}");
    }
}

#[test]
fn encode_mask_write() {
    let r = Request::MaskWrite { addr: 1, reg: 0x0200, and: 0xFFFB, or: 0x0004 };
    assert_eq!(r.encode(), with_crc(hex("01 16 02 00 FF FB 00 04")));
}

#[test]
fn encode_and_parse_requests_roundtrip() {
    for name in ["setpoint_write", "mask_req", "zone_read_req", "ident", "presence_req"] {
        let f = fixture(name);
        let r = Request::parse(&f).unwrap();
        assert_eq!(r.encode(), f, "{name}");
    }
    assert_eq!(
        Request::parse(&fixture("setpoint_write")).unwrap(),
        Request::WriteSingle { addr: 1, reg: 0x0203, value: 0x00D7 }
    );
}

#[test]
fn parse_mask_ack() {
    let req = Request::parse(&fixture("mask_req2")).unwrap();
    let r = parse_response(&req, &fixture("mask_ack2")).unwrap();
    assert_eq!(r, Response::MaskAck { reg: 0x2F01, and: 0xFFDF });
}

#[test]
fn parse_read_reply() {
    let req = Request::Read { addr: 1, reg: 0x0100, count: 65 };
    match parse_response(&req, &fixture("zone_read_reply")).unwrap() {
        Response::Registers(v) => {
            assert_eq!(v.len(), 65);
            assert_eq!(v[0], 0x0504);
            assert_eq!(v[14], 0x4375);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn parse_diag_echo() {
    let f = fixture("ident");
    let req = Request::parse(&f).unwrap();
    match parse_response(&req, &f).unwrap() {
        Response::Diag { sub, data } => {
            assert_eq!(sub, 3);
            assert_eq!(&data[data.len() - 10..], b"AZWSWFE92C");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn exception_reply_is_rejected() {
    let req = Request::MaskWrite { addr: 1, reg: 0x0200, and: 0xFFFB, or: 4 };
    assert_eq!(
        parse_response(&req, &with_crc(hex("01 96 02"))).unwrap(),
        Response::Exception { function: 0x16, code: 2 }
    );
    let req = Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 };
    assert_eq!(
        parse_response(&req, &with_crc(hex("01 86 03"))).unwrap(),
        Response::Exception { function: 0x06, code: 3 }
    );
}

#[test]
fn garbled_reply_errors() {
    let req = Request::Read { addr: 1, reg: 0x0030, count: 1 };
    assert_eq!(parse_response(&req, &hex("01 03")), Err(FrameError::TooShort));
    assert_eq!(parse_response(&req, &hex("01 03 02 A0 79 01 A7")), Err(FrameError::BadCrc));
    assert_eq!(parse_response(&req, &with_crc(hex("02 03 02 A0 79"))), Err(FrameError::WrongAddress));
    assert_eq!(parse_response(&req, &with_crc(hex("01 06 02 A0 79"))), Err(FrameError::WrongFunction));
    assert_eq!(parse_response(&req, &with_crc(hex("01 03 04 A0 79"))), Err(FrameError::BadLength));
    assert!(Request::parse(&hex("01")).is_err());
}
