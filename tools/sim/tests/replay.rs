use airzone_core::frame::Request;
use airzone_core::listen::{ListenEvent, Listener};
use airzone_core::model::Value;
use airzone_core::topology::build_central;

/// Capture lines `<iso ts> <mono> <len> <hex…>`; other lines (`# alive …`) are skipped.
fn chunks(text: &str) -> Vec<Vec<u8>> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let _ts = it.next()?;
            let mono = it.next()?;
            if mono.starts_with('#') {
                return None;
            }
            let len: usize = it.next()?.parse().ok()?;
            let bytes: Vec<u8> = it.map(|b| u8::from_str_radix(b, 16).unwrap()).collect();
            (bytes.len() == len).then_some(bytes)
        })
        .collect()
}

fn replay(text: &str) -> (Listener, Vec<ListenEvent>) {
    let mut l = Listener::default();
    let mut events = Vec::new();
    for (i, c) in chunks(text).iter().enumerate() {
        events.extend(l.feed(c, 100_000, i as u64));
    }
    (l, events)
}

const STARTUP: &str = include_str!("../fixtures/capture_startup.log");
const ZONES: &str = include_str!("../fixtures/capture_zone_tests.log");
const OUTLET: &str = include_str!("../fixtures/capture_outlet.log");

#[test]
fn replay_startup_topology() {
    let (l, _) = replay(STARTUP);
    let c = build_central(1, l.registry()).expect("central known after start-up");
    assert_eq!(c.zones.len(), 6);
    assert_eq!(c.master_zone(), Some(2));
    assert_eq!(c.zones[0].name, "Cuisine");
}

#[test]
fn replay_identity_seen() {
    let (_, ev) = replay(STARTUP);
    assert!(ev.iter().any(|e| matches!(e,
        ListenEvent::Write { request: Request::WriteSingle { addr: 1, reg: 0x0059, value: 0x0423 }, .. })));
}

#[test]
fn replay_no_unpaired_in_startup() {
    let (_, ev) = replay(STARTUP);
    let unpaired: Vec<_> = ev.iter().filter(|e| matches!(e, ListenEvent::Unpaired(_))).collect();
    assert!(unpaired.is_empty(), "{unpaired:?}");
}

#[test]
fn replay_outlet_stage_write() {
    let (_, ev) = replay(OUTLET);
    let first = ev.iter().find_map(|e| match e {
        ListenEvent::Write { request: Request::MaskWrite { reg: 0x2F01, .. }, params } => Some(params.clone()),
        _ => None,
    });
    assert_eq!(first.unwrap(), vec![("outlet.cool_stage", Some(7), Value::Bool(false))]);
}

#[test]
fn replay_zone_tests_decodes_basic_mode() {
    let (_, ev) = replay(ZONES);
    let writes: Vec<_> = ev.iter().filter_map(|e| match e {
        ListenEvent::Write { params, .. } => Some(params.clone()),
        _ => None,
    }).flatten().collect();
    assert!(writes.iter().any(|(id, z, v)| *id == "zone.basic_mode" && *z == Some(1) && *v == Value::Bool(true)), "{writes:?}");
}

#[test]
fn listener_never_transmits() {
    let src = include_str!("../../../firmware/core/src/listen.rs");
    assert!(!src.contains("Bus"));
    assert!(!src.contains("transact"));
}

#[test]
fn reply_with_chance_crc_at_offset_8_is_not_cut() {
    use airzone_core::frame::crc16;
    let req = Request::Read { addr: 1, reg: 0x0000, count: 80 }.encode();
    let mut reply = vec![0x01, 0x03, 0xA0, 0x12, 0x34, 0x56];
    let c = crc16(&reply);
    reply.extend([(c & 0xFF) as u8, (c >> 8) as u8]);
    while reply.len() < 3 + 160 {
        reply.push(0x11);
    }
    let c = crc16(&reply);
    reply.extend([(c & 0xFF) as u8, (c >> 8) as u8]);
    let mut chunk = req;
    chunk.extend(&reply);
    let mut l = Listener::default();
    let ev = l.feed(&chunk, 100_000, 0);
    assert!(ev.iter().all(|e| !matches!(e, ListenEvent::Unpaired(_))), "{ev:?}");
    assert_eq!(l.registry().get(1, 0x0000), Some(0x1234));
    assert_eq!(l.registry().get(1, 79), Some(0x1111));
}
