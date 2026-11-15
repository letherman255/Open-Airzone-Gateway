use airzone_core::auth::*;
use airzone_core::frame::Request;
use airzone_core::journal::Journal;

fn rnd(n: u8) -> [u8; 32] {
    [n; 32]
}

#[test]
fn auth_password_roundtrip() {
    let mut a = Auth::default();
    assert!(!a.has_password());
    a.set_password("correct horse", [7; 16]);
    assert!(a.hash().starts_with("pbkdf2$10000$"));
    assert!(!a.hash().contains("correct"));
    assert_eq!(a.check_password("correct horse", 0), Ok(()));
    assert_eq!(a.check_password("wrong", 1), Err(AuthError::Wrong));
    let b = Auth::from_hash(a.hash());
    assert_eq!(b.clone().check_password("correct horse", 0), Ok(()));
}

#[test]
fn auth_lockout() {
    let mut a = Auth::default();
    a.set_password("pw", [1; 16]);
    for i in 0..5 {
        assert_eq!(a.check_password("x", i), Err(AuthError::Wrong));
    }
    assert_eq!(a.check_password("pw", 10), Err(AuthError::Locked { until_ms: 4 + 60_000 }));
    assert_eq!(a.check_password("pw", 60_010), Ok(()));
}

#[test]
fn auth_session_expires() {
    let mut a = Auth::default();
    a.set_password("pw", [1; 16]);
    let s = a.login("pw", 0, rnd(3)).unwrap();
    assert!(a.bearer_ok(&s, 1_000));
    assert!(a.bearer_ok(&s, 1_000 + 29 * 60_000));
    assert!(!a.bearer_ok(&s, 1_000 + 29 * 60_000 + 31 * 60_000));
    assert!(a.login("bad", 0, rnd(4)).is_err());
}

#[test]
fn auth_tokens_hashed() {
    let mut a = Auth::default();
    let t = a.create_token("claude", rnd(9)).unwrap();
    assert_eq!(t.len(), 64);
    assert!(a.bearer_ok(&t, 0));
    assert!(!a.stored_tokens().iter().any(|(_, h)| h == &t));
    assert_eq!(a.token_names(), vec!["claude".to_string()]);
    assert!(a.revoke("claude"));
    assert!(!a.bearer_ok(&t, 0));
    for i in 0..8 {
        a.create_token(&format!("t{i}"), rnd(i)).unwrap();
    }
    assert!(a.create_token("t9", rnd(20)).is_none());
}

#[test]
fn journal_undo_restores_old_value() {
    let mut j = Journal::default();
    j.record("zone.setpoint", 1, 0x0203, Some(210), 215, 0xFFFF, 5);
    let e = &j.list()[0];
    assert_eq!((e.old, e.new), (Some(210), 215));
    assert_eq!(j.undo(0), Some(vec![Request::WriteSingle { addr: 1, reg: 0x0203, value: 210 }]));
    j.record("zone.power", 1, 0x0200, Some(0x0504), 0x0500, 0x0004, 6);
    assert_eq!(j.undo(1), Some(vec![Request::MaskWrite { addr: 1, reg: 0x0200, and: 0xFFFB, or: 0x0004 }]));
    assert_eq!(j.undo(9), None);
    for i in 0..150 {
        j.record("x", 1, 0, Some(0), i, 0xFFFF, i as u64);
    }
    assert_eq!(j.list().len(), 100);
}
