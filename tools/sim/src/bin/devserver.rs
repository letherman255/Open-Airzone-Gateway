//! Host dev server: the gateway web page and API on http://127.0.0.1:8780, backed by the simulated home
//! central in master mode. `cargo run -p airzone-sim --bin devserver [port] [--fake-update VERSION]`.
//! `--fake-update 9.9.9` seeds an available update so the page shows its banner; installing it is simulated.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use airzone_core::api::{handle, ApiRequest};
use airzone_core::crypto::{sha256, websocket_accept};
use airzone_core::gateway::{Action, Gateway, GatewayInfo};
use airzone_core::ha::{states, HaConfig};
use airzone_core::master::{Config, Master};
use airzone_core::settings::{Mode, Settings};
use airzone_core::ws::{live_messages, text_frame, WsCursor};
use airzone_sim::{SimBus, SimCentral};

const HOME: &str = include_str!("../../fixtures/home_2026-11-05.json");
const CATALOGUE: &str = include_str!("../../../../web/catalogue.json");
const INDEX: &str = include_str!("../../../../web/index.html");
const APP: &str = include_str!("../../../../web/app.js");
const STYLE: &str = include_str!("../../../../web/style.css");

struct State {
    gw: Gateway,
    master: Master<SimBus>,
    start: Instant,
    seq: u64,
}

fn random(seed: u64) -> [u8; 32] {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    sha256(format!("{t}-{seed}").as_bytes())
}

fn now_ms(s: &State) -> u64 {
    s.start.elapsed().as_millis() as u64
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let port: u16 = args.first().and_then(|p| p.parse().ok()).unwrap_or(8780);
    let fake_update = args.iter().position(|a| a == "--fake-update").and_then(|i| args.get(i + 1)).cloned();
    let mut master = Master::new(SimBus::new(vec![SimCentral::from_fixture(HOME, 1)]), Config::default());
    let topo = master.start(0).expect("simulated central").clone();
    let mut settings = Settings::default();
    settings.mode = Mode::Master;
    let mut info = GatewayInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        build: "dev".into(),
        board: "host-devserver".into(),
        mac: "00:00:00:00:00:00".into(),
        ip: "127.0.0.1".into(),
        master_capable: true,
        master_running: true,
        ..Default::default()
    };
    if let Some(v) = fake_update {
        settings.update_manifest = "https://example.org/ota.json".into();
        info.update_checked = "uptime 0 s ok".into();
        info.latest_url = Some(format!("https://example.org/airzone-gateway-{v}.bin"));
        info.latest_version = Some(v);
    }
    let gw = Gateway::new(settings, master.registry().clone(), topo, info, CATALOGUE);
    let st = Arc::new(Mutex::new(State { gw, master, start: Instant::now(), seq: 0 }));

    {
        let st = st.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(2300));
            let mut s = st.lock().unwrap();
            let now = now_ms(&s);
            let queue = std::mem::take(&mut s.gw.queue);
            let mut ids = Vec::new();
            for (id, reqs) in queue {
                ids.push((id, s.master.submit(reqs)));
            }
            let rep = s.master.cycle(now);
            for (gid, mid) in ids {
                if let Some((_, r)) = rep.writes.iter().find(|(w, _)| *w == mid) {
                    s.gw.complete(gid, r.clone(), now);
                }
            }
            s.gw.registry = s.master.registry().clone();
            s.gw.topology = s.master.topology().clone();
            s.gw.info.uptime_s = now / 1000;
            for a in std::mem::take(&mut s.gw.actions) {
                println!("action: {a:?}");
                if a == Action::Rediscover {
                    let _ = s.master.start(now);
                }
                if matches!(a, Action::InstallUpdate { .. }) {
                    simulate_install(st.clone());
                }
            }
        });
    }

    let l = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    println!("devserver on http://127.0.0.1:{port}/ (simulated home central, master mode)");
    for c in l.incoming().flatten() {
        let st = st.clone();
        std::thread::spawn(move || {
            let _ = serve(c, st);
        });
    }
}

/// Simulated install: progress, installing, restarting, then the "new version" answers (no update left).
fn simulate_install(st: Arc<Mutex<State>>) {
    std::thread::spawn(move || {
        let mut steps: Vec<String> = (0..=4).map(|i| format!("downloading {}%", i * 25)).collect();
        steps.push("installing".into());
        steps.push("restarting".into());
        for step in steps {
            st.lock().unwrap().gw.info.update_state = step;
            std::thread::sleep(Duration::from_millis(1500));
        }
        let mut s = st.lock().unwrap();
        s.gw.info.update_state.clear();
        s.gw.info.latest_version = None;
        s.gw.info.latest_url = None;
    });
}

fn read_request(c: &mut TcpStream) -> Option<(String, String, Vec<(String, String)>, String)> {
    c.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        let n = c.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 32 * 1024 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let first = lines.next()?;
    let mut it = first.split(' ');
    let (method, target) = (it.next()?.to_string(), it.next()?.to_string());
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
    let len: usize = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0).min(16 * 1024);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = c.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    Some((method, target, headers, String::from_utf8_lossy(&body).to_string()))
}

fn respond(c: &mut TcpStream, code: u16, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        c,
        "HTTP/1.1 {code} {}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        if code < 300 { "OK" } else { "Error" },
        body.len()
    )?;
    c.write_all(body)
}

fn serve(mut c: TcpStream, st: Arc<Mutex<State>>) -> std::io::Result<()> {
    let Some((method, target, headers, body)) = read_request(&mut c) else { return Ok(()) };
    let (path, query) = target.split_once('?').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((target.clone(), String::new()));
    match (method.as_str(), path.as_str()) {
        ("GET", "/") => return respond(&mut c, 200, "text/html; charset=utf-8", INDEX.as_bytes()),
        ("GET", "/app.js") => return respond(&mut c, 200, "text/javascript; charset=utf-8", APP.as_bytes()),
        ("GET", "/style.css") => return respond(&mut c, 200, "text/css; charset=utf-8", STYLE.as_bytes()),
        ("GET", "/api/v1/ws") => return websocket(c, &headers, st),
        _ => {}
    }
    let bearer = headers.iter().find(|(k, _)| k == "authorization").and_then(|(_, v)| v.strip_prefix("Bearer ").map(String::from));
    let header = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    let r = ApiRequest { method, path, query, body, bearer, content_type: header("content-type"), host: header("host"), origin: header("origin") };
    let resp = {
        let mut s = st.lock().unwrap();
        s.seq += 1;
        let (now, seed) = (now_ms(&s), s.seq);
        handle(&mut s.gw, &r, now, random(seed))
    };
    respond(&mut c, resp.status, resp.content_type, resp.body.as_bytes())
}

/// `/api/v1/ws`: every 2 s, one `{"topic":"values","states":[…]}` message with the device states.
fn websocket(mut c: TcpStream, headers: &[(String, String)], st: Arc<Mutex<State>>) -> std::io::Result<()> {
    let Some((_, key)) = headers.iter().find(|(k, _)| k == "sec-websocket-key") else { return respond(&mut c, 400, "text/plain", b"no key") };
    write!(c, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", websocket_accept(key))?;
    let ha = HaConfig { prefix: "airzone".into(), discovery: "homeassistant".into(), gateway_id: "dev".into(), gateway_name: "dev".into(), fw: "dev".into(), commands: false };
    let mut cursor = WsCursor::default();
    loop {
        let msgs = {
            let mut s = st.lock().unwrap();
            let items: Vec<String> = states(&ha, &s.gw.topology, &s.gw.registry)
                .into_iter()
                .map(|m| format!(r#"{{"topic":{},"state":{}}}"#, airzone_core::ha::js(m.topic.trim_start_matches("airzone/dev/")), m.payload))
                .collect();
            let values = format!(r#"{{"topic":"values","states":[{}]}}"#, items.join(","));
            let now = now_ms(&s);
            live_messages(&mut s.gw, &mut cursor, values, now)
        };
        for m in msgs {
            c.write_all(&text_frame(&m))?;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
