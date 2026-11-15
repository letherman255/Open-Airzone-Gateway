//! Everything the API works on. The platform (ESP firmware or host dev server) owns one `Gateway`,
//! keeps `registry` / `topology` / `info` current, drains `queue` into the master engine, reports results
//! with `complete`, and carries out `actions`.

use std::collections::{BTreeMap, VecDeque};

use crate::auth::Auth;
use crate::frame::Request;
use crate::journal::Journal;
use crate::master::WriteResult;
use crate::registry::Registry;
use crate::settings::{Mode, Settings};
use crate::topology::Topology;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GatewayInfo {
    pub version: String,
    pub build: String,
    pub board: String,
    pub mac: String,
    pub ip: String,
    pub rssi: Option<i32>,
    pub mqtt_connected: bool,
    /// Built with the `master` feature.
    pub master_capable: bool,
    /// Master engine running (master mode active and no other master heard).
    pub master_running: bool,
    pub uptime_s: u64,
    pub time: String,
    pub stream_dropped: u32,
    pub listen_dropped: u32,
    /// Newer release found in the update manifest (offered, never installed automatically).
    pub latest_version: Option<String>,
    pub latest_url: Option<String>,
    /// Why the board last restarted (power on, panic, watchdog, brownout…).
    pub reset_reason: String,
    pub free_heap: u32,
    /// Lowest free heap since boot.
    pub min_free_heap: u32,
    /// Message of the Rust panic that caused the last restart, if any.
    pub last_panic: String,
    /// Poll reads that never got a valid answer since boot.
    pub bus_read_errors: u32,
    /// Writes that ended not applied (no answer, rejected, unverified) since boot.
    pub write_failures: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    SaveSettings,
    Restart,
    Rediscover,
}

/// What the journal needs to know about one request of a queued write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: String,
    pub addr: u8,
    pub reg: u16,
    /// Register value before the write; `None` when it was never read (undo is then refused).
    pub old: Option<u16>,
    pub mask: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteState {
    Queued,
    Done(WriteResult),
}

pub struct Gateway {
    pub settings: Settings,
    pub auth: Auth,
    pub journal: Journal,
    pub registry: Registry,
    pub topology: Topology,
    pub dev_mode: bool,
    pub info: GatewayInfo,
    pub catalogue_json: &'static str,
    /// Writes waiting for the master engine: (id, requests).
    pub queue: Vec<(u32, Vec<Request>)>,
    pub writes: BTreeMap<u32, (WriteState, Vec<Note>)>,
    pub actions: Vec<Action>,
    /// Last bus frames (sequence, hex line) for the WebSocket `frames` topic.
    pub frames: VecDeque<(u64, String)>,
    /// Gateway events (sequence, JSON object) for the WebSocket `events` topic.
    pub events: VecDeque<(u64, String)>,
    /// Firmware log level: error, warn, info or debug.
    pub log_level: String,
    next_id: u32,
    seq: u64,
}

impl Gateway {
    pub fn new(settings: Settings, registry: Registry, topology: Topology, info: GatewayInfo, catalogue_json: &'static str) -> Self {
        let mut auth = Auth::from_hash(&settings.admin_hash);
        auth.load_tokens(&settings.api_tokens);
        Gateway {
            settings, auth, journal: Journal::default(), registry, topology, dev_mode: false, info, catalogue_json,
            queue: Vec::new(), writes: BTreeMap::new(), actions: Vec::new(), frames: VecDeque::new(), events: VecDeque::new(),
            log_level: "info".into(), next_id: 1, seq: 0,
        }
    }

    /// Effective mode: master only when configured, built in and running.
    pub fn master_active(&self) -> bool {
        self.settings.mode == Mode::Master && self.info.master_capable && self.info.master_running
    }

    pub fn enqueue(&mut self, requests: Vec<Request>, notes: Vec<Note>) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let what = notes.first().map(|n| n.id.clone()).unwrap_or_else(|| "write".into());
        self.push_event("write", &format!("write {id} queued ({what})"));
        self.queue.push((id, requests));
        self.writes.insert(id, (WriteState::Queued, notes));
        // forget the oldest finished writes only: a queued write is always tracked until it completes
        while self.writes.len() > 64 {
            let Some(done) = self.writes.iter().find(|(_, (s, _))| *s != WriteState::Queued).map(|(k, _)| *k) else { break };
            self.writes.remove(&done);
        }
        id
    }

    /// Result of a queued write from the master engine; applied writes go to the journal.
    pub fn complete(&mut self, id: u32, result: WriteResult, now_ms: u64) {
        let Some((state, notes)) = self.writes.get_mut(&id) else { return };
        if let WriteResult::Applied(values) = &result {
            for (n, v) in notes.iter().zip(values) {
                self.journal.record(&n.id, n.addr, n.reg, n.old, *v, n.mask, now_ms);
            }
        }
        let word = match &result {
            WriteResult::Applied(_) => "applied",
            WriteResult::Rejected { .. } => "rejected",
            WriteResult::NoAnswer { .. } => "no answer",
            WriteResult::Unverified => "unverified",
        };
        *state = WriteState::Done(result);
        self.push_event("write", &format!("write {id} {word}"));
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    pub fn push_frame(&mut self, line: String) {
        let s = self.next_seq();
        // frame lines are up to ~500 bytes: keep few (the ESP has ~100 KB of heap in all)
        if self.frames.len() >= 40 {
            self.frames.pop_front();
        }
        self.frames.push_back((s, line));
    }

    pub fn push_event(&mut self, kind: &str, text: &str) {
        let s = self.next_seq();
        if self.events.len() >= 100 {
            self.events.pop_front();
        }
        let j = format!(r#"{{"seq":{s},"kind":{},"text":{}}}"#, crate::ha::js(kind), crate::ha::js(text));
        self.events.push_back((s, j));
    }

    /// Persist auth state into settings (the platform saves settings on `Action::SaveSettings`).
    pub fn sync_auth_into_settings(&mut self) {
        self.settings.admin_hash = self.auth.hash().to_string();
        self.settings.api_tokens = self.auth.tokens_blob();
    }
}
