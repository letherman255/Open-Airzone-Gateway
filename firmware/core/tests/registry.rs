use airzone_core::registry::{Change, Registry};

#[test]
fn update_reports_changes() {
    let mut r = Registry::default();
    let first = r.update(1, 0x0200, &[0x0504, 0x00BE], 10);
    assert_eq!(first.len(), 2);
    assert!(first.iter().all(|c| c.old.is_none()));
    let second = r.update(1, 0x0200, &[0x0500, 0x00BE], 20);
    assert_eq!(second, vec![Change { addr: 1, reg: 0x0200, old: Some(0x0504), new: 0x0500 }]);
    assert_eq!(r.get(1, 0x0200), Some(0x0500));
    assert_eq!(r.get(2, 0x0200), None);
    assert_eq!(r.changed_at(1, 0x0200), Some(20));
    assert_eq!(r.changed_at(1, 0x0201), Some(10));
    assert_eq!(r.get_range(1, 0x0200, 2), Some(vec![0x0500, 0x00BE]));
    assert_eq!(r.get_range(1, 0x0200, 3), None);
}

#[test]
fn apply_changes_mirrors_the_source() {
    // the ESP keeps a mirror of the master engine's registry fed with changes only (a full copy is
    // ~30 KB and the board has ~50 KB free)
    let mut src = Registry::default();
    let mut mirror = Registry::default();
    let c1 = src.update(1, 0x0200, &[200, 1, 2], 10);
    mirror.apply(&c1, 10);
    let c2 = src.update(1, 0x0200, &[205, 1, 3], 20);
    mirror.apply(&c2, 20);
    assert_eq!(mirror.dump(1), src.dump(1));
    assert_eq!(mirror.changed_at(1, 0x0200), Some(20));
    assert_eq!(mirror.changed_at(1, 0x0201), Some(10));
}
