use airzone_core::frame::{crc16, parse_response, Request, Response};
use airzone_core::poll::poll_plan;
use airzone_core::registry::Registry;
use airzone_core::topology::{build_central, Topology};
use airzone_sim::SimCentral;

const HOME: &str = include_str!("../fixtures/home_2026-11-05.json");

fn ask(c: &mut SimCentral, r: &Request) -> Response {
    let reply = c.handle(&r.encode()).expect("no answer");
    parse_response(r, &reply).unwrap()
}

fn regs(c: &mut SimCentral, addr: u8, reg: u16, count: u16) -> Vec<u16> {
    match ask(c, &Request::Read { addr, reg, count }) {
        Response::Registers(v) => v,
        other => panic!("{other:?}"),
    }
}

#[test]
fn read_zone_block_from_fixture() {
    let mut c = SimCentral::from_fixture(HOME, 1);
    let v = regs(&mut c, 1, 0x0200, 65);
    assert_eq!(v[0], 0x0504);
    assert_eq!(v[4], 0x8807);
}

#[test]
fn mask_write_reply_format() {
    let mut c = SimCentral::from_fixture(HOME, 1);
    let req = Request::MaskWrite { addr: 1, reg: 0x0200, and: 0xFFFB, or: 0 };
    let reply = c.handle(&req.encode()).unwrap();
    let crc = crc16(&[0x01, 0x16, 0x02, 0x00, 0x00, 0x00, 0xFF, 0xFB]);
    assert_eq!(reply, vec![0x01, 0x16, 0x02, 0x00, 0x00, 0x00, 0xFF, 0xFB, (crc & 0xFF) as u8, (crc >> 8) as u8]);
    assert_eq!(c.reg(0x0200) & 0x0004, 0);
}

#[test]
fn mode_copies_to_all_zones() {
    let mut c = SimCentral::from_fixture(HOME, 1);
    let before: Vec<u16> = (1..=6).map(|z| c.reg((z << 8) + 3)).collect();
    ask(&mut c, &Request::MaskWrite { addr: 1, reg: 0x0300, and: 0xF0FF, or: 0x0300 });
    for z in 1..=6u16 {
        assert_eq!((c.reg(z << 8) >> 8) & 0xF, 3, "zone {z} mode");
        assert_eq!(c.reg((z << 8) + 3), 150, "zone {z} fan setpoint");
    }
    ask(&mut c, &Request::MaskWrite { addr: 1, reg: 0x0200, and: 0xF0FF, or: 0x0500 });
    let after: Vec<u16> = (1..=6).map(|z| c.reg((z << 8) + 3)).collect();
    assert_eq!(after, before);
    assert_eq!(c.reg(0x0000) & 0xF, 5);
}

#[test]
fn limit_copies() {
    let mut c = SimCentral::from_fixture(HOME, 1);
    ask(&mut c, &Request::WriteSingle { addr: 1, reg: 0x001A, value: 250 });
    for z in 1..=6u16 {
        assert_eq!(c.reg((z << 8) + 2), 250);
    }
}

#[test]
fn heartbeat_cleared_on_read() {
    let mut c = SimCentral::from_fixture(HOME, 1);
    ask(&mut c, &Request::MaskWrite { addr: 1, reg: 0x0001, and: 0xFFEF, or: 0x0010 });
    assert_ne!(c.reg(0x0001) & 0x10, 0);
    regs(&mut c, 1, 0x0000, 80);
    assert_eq!(c.reg(0x0001) & 0x10, 0);
}

#[test]
fn unknown_address_silent() {
    let mut c = SimCentral::from_fixture(HOME, 1);
    assert!(c.handle(&Request::Read { addr: 2, reg: 0x0030, count: 1 }.encode()).is_none());
}

#[test]
fn outside_block_exception() {
    let mut c = SimCentral::from_fixture(HOME, 1);
    assert_eq!(ask(&mut c, &Request::Read { addr: 1, reg: 0x7000, count: 1 }), Response::Exception { function: 3, code: 2 });
}

#[test]
fn refused_write_exception() {
    let mut c = SimCentral::from_fixture(HOME, 1);
    c.refuse.push(0x0203);
    assert_eq!(
        ask(&mut c, &Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 }),
        Response::Exception { function: 6, code: 3 }
    );
}

#[test]
fn builder_non_contiguous() {
    let mut c = SimCentral::builder().zones(&[1, 2, 5]).iaq(false).build();
    assert_eq!(c.reg(0x0009), 0b10011);
    let v = regs(&mut c, 1, 0x0500, 65);
    assert_eq!(airzone_core::topology::zone_name(&v[14..20]), "Zone 5");
    assert_eq!(regs(&mut c, 1, 0x2100, 1), vec![0xFFFF]);
}

#[test]
fn build_home_central() {
    let mut c = SimCentral::from_fixture(HOME, 1);
    let mut r = Registry::default();
    for req in [Request::Read { addr: 1, reg: 0, count: 80 }, Request::Read { addr: 1, reg: 0x2100, count: 33 }] {
        if let Request::Read { reg, count, .. } = req {
            r.update(1, reg, &regs(&mut c, 1, reg, count), 0);
        }
    }
    for z in 1..=6u16 {
        r.update(1, z << 8, &regs(&mut c, 1, z << 8, 65), 0);
    }
    let central = build_central(1, &r).unwrap();
    let names: Vec<&str> = central.zones.iter().map(|z| z.name.as_str()).collect();
    assert_eq!(names, ["Cuisine", "Salon", "Musculation", "Bureau L.", "Bureau M.", "Chambre"]);
    assert_eq!(central.master_zone(), Some(2));
    assert!(central.iaq);
    assert_eq!(poll_plan(&Topology { centrals: vec![central] }).len(), 17);
}
