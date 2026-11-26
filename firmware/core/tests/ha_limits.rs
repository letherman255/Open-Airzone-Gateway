use airzone_core::ha::{setpoint_limits, COOL_SETPOINT_MAX, HEAT_SETPOINT_MIN};
use airzone_core::registry::Registry;
use airzone_core::topology::Zone;

/// Zone 1 of central 1 with word +0 (mode bits 8-11, power bit 2), +1 = 19.0 (cool min), +2 = 25.0 (heat max).
fn reg_with(word0: u16) -> Registry {
    let mut r = Registry::default();
    let ch = r.update(1, 0x0100, &[word0, 190, 250], 0);
    r.apply(&ch, 0);
    r
}

fn zone() -> Zone {
    Zone { n: 1, name: "Salon".into(), master: true }
}

#[test]
fn heat_ignores_cool_minimum() {
    // heat (5), on: 17 °C must be allowed (Tempo red-day setback, 2026-11-26)
    let (min, max) = setpoint_limits(&zone(), 1, &reg_with(0x0504));
    assert_eq!(min, HEAT_SETPOINT_MIN);
    assert_eq!(max, 25.0);
}

#[test]
fn cool_uses_cool_minimum() {
    let (min, max) = setpoint_limits(&zone(), 1, &reg_with(0x0904));
    assert_eq!(min, 19.0);
    assert_eq!(max, COOL_SETPOINT_MAX);
}

#[test]
fn zone_off_keeps_global_mode_limits() {
    let (min, _) = setpoint_limits(&zone(), 1, &reg_with(0x0500));
    assert_eq!(min, HEAT_SETPOINT_MIN);
}

#[test]
fn unknown_registers_fall_back_to_full_range() {
    let r = Registry::default();
    assert_eq!(setpoint_limits(&zone(), 1, &r), (HEAT_SETPOINT_MIN, COOL_SETPOINT_MAX));
}
