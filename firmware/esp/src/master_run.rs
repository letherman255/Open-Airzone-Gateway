//! Master mode (feature `master`): the gateway drives the bus with the core `Master` engine.
//! Starts by listening 5 s; if another master is heard it never transmits and stays a listener.

use std::sync::atomic::Ordering;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use airzone_core::bus::{Bus, BusError};
use airzone_core::deframe::Deframer;
use airzone_core::master::{Config, Master};
use airzone_core::rules::{calendar_writes, Identity};
use airzone_core::settings::Settings;
use anyhow::Result;
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::uart::{config, Uart, UartDriver};
use esp_idf_svc::sys;

use crate::board::BOARD;
use crate::stream::{self, now_us};
use crate::Shared;

const CALENDAR_MS: u64 = 270_000;
/// Silence before each request: the central ignored requests sent 6-8 ms after its previous reply
/// (2026-11-06); the stock webserver leaves at least 18 ms.
const TURNAROUND: Duration = Duration::from_millis(20);

struct UartBus {
    uart: UartDriver<'static>,
    to_stream: SyncSender<String>,
    /// Last byte sent or received on the bus.
    last_activity: Instant,
}

fn de(high: bool) {
    unsafe {
        sys::gpio_set_level(BOARD.de, high as u32);
    }
}

impl UartBus {
    fn read_frames(&mut self, deadline: Instant, want_one: bool) -> Vec<Vec<u8>> {
        let mut d = Deframer::default();
        let mut out = Vec::new();
        let mut buf = [0u8; 256];
        while Instant::now() < deadline {
            let n = self.uart.read(&mut buf, 2).unwrap_or(0);
            if n == 0 {
                continue;
            }
            self.last_activity = Instant::now();
            stream::send(&self.to_stream, stream::chunk_line(now_us(), &buf[..n]));
            out.extend(d.push(&buf[..n], 0));
            if want_one && !out.is_empty() {
                break;
            }
        }
        out
    }
}

impl Bus for UartBus {
    fn transact(&mut self, request: &[u8], timeout_ms: u32) -> Result<Vec<u8>, BusError> {
        // a late tail of the previous reply also counts as activity: drain it, then keep the turnaround
        let mut junk = [0u8; 64];
        while self.uart.read(&mut junk, 0).unwrap_or(0) > 0 {
            self.last_activity = Instant::now();
        }
        let quiet = self.last_activity.elapsed();
        if quiet < TURNAROUND {
            std::thread::sleep(TURNAROUND - quiet);
        }
        de(true);
        let wrote = self.uart.write(request).map_err(|_| BusError::Io);
        let _ = self.uart.wait_tx_done(50);
        de(false);
        self.last_activity = Instant::now();
        wrote?;
        stream::send(&self.to_stream, stream::chunk_line(now_us(), request));
        let frames = self.read_frames(Instant::now() + Duration::from_millis(timeout_ms as u64), true);
        frames.into_iter().next().ok_or(BusError::Timeout)
    }

    fn listen(&mut self, duration_ms: u32) -> Vec<Vec<u8>> {
        self.read_frames(Instant::now() + Duration::from_millis(duration_ms as u64), false)
    }
}

pub fn start(uart1: impl Uart + 'static, cfg: &config::Config, shared: Arc<Shared>, to_stream: SyncSender<String>, s: &Settings) -> Result<()> {
    let uart = UartDriver::new(
        uart1,
        unsafe { AnyIOPin::steal(BOARD.uart_tx as u8) },
        unsafe { AnyIOPin::steal(BOARD.uart_rx as u8) },
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
        cfg,
    )?;
    // release the hold set at boot; DE is now driven only around each request
    unsafe {
        sys::gpio_hold_dis(BOARD.de);
    }
    de(false);
    let mut identity = Identity { name: s.identity_name, ..Default::default() };
    let mac = crate::sta_mac();
    identity.mac = mac;
    let mcfg = Config { identity, heartbeat: s.heartbeat, identification: s.identification, ..Default::default() };
    let (renew, fcool) = (s.renew_days.clone(), s.fcool_months.clone());
    std::thread::Builder::new().name("master".into()).stack_size(16 * 1024).spawn(move || {
        let mut m = Master::new(UartBus { uart, to_stream: to_stream.clone(), last_activity: Instant::now() }, mcfg);
        let start = crate::now_ms();
        match m.start(start) {
            Ok(t) => {
                log::info!("master mode: {} central(s)", t.centrals.len());
                stream::send(&to_stream, format!("# master running centrals={}\n", t.centrals.len()));
            }
            Err(e) => {
                // another master (or no central): never transmit; stay a passive listener on this UART
                log::error!("master mode refused: {e:?}; listening only");
                stream::send(&to_stream, format!("# master refused: {e:?}, listen only\n"));
                unsafe {
                    sys::gpio_hold_en(BOARD.de);
                }
                let mut bus = m.into_bus();
                loop {
                    for f in bus.listen(1000) {
                        if let Ok(mut l) = shared.listener.lock() {
                            l.feed(&f, 100_000, crate::now_ms());
                        }
                    }
                }
            }
        }
        shared.master_running.store(true, Ordering::SeqCst);
        shared.gw.lock().unwrap().info.master_running = true;
        let mut last_calendar = 0u64;
        // results waiting for the API lock: the bus loop never blocks on the API (PBKDF2 can hold it ~1 s)
        let mut pending: Vec<(u32, airzone_core::master::WriteResult)> = Vec::new();
        // gw.registry mirrors the engine's registry through its changes: a full copy (~30 KB) is made only
        // after (re)discovery; copying it every cycle exhausted the heap (2026-11-06)
        let mut full_copy = true;
        let mut pending_changes: Vec<airzone_core::registry::Change> = Vec::new();
        let cycle = Duration::from_millis(m.config().cycle_ms as u64);
        // health counters for /api/v1/info and Home Assistant
        let mut write_failures = 0u32;
        let mut last_cycle = Instant::now();
        loop {
            // short ticks: a queued command goes out within ~50 ms instead of waiting for the poll cycle
            // (clicks in Home Assistant took 2-6 s, 2026-11-06)
            std::thread::sleep(Duration::from_millis(40));
            let now = crate::now_ms();
            if crate::clock::Clock::time_valid() && (last_calendar == 0 || now >= last_calendar + CALENDAR_MS) {
                let (_, mon, _, _, _, _, wd) = crate::clock::local_now();
                let addrs: Vec<u8> = m.topology().centrals.iter().filter(|c| c.iaq).map(|c| c.addr).collect();
                for a in addrs {
                    m.submit(calendar_writes(a, &renew, &fcool, wd as u8, (mon - 1) as u8).to_vec());
                }
                last_calendar = now;
            }
            let queue = match shared.gw.try_lock() {
                Ok(mut gw) => std::mem::take(&mut gw.queue),
                Err(_) => Vec::new(),
            };
            let ids: Vec<(u32, u32)> = queue.into_iter().map(|(gid, reqs)| (gid, m.submit(reqs))).collect();
            // the plan is read one block per step, spread over the cycle period: a command waits one block
            // read at most instead of a whole cycle (~2 s of bus time for 6 zones)
            let step = cycle / (m.plan().len().max(1) as u32);
            let due = last_cycle.elapsed() >= step;
            // results and changes not yet handed over (API lock busy) are retried every tick: waiting for the
            // next cycle delayed the state seen in Home Assistant by seconds
            let waiting = !pending.is_empty() || !pending_changes.is_empty() || full_copy;
            if ids.is_empty() && !due && !waiting {
                continue;
            }
            let rep = if due {
                last_cycle = Instant::now();
                if shared.rediscover.swap(false, Ordering::SeqCst) {
                    let _ = m.rediscover(now);
                    full_copy = true;
                }
                m.poll_step(now)
            } else if !ids.is_empty() {
                m.apply_queued(now)
            } else {
                airzone_core::master::CycleReport::default()
            };
            write_failures += rep.writes.iter().filter(|(_, r)| !matches!(r, airzone_core::master::WriteResult::Applied(_))).count() as u32;
            for (gid, mid) in ids {
                if let Some((_, r)) = rep.writes.iter().find(|(w, _)| *w == mid) {
                    pending.push((gid, r.clone()));
                }
            }
            full_copy |= rep.rediscovered;
            if !full_copy {
                pending_changes.extend(rep.changes);
            }
            if let Ok(mut gw) = shared.gw.try_lock() {
                // tell MQTT once the new values are where it reads them
                if !pending.is_empty() || !pending_changes.is_empty() || full_copy {
                    shared.state_dirty.store(true, Ordering::SeqCst);
                }
                for (gid, r) in pending.drain(..) {
                    gw.complete(gid, r, now);
                }
                gw.topology = m.topology().clone();
                gw.info.bus_read_errors = m.read_failures();
                gw.info.write_failures = write_failures;
                if full_copy {
                    // drop the old copy first: never two full registries at once
                    gw.registry = airzone_core::registry::Registry::default();
                    gw.registry = m.registry().clone();
                    full_copy = false;
                    pending_changes.clear();
                } else {
                    gw.registry.apply(&pending_changes, now);
                    pending_changes.clear();
                }
            }
            if let Ok(mut t) = shared.topology.try_lock() {
                *t = m.topology().clone();
            }
        }
    })?;
    Ok(())
}
