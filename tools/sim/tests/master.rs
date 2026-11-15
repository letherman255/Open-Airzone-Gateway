use airzone_core::bus::Bus;
use airzone_core::catalogue::param;
use airzone_core::frame::Request;
use airzone_core::master::*;
use airzone_core::model::{encode, Input, Target};
use airzone_core::rules::{heartbeat, Identity};
use airzone_sim::{SimBus, SimCentral};

const HOME: &str = include_str!("../fixtures/home_2026-11-05.json");

fn cfg() -> Config {
    Config {
        identity: Identity { mac: [0x24, 0x0A, 0xC4, 0x01, 0x02, 0x03], ip: [192, 168, 7, 128], ..Default::default() },
        ..Default::default()
    }
}

fn home() -> Master<SimBus> {
    Master::new(SimBus::new(vec![SimCentral::from_fixture(HOME, 1)]), cfg())
}

#[test]
fn start_discovers_home() {
    let mut m = home();
    let t = m.start(0).unwrap().clone();
    assert_eq!(t.centrals.len(), 1);
    let c = &t.centrals[0];
    assert_eq!(c.zones.len(), 6);
    assert_eq!(c.zones[1].name, "Salon");
    assert!(c.zones[1].master);
    assert!(c.iaq);
}

#[test]
fn discovery_probes_like_the_stock_webserver() {
    // the real central gave no answer to a 1-register read of 0x0030 (2026-11-06); the stock webserver
    // only ever reads the system block 0x0000 x 0x50 at a central address
    let mut m = home();
    m.start(0).unwrap();
    let sent = &m.bus().sent;
    assert_eq!(sent[0], Request::Read { addr: 1, reg: 0x0000, count: 0x50 }.encode());
    let first_to_2 = sent.iter().find(|f| f[0] == 2).unwrap();
    assert_eq!(*first_to_2, Request::Read { addr: 2, reg: 0x0000, count: 0x50 }.encode());
    assert!(!sent.iter().any(|f| f[1] == 0x03 && f[2..4] == [0x00, 0x30]), "no read of 0x0030");
}

#[test]
fn default_timeout_covers_the_longest_reply() {
    // an 80-register reply is 165 bytes = 95 ms on the wire at 19200 8E1, plus the central's latency
    // (up to ~70 ms seen in the capture)
    assert!(Config::default().timeout_ms >= 300);
}

#[test]
fn sim_discovers_non_contiguous_zones() {
    let bus = SimBus::new(vec![SimCentral::builder().zones(&[1, 2, 5]).build()]);
    let mut m = Master::new(bus, cfg());
    let t = m.start(0).unwrap();
    assert_eq!(t.centrals[0].zones.iter().map(|z| z.n).collect::<Vec<_>>(), vec![1, 2, 5]);
}

#[test]
fn two_centrals() {
    let bus = SimBus::new(vec![
        SimCentral::builder().zones(&[1, 2]).build(),
        SimCentral::builder().addr(2).zones(&[1]).build(),
    ]);
    let mut m = Master::new(bus, cfg());
    let t = m.start(0).unwrap();
    assert_eq!(t.centrals.iter().map(|c| c.addr).collect::<Vec<_>>(), vec![1, 2]);
}

#[test]
fn no_iaq() {
    let bus = SimBus::new(vec![SimCentral::builder().zones(&[1]).iaq(false).build()]);
    let mut m = Master::new(bus, cfg());
    m.start(0).unwrap();
    assert!(!m.topology().centrals[0].iaq);
    assert!(!m.plan().contains(&Request::Read { addr: 1, reg: 0x2100, count: 33 }));
}

#[test]
fn foreign_master_refuses() {
    let mut bus = SimBus::new(vec![SimCentral::from_fixture(HOME, 1)]);
    bus.faults.foreign_master = true;
    let mut m = Master::new(bus, cfg());
    assert_eq!(m.start(0).err(), Some(StartError::ForeignMaster));
    assert!(m.bus().sent.is_empty());
    assert!(m.bus().listened_ms >= 5000);
}

#[test]
fn no_central() {
    let mut m = Master::new(SimBus::new(vec![]), cfg());
    assert_eq!(m.start(0).err(), Some(StartError::NoCentral));
}

#[test]
fn setpoint_applied_after_reread() {
    let mut m = home();
    m.start(0).unwrap();
    let w = encode(param("zone.setpoint").unwrap(), &Target::Zone { addr: 1, zone: 2 }, &Input::Num(21.5), None).unwrap();
    let id = m.submit(w);
    let r = m.cycle(1000);
    assert_eq!(r.writes, vec![(id, WriteResult::Applied(vec![215]))]);
    assert!(r.changes.iter().any(|c| c.reg == 0x0203 && c.new == 215));
    assert_eq!(m.registry().get(1, 0x0203), Some(215));
}

#[test]
fn master_write_rejected_by_central() {
    let mut m = home();
    m.start(0).unwrap();
    m.bus_mut().central_mut(1).refuse.push(0x0203);
    let id = m.submit(vec![Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 }]);
    let r = m.cycle(1000);
    assert_eq!(r.writes, vec![(id, WriteResult::Rejected { code: 3, sent: 0 })]);
}

#[test]
fn ignored_write_resent_once_when_not_applied() {
    // the real central ignores roughly one write in ten (no echo, value unchanged): writes are absolute, so
    // re-sending once after a re-read that shows the old value is safe (2026-11-06)
    let mut m = home();
    m.start(0).unwrap();
    m.bus_mut().faults.ignore_next_writes = 1;
    let before = m.bus().sent.len();
    let write = Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 };
    let id = m.submit(vec![write.clone()]);
    let r = m.cycle(1000);
    assert_eq!(r.writes, vec![(id, WriteResult::Applied(vec![215]))]);
    assert_eq!(m.bus().sent[before..].iter().filter(|f| **f == write.encode()).count(), 2);
}

#[test]
fn write_resent_at_most_once() {
    let mut m = home();
    m.start(0).unwrap();
    m.bus_mut().faults.drop_writes = true;
    let before = m.bus().sent.len();
    let write = Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 };
    let id = m.submit(vec![write.clone()]);
    let r = m.cycle(1000);
    assert_eq!(r.writes, vec![(id, WriteResult::NoAnswer { sent: 0 })]);
    let sent = &m.bus().sent[before..];
    assert_eq!(sent.iter().filter(|f| **f == write.encode()).count(), 2);
}

#[test]
fn apply_queued_writes_without_polling() {
    // a click in Home Assistant must not wait for the 2.3 s poll cycle: writes go out alone
    let mut m = home();
    m.start(0).unwrap();
    let before = m.bus().sent.len();
    let write = Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 };
    let id = m.submit(vec![write.clone()]);
    let r = m.apply_queued(1000);
    assert_eq!(r.writes, vec![(id, WriteResult::Applied(vec![215]))]);
    let sent = &m.bus().sent[before..];
    assert_eq!(sent[0], write.encode());
    // the write plus the re-read of its zone block, nothing else
    assert_eq!(sent.len(), 2, "{sent:02X?}");
    assert!(r.changes.iter().any(|c| c.reg == 0x0203 && c.new == 215));
    // nothing queued: nothing sent
    let n = m.bus().sent.len();
    assert!(m.apply_queued(2000).writes.is_empty());
    assert_eq!(m.bus().sent.len(), n);
}

#[test]
fn poll_step_spreads_the_cycle_one_block_at_a_time() {
    // a full cycle keeps the bus busy ~2 s: done step by step, a command waits one block read at most
    let mut m = home();
    m.start(0).unwrap();
    let plan = m.plan().to_vec();
    let before = m.bus().sent.len();
    let mut done = 0;
    for i in 0..plan.len() {
        let r = m.poll_step(1000 + i as u64);
        if r.round_done {
            done += 1;
        }
    }
    assert_eq!(done, 1, "one round per plan length");
    let reads: Vec<_> = m.bus().sent[before..].iter().filter(|f| f[1] == 0x03).cloned().collect();
    for p in &plan {
        assert_eq!(reads.iter().filter(|f| **f == p.encode()).count(), 1, "{p:?}");
    }
    // a queued write goes out before the next block read
    let write = Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 };
    let id = m.submit(vec![write.clone()]);
    let n = m.bus().sent.len();
    let r = m.poll_step(5000);
    assert_eq!(m.bus().sent[n], write.encode());
    assert_eq!(r.writes, vec![(id, WriteResult::Applied(vec![215]))]);
}

#[test]
fn read_failures_counted() {
    // health counter for Home Assistant: poll reads that never got a valid answer
    let mut m = home();
    m.start(0).unwrap();
    assert_eq!(m.read_failures(), 0);
    m.bus_mut().faults.drop_reads = true;
    m.poll_step(1000);
    assert_eq!(m.read_failures(), 1);
}

#[test]
fn cycle_reports_periodic_rediscovery() {
    // discovery reads are not in `changes`: the ESP mirror needs a full copy after it
    let mut m = home();
    m.start(0).unwrap();
    assert!(!m.cycle(1_000).rediscovered);
    let later = m.config().rediscover_ms + 1_000;
    assert!(m.cycle(later).rediscovered);
}

#[test]
fn lost_echo_but_value_applied_is_applied() {
    // seen on the real bus (2026-11-06): the central applies a write but its echo never arrives
    let mut m = home();
    m.start(0).unwrap();
    m.bus_mut().faults.lose_write_replies = true;
    let before = m.bus().sent.len();
    let write = Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 };
    let id = m.submit(vec![write.clone()]);
    let r = m.cycle(1000);
    assert_eq!(r.writes, vec![(id, WriteResult::Applied(vec![215]))]);
    // still never retried
    assert_eq!(m.bus().sent[before..].iter().filter(|f| **f == write.encode()).count(), 1);
}

#[test]
fn lost_echo_with_value_in_place_continues_the_batch() {
    // a write verified in place by the re-read counts as done: the rest of the batch goes on
    let mut m = home();
    m.start(0).unwrap();
    m.bus_mut().faults.lose_write_replies = true;
    let w1 = Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 };
    let w2 = Request::WriteSingle { addr: 1, reg: 0x0303, value: 215 };
    let id = m.submit(vec![w1.clone(), w2.clone()]);
    let r = m.cycle(1000);
    assert_eq!(r.writes, vec![(id, WriteResult::Applied(vec![215, 215]))]);
    // each sent once: no resend when the value is already there
    assert_eq!(m.bus().sent.iter().filter(|f| **f == w1.encode()).count(), 1);
    assert_eq!(m.bus().sent.iter().filter(|f| **f == w2.encode()).count(), 1);
}

#[test]
fn unapplied_write_stops_the_batch() {
    // the following requests of the batch are not sent when a write never took
    let mut m = home();
    m.start(0).unwrap();
    m.bus_mut().faults.drop_writes = true;
    let w1 = Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 };
    let w2 = Request::WriteSingle { addr: 1, reg: 0x0303, value: 215 };
    let id = m.submit(vec![w1, w2.clone()]);
    let r = m.cycle(1000);
    assert_eq!(r.writes, vec![(id, WriteResult::NoAnswer { sent: 0 })]);
    assert!(!m.bus().sent.contains(&w2.encode()));
}

#[test]
fn reads_retried_on_bad_crc() {
    let mut m = home();
    m.start(0).unwrap();
    m.bus_mut().faults.bad_crc_every = Some(5);
    let r = m.cycle(1000);
    assert!(r.unavailable.is_empty());
}

#[test]
fn central_unavailable_after_3_cycles() {
    let mut m = home();
    m.start(0).unwrap();
    assert!(m.cycle(1000).unavailable.is_empty());
    m.bus_mut().faults.silent.push(1);
    assert!(m.cycle(4000).unavailable.is_empty());
    assert!(m.cycle(7000).unavailable.is_empty());
    assert_eq!(m.cycle(10000).unavailable, vec![1]);
}

#[test]
fn identity_sent_on_start() {
    let mut m = home();
    m.start(0).unwrap();
    let c = m.bus().central(1);
    assert_eq!([c.reg(0x004E), c.reg(0x004F), c.reg(0x0050)], [0x240A, 0xC401, 0x0203]);
    assert_eq!(c.reg(0x008B) & 0x0700, 0x0500);
}

#[test]
fn identity_off_sends_nothing() {
    let mut c = cfg();
    c.identification = false;
    c.heartbeat = false;
    let mut m = Master::new(SimBus::new(vec![SimCentral::from_fixture(HOME, 1)]), c);
    m.start(0).unwrap();
    assert!(m.bus().sent.iter().all(|f| f[1] == 0x03));
}

#[test]
fn heartbeat_every_45s() {
    let mut m = home();
    m.start(0).unwrap();
    let hb = heartbeat(1).encode();
    let count = |m: &Master<SimBus>| m.bus().sent.iter().filter(|f| **f == hb).count();
    let base = count(&m);
    m.cycle(0);
    m.cycle(44_999);
    assert_eq!(count(&m), base);
    m.cycle(45_000);
    assert_eq!(count(&m), base + 1);
}

#[test]
fn bus_trait_is_object_safe() {
    let b: Box<dyn Bus> = Box::new(SimBus::new(vec![]));
    drop(b);
}

#[test]
fn write_outside_poll_plan_is_unverified() {
    let mut m = home();
    m.start(0).unwrap();
    let id = m.submit(vec![Request::WriteSingle { addr: 1, reg: 0x00F0, value: 1 }]);
    assert_eq!(m.cycle(1000).writes, vec![(id, WriteResult::Unverified)]);
}

#[test]
fn failed_reread_is_unverified() {
    let mut m = home();
    m.start(0).unwrap();
    m.bus_mut().faults.drop_reads = true;
    let id = m.submit(vec![Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 }]);
    assert_eq!(m.cycle(1000).writes, vec![(id, WriteResult::Unverified)]);
}

#[test]
fn ignored_write_is_unverified() {
    let mut m = home();
    m.start(0).unwrap();
    m.bus_mut().faults.ack_without_apply = true;
    let id = m.submit(vec![Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 }]);
    assert_eq!(m.cycle(1000).writes, vec![(id, WriteResult::Unverified)]);
}

#[test]
fn partial_batch_reports_sent_and_rereads() {
    let mut m = home();
    m.start(0).unwrap();
    m.bus_mut().central_mut(1).refuse.push(0x0303);
    let id = m.submit(vec![
        Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 },
        Request::WriteSingle { addr: 1, reg: 0x0303, value: 215 },
    ]);
    let r = m.cycle(1000);
    assert_eq!(r.writes, vec![(id, WriteResult::Rejected { code: 3, sent: 1 })]);
    assert!(r.changes.iter().any(|c| c.reg == 0x0203 && c.new == 215));
}

#[test]
fn rediscover_finds_a_new_zone() {
    let bus = SimBus::new(vec![SimCentral::builder().zones(&[1, 2]).build()]);
    let mut m = Master::new(bus, cfg());
    m.start(0).unwrap();
    assert_eq!(m.topology().centrals[0].zones.len(), 2);
    // a third zone appears: zones-present word and its block (copied from zone 2)
    {
        let c = m.bus_mut().central_mut(1);
        let block: Vec<u16> = (0..65).map(|i| c.reg(0x0200 + i)).collect();
        for (i, v) in block.iter().enumerate() {
            c.handle(&Request::WriteSingle { addr: 1, reg: 0x0500 + i as u16, value: *v }.encode());
        }
        c.handle(&Request::WriteSingle { addr: 1, reg: 0x0009, value: 0b10011 }.encode());
    }
    assert!(m.rediscover(1000));
    assert_eq!(m.topology().centrals[0].zones.iter().map(|z| z.n).collect::<Vec<_>>(), vec![1, 2, 5]);
    assert_eq!(m.config().cycle_ms, 2300);
    let _bus: SimBus = m.into_bus();
}
