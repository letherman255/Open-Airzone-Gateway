use airzone_core::catalogue::param;
use airzone_core::frame::Request;
use airzone_core::model::*;

fn p(id: &str) -> &'static airzone_core::catalogue::Param {
    param(id).unwrap_or_else(|| panic!("no param {id}"))
}

const SALON: Target = Target::Zone { addr: 1, zone: 2 };
const SYS: Target = Target::System { addr: 1 };

#[test]
fn decode_setpoint() {
    assert_eq!(decode(p("zone.setpoint"), &[215]), Value::Num(21.5));
}

#[test]
fn decode_mode_heat() {
    assert_eq!(decode(p("zone.mode"), &[0x0504]), Value::Enum(5, "heat"));
    assert_eq!(decode(p("zone.power"), &[0x0504]), Value::Bool(true));
}

#[test]
fn decode_unknown_enum_value() {
    assert_eq!(decode(p("zone.mode"), &[0x0704]), Value::Unknown(7));
}

#[test]
fn decode_ffff_number_is_unknown() {
    assert_eq!(decode(p("zone.temperature"), &[0xFFFF]), Value::Unknown(0xFFFF));
}

#[test]
fn decode_offset_nibble() {
    assert_eq!(decode(p("zone.offset_cool"), &[0x55]), Value::Num(0.0));
    assert_eq!(decode(p("zone.offset_cool"), &[0x57]), Value::Num(1.0));
    assert_eq!(decode(p("zone.offset_heat"), &[0x35]), Value::Num(-1.0));
}

#[test]
fn decode_name() {
    assert_eq!(decode(p("zone.name"), &[0x4375, 0x6973, 0x696E, 0x6500, 0, 0]), Value::Text("Cuisine".into()));
}

#[test]
fn decode_score() {
    assert_eq!(decode(p("iaq.score_t"), &[346]), Value::Num(43.25));
    assert_eq!(decode(p("iaq.score_rh"), &[0x0800 | 338]), Value::Num(42.25));
}

#[test]
fn encode_setpoint() {
    assert_eq!(
        encode(p("zone.setpoint"), &SALON, &Input::Num(21.5), None),
        Ok(vec![Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 }])
    );
}

#[test]
fn encode_power_on() {
    assert_eq!(
        encode(p("zone.power"), &SALON, &Input::Bool(true), None),
        Ok(vec![Request::MaskWrite { addr: 1, reg: 0x0200, and: 0xFFFB, or: 0x0004 }])
    );
}

#[test]
fn encode_outlet_zone() {
    let t = Target::Outlet { addr: 1, outlet: 7 };
    assert_eq!(encode(p("outlet.zone"), &t, &Input::Num(3.0), None), Err(EncodeError::ConfirmRequired));
    assert_eq!(
        encode(p("outlet.zone"), &t, &Input::Num(3.0), Some("OUTLET")),
        Ok(vec![Request::MaskWrite { addr: 1, reg: 0x2F00, and: 0xFF00, or: 0x0003 }])
    );
}

#[test]
fn encode_off_step_rejected() {
    // CO2 thresholds go by 50 ppm: 801 used to be rounded to 800 and reported applied (2026-11-06)
    assert_eq!(encode(p("iaq.co2_gm"), &SYS, &Input::Num(801.0), None), Err(EncodeError::OffStep));
    assert!(encode(p("iaq.co2_gm"), &SYS, &Input::Num(850.0), None).is_ok());
}

#[test]
fn encode_tenths_accepted() {
    // float noise on 0.1 steps must not count as off step
    let z = Target::Zone { addr: 1, zone: 2 };
    assert!(encode(p("zone.setpoint"), &z, &Input::Num(20.5), None).is_ok());
    assert!(encode(p("zone.setpoint"), &z, &Input::Num(21.3), None).is_ok());
    assert_eq!(encode(p("zone.setpoint"), &z, &Input::Num(21.35), None), Err(EncodeError::OffStep));
}

#[test]
fn encode_out_of_range_rejected() {
    assert_eq!(encode(p("system.filter_hours"), &SYS, &Input::Num(50.0), None), Err(EncodeError::OutOfRange));
}

#[test]
fn encode_unknown_choice() {
    assert_eq!(encode(p("zone.mode"), &SALON, &Input::Choice(4), None), Err(EncodeError::UnknownChoice));
}

#[test]
fn encode_matched_refused() {
    assert_eq!(encode(p("zone.temperature"), &SALON, &Input::Num(20.0), None), Err(EncodeError::NotWritable));
}

#[test]
fn encode_wrong_target_or_input() {
    assert_eq!(encode(p("zone.setpoint"), &SYS, &Input::Num(21.0), None), Err(EncodeError::WrongTarget));
    assert_eq!(encode(p("zone.setpoint"), &SALON, &Input::Bool(true), None), Err(EncodeError::WrongInput));
}

#[test]
fn encode_danger_needs_confirm() {
    let d1 = p("system.d1_input");
    assert_eq!(encode(d1, &SYS, &Input::Choice(1), None), Err(EncodeError::ConfirmRequired));
    assert_eq!(encode(d1, &SYS, &Input::Choice(1), Some("yes")), Err(EncodeError::ConfirmRequired));
    assert_eq!(
        encode(d1, &SYS, &Input::Choice(1), Some("D1")),
        Ok(vec![Request::MaskWrite { addr: 1, reg: 0x0048, and: 0xF1FF, or: 0x0200 }])
    );
}

#[test]
fn encode_name_too_long() {
    assert_eq!(
        encode(p("zone.name"), &SALON, &Input::Text("ThirteenChars".into()), None),
        Err(EncodeError::TextTooLong)
    );
}

#[test]
fn encode_name() {
    let r = encode(p("zone.name"), &SALON, &Input::Text("Salon".into()), None).unwrap();
    assert_eq!(
        r,
        vec![
            Request::WriteSingle { addr: 1, reg: 0x020E, value: 0x5361 },
            Request::WriteSingle { addr: 1, reg: 0x020F, value: 0x6C6F },
            Request::WriteSingle { addr: 1, reg: 0x0210, value: 0x6E00 },
            Request::WriteSingle { addr: 1, reg: 0x0211, value: 0 },
            Request::WriteSingle { addr: 1, reg: 0x0212, value: 0 },
            Request::WriteSingle { addr: 1, reg: 0x0213, value: 0 },
        ]
    );
}

#[test]
fn encode_filter_reset_command() {
    assert_eq!(
        encode(p("system.filter_reset"), &SYS, &Input::Trigger, Some("RESET FILTER")),
        Ok(vec![Request::MaskWrite { addr: 1, reg: 0x003C, and: 0xFEFF, or: 0x0100 }])
    );
}

#[test]
fn encode_offset_roundtrip() {
    let r = encode(p("zone.offset_heat"), &SALON, &Input::Num(1.0), None).unwrap();
    assert_eq!(r, vec![Request::MaskWrite { addr: 1, reg: 0x0220, and: 0xFF0F, or: 0x0070 }]);
}

#[test]
fn encode_rejects_invalid_zone_or_outlet() {
    let sp = p("zone.setpoint");
    for zone in [0u8, 33, 127] {
        assert_eq!(encode(sp, &Target::Zone { addr: 1, zone }, &Input::Num(21.0), None), Err(EncodeError::WrongTarget), "zone {zone}");
    }
    let oz = p("outlet.zone");
    for outlet in [0u8, 9, 255] {
        assert_eq!(encode(oz, &Target::Outlet { addr: 1, outlet }, &Input::Num(1.0), Some("OUTLET")), Err(EncodeError::WrongTarget), "outlet {outlet}");
    }
}

#[test]
fn decode_ffff_is_unknown_for_every_kind() {
    assert_eq!(decode(p("iaq.vmc"), &[0xFFFF]), Value::Unknown(0xFFFF));
    assert_eq!(decode(p("iaq.vmin"), &[0xFFFF]), Value::Unknown(0xFFFF));
    assert_eq!(decode(p("zone.mode"), &[0xFFFF]), Value::Unknown(0xFFFF));
}
