mod common;
use airzone_core::deframe::Deframer;
use common::{fixture, hex};

#[test]
fn merged_request_and_reply() {
    let mut d = Deframer::default();
    let out = d.push(&hex("01 16 2F 01 FF EF 00 10 CC 74 01 16 2F 01 00 00 FF EF CC 25"), 100_000);
    assert_eq!(out, vec![fixture("mask_req"), fixture("mask_ack")]);
}

#[test]
fn gap_splits_frames() {
    let mut d = Deframer::default();
    let mut out = d.push(&fixture("presence_req"), 100_000);
    out.extend(d.push(&fixture("presence_reply"), 3_000));
    assert_eq!(out, vec![fixture("presence_req"), fixture("presence_reply")]);
}

#[test]
fn read_request_then_reply_in_pieces() {
    let mut d = Deframer::default();
    let reply = fixture("zone_read_reply");
    let mut out = d.push(&fixture("zone_read_req"), 100_000);
    out.extend(d.push(&reply[..60], 2_000));
    out.extend(d.push(&reply[60..], 2_000));
    assert_eq!(out, vec![fixture("zone_read_req"), reply]);
}

#[test]
fn diag_frames_found_by_crc() {
    let mut d = Deframer::default();
    let mut f = fixture("ident");
    f.extend(fixture("ident"));
    let out = d.push(&f, 100_000);
    assert_eq!(out, vec![fixture("ident"), fixture("ident")]);
    assert_eq!(d.push(&fixture("broadcast_ident"), 100_000), vec![fixture("broadcast_ident")]);
}

// 2026-11-05 22:00:44: system reply split in two chunks; its first 17 bytes end with a chance-valid CRC.
const P1: &str = "01 03 A0 01 02 04 10 00 00 48 20 81 B4 28 22 00 3F 01 08 FE D4 00 3F 00 00 00 00 44 0A 00 00 00 00 00 00 00 00 \
83 77 00 B4 00 D2 12 0C 00 00 00 00 00 00 00 00 00 BE 00 FA 00 08 00 08 00 8D 00 00 00 00 FF FF FF FF FF FF FF \
FF 08 C0 35 65 00 CD 10 00 00 0A 7F 04 FF FF FF FF FF FF FF FF C0 A8 07 D1 A0 79 00 00 62 00 FF FF FF FF FF FF \
FF FF 41 00 40 00 FF FF 12";
const P2: &str = "00 FF FF 26 84 64 0A 00 00 FF FF FF FF 00 00 00 00 00 00 00 06 00 01 00 03 0B 05 00 A1 04 30 04 53 00 00 C0 \
A8 07 01 B0 65 3A 42 85 3C";

#[test]
fn split_reply_chance_crc() {
    let mut d = Deframer::default();
    let mut out = d.push(&hex(P1), 100_000);
    out.extend(d.push(&hex(P2), 9_000));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].len(), 165);
}

#[test]
fn garbage_is_skipped() {
    let mut d = Deframer::default();
    let mut f = hex("FF 00");
    f.extend(fixture("presence_req"));
    assert_eq!(d.push(&f, 100_000), vec![fixture("presence_req")]);
}
