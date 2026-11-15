use airzone_core::frame::Request;
use airzone_core::model::EncodeError;
use airzone_core::poll::poll_plan;
use airzone_core::registry::Registry;
use airzone_core::rules::*;
use airzone_core::topology::{Central, Topology, Zone};

fn home(master_power_on: bool) -> (Central, Registry) {
    let zones = (1..=6)
        .map(|n| Zone { n, name: format!("Z{n}"), master: n == 2 })
        .collect();
    let mut r = Registry::default();
    r.update(1, 0x0200, &[if master_power_on { 0x0504 } else { 0x0500 }], 0);
    (Central { addr: 1, zones, iaq: true }, r)
}

#[test]
fn mode_heat_master_on() {
    let (c, r) = home(true);
    assert_eq!(set_mode(&c, &r, 5), Ok(vec![Request::MaskWrite { addr: 1, reg: 0x0200, and: 0xF0FF, or: 0x0500 }]));
}

#[test]
fn mode_fan_master_off() {
    let (c, r) = home(false);
    assert_eq!(
        set_mode(&c, &r, 3),
        Ok(vec![
            Request::MaskWrite { addr: 1, reg: 0x0200, and: 0xFFFB, or: 0x0004 },
            Request::MaskWrite { addr: 1, reg: 0x0200, and: 0xF0FF, or: 0x0300 },
        ])
    );
}

#[test]
fn mode_cool_code_is_9() {
    let (c, r) = home(true);
    assert_eq!(set_mode(&c, &r, 9), Ok(vec![Request::MaskWrite { addr: 1, reg: 0x0200, and: 0xF0FF, or: 0x0900 }]));
}

#[test]
fn mode_unknown_or_no_master() {
    let (mut c, r) = home(true);
    assert_eq!(set_mode(&c, &r, 4), Err(EncodeError::UnknownChoice));
    c.zones.iter_mut().for_each(|z| z.master = false);
    assert_eq!(set_mode(&c, &r, 5), Err(EncodeError::WrongTarget));
}

#[test]
fn calendar_friday_november() {
    assert_eq!(
        calendar_writes(1, &[0, 1, 2, 3, 4], &[5, 6, 7], 4, 10),
        [
            Request::MaskWrite { addr: 1, reg: 0x2101, and: 0xDFFF, or: 0x2000 },
            Request::MaskWrite { addr: 1, reg: 0x2101, and: 0x7FFF, or: 0 },
        ]
    );
}

#[test]
fn calendar_all_months() {
    let months: Vec<u8> = (0..12).collect();
    let w = calendar_writes(1, &[5, 6], &months, 4, 10);
    assert_eq!(w[0], Request::MaskWrite { addr: 1, reg: 0x2101, and: 0xDFFF, or: 0 });
    assert_eq!(w[1], Request::MaskWrite { addr: 1, reg: 0x2101, and: 0x7FFF, or: 0x8000 });
}

#[test]
fn identity_default_name() {
    assert_eq!(Identity::default().name, *b"OPENAZWS01");
}

#[test]
fn identity_order() {
    let id = Identity { mac: [0xB0, 0x65, 0x3A, 0x42, 0xE9, 0x2C], ip: [192, 168, 7, 128], gateway_ip: [192, 168, 7, 1], ..Default::default() };
    let r = identity_requests(1, &id);
    match &r[0] {
        Request::Diag { addr: 1, sub: 3, data } => assert_eq!(&data[data.len() - 10..], b"OPENAZWS01"),
        other => panic!("{other:?}"),
    }
    assert!(r.contains(&Request::WriteSingle { addr: 1, reg: 0x004E, value: 0xB065 }));
    assert!(r.contains(&Request::WriteSingle { addr: 1, reg: 0x0050, value: 0xE92C }));
    assert!(r.contains(&Request::WriteSingle { addr: 1, reg: 0x002E, value: 0xC0A8 }));
    assert!(r.contains(&Request::WriteSingle { addr: 1, reg: 0x002F, value: 0x0780 }));
    let n = r.len();
    assert_eq!(r[n - 2], Request::MaskWrite { addr: 1, reg: 0x008B, and: 0xF8FF, or: 0x0100 });
    assert_eq!(r[n - 1], Request::MaskWrite { addr: 1, reg: 0x008B, and: 0xF8FF, or: 0x0500 });
}

#[test]
fn heartbeat_frame() {
    assert_eq!(heartbeat(1), Request::MaskWrite { addr: 1, reg: 0x0001, and: 0xFFEF, or: 0x0010 });
}

#[test]
fn poll_plan_home() {
    let (c, _) = home(true);
    let plan = poll_plan(&Topology { centrals: vec![c] });
    assert_eq!(plan.len(), 17);
    assert_eq!(plan[0], Request::Read { addr: 1, reg: 0x0000, count: 80 });
    assert_eq!(plan[1], Request::Read { addr: 1, reg: 0x0050, count: 80 });
    assert_eq!(plan[2], Request::Read { addr: 1, reg: 0x2100, count: 33 });
    assert_eq!(plan[3], Request::Read { addr: 1, reg: 0x2900, count: 12 });
    assert_eq!(plan[10], Request::Read { addr: 1, reg: 0x3000, count: 12 });
    assert_eq!(plan[16], Request::Read { addr: 1, reg: 0x0600, count: 65 });
}

#[test]
fn poll_plan_no_iaq() {
    let (mut c, _) = home(true);
    c.iaq = false;
    let plan = poll_plan(&Topology { centrals: vec![c] });
    assert_eq!(plan.len(), 16);
    assert!(!plan.contains(&Request::Read { addr: 1, reg: 0x2100, count: 33 }));
}
