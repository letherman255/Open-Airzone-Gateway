use airzone_core::catalogue::{param, Confidence, Scope, PARAMS};

#[test]
fn lookup_setpoint() {
    let p = param("zone.setpoint").unwrap();
    assert_eq!(p.reg, 3);
    assert_eq!(p.scope, Scope::Zone);
    assert!((p.scale - 0.1).abs() < 1e-6);
}

#[test]
fn all_writable_confirmed() {
    for p in PARAMS {
        if p.writable {
            assert_eq!(p.confidence, Confidence::Confirmed, "{}", p.id);
        }
    }
}

#[test]
fn sorted_by_id() {
    assert!(PARAMS.windows(2).all(|w| w[0].id < w[1].id));
    assert!(PARAMS.len() >= 95);
}
