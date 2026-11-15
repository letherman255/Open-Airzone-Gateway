use airzone_core::registry::Registry;
use airzone_core::topology::*;

#[test]
fn zones_present_home() {
    assert_eq!(zones_present(0x003F, 0), vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn zones_present_non_contiguous() {
    assert_eq!(zones_present(0b10011, 0), vec![1, 2, 5]);
}

#[test]
fn zones_present_high() {
    assert_eq!(zones_present(0, 0x0001), vec![17]);
    assert_eq!(zones_present(0x8000, 0x8000), vec![16, 32]);
}

#[test]
fn zone_name_full_and_latin1() {
    let full: Vec<u16> = b"Bureau Maiso".chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    assert_eq!(zone_name(&full), "Bureau Maiso");
    assert_eq!(zone_name(&[0x42E9, 0, 0, 0, 0, 0]), "B\u{FFFD}");
    assert_eq!(zone_name(&[0x4142, 0x0043, 0x4400, 0, 0, 0]), "AB");
}

#[test]
fn iaq_presence() {
    assert!(iaq_present(0x0105));
    assert!(!iaq_present(0));
    assert!(!iaq_present(0xFFFF));
}

fn zone_block(name: &str, word4: u16) -> Vec<u16> {
    let mut v = vec![0u16; 65];
    v[0] = 0x0504;
    v[4] = word4;
    let b = name.as_bytes();
    for i in 0..6 {
        v[14 + i] = u16::from_be_bytes([*b.get(2 * i).unwrap_or(&0), *b.get(2 * i + 1).unwrap_or(&0)]);
    }
    v
}

#[test]
fn build_central_from_registry() {
    let mut r = Registry::default();
    r.update(1, 0x0009, &[0b101], 0);
    r.update(1, 0x000A, &[0], 0);
    r.update(1, 0x2100, &[0xFFFF], 0);
    r.update(1, 0x0100, &zone_block("Kitchen", 0x8806), 0);
    r.update(1, 0x0300, &zone_block("Living", 0x8807), 0);
    let c = build_central(1, &r).unwrap();
    assert_eq!(c.addr, 1);
    assert!(!c.iaq);
    assert_eq!(c.zones.iter().map(|z| (z.n, z.name.as_str(), z.master)).collect::<Vec<_>>(),
               vec![(1, "Kitchen", false), (3, "Living", true)]);
    assert_eq!(c.master_zone(), Some(3));
}

#[test]
fn build_central_missing_zone_block() {
    let mut r = Registry::default();
    r.update(1, 0x0009, &[0b1], 0);
    r.update(1, 0x000A, &[0], 0);
    assert!(build_central(1, &r).is_none());
}
