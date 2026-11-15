//! MQTT: Home Assistant discovery + state publishing, gateway entities, and (master mode only) commands.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use airzone_core::catalogue::param;
use airzone_core::ha::{
    availability_topic, command, discovery_each, discovery_topics, gateway_action, gateway_discovery, gateway_state, removed_topics, states,
    Command, HaConfig,
};
use airzone_core::model::{encode, Input, Target};
use airzone_core::rules::set_mode;
use airzone_core::settings::Settings;
use anyhow::Result;
use esp_idf_svc::mqtt::client::{EspMqttClient, EventPayload, LwtConfiguration, MqttClientConfiguration, QoS};

use crate::Shared;

pub static CONNECTED: AtomicBool = AtomicBool::new(false);
static NEED_DISCOVERY: AtomicBool = AtomicBool::new(true);
static NEED_SUBSCRIBE: AtomicBool = AtomicBool::new(true);
const FULL_REFRESH: Duration = Duration::from_secs(300);

/// Turn an accepted MQTT command into queued bus writes (master mode).
fn apply_command(shared: &Shared, cmd: Command) {
    let mut gw = shared.gw.lock().unwrap();
    if !gw.master_active() || !shared.master_running.load(Ordering::SeqCst) {
        return;
    }
    let reqs = match cmd {
        Command::Param { param: p, target, input } => match encode(p, &target, &input, None) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("mqtt command {} refused: {e:?}", p.id);
                return;
            }
        },
        Command::Mode { addr, zone, mode } => {
            let Some(c) = gw.topology.centrals.iter().find(|c| c.addr == addr).cloned() else { return };
            let Ok(mut r) = set_mode(&c, &gw.registry, mode) else { return };
            if let Some(p) = param("zone.power") {
                if let Ok(on) = encode(p, &Target::Zone { addr, zone }, &Input::Bool(true), None) {
                    r.extend(on);
                }
            }
            r
        }
    };
    gw.enqueue(reqs, Vec::new());
}

pub fn run(shared: Arc<Shared>, s: Settings, ha: HaConfig) -> Result<()> {
    if s.mqtt_url.is_empty() {
        log::info!("mqtt disabled (no broker configured)");
        return Ok(());
    }
    let avail = availability_topic(&ha);
    let client_id = ha.gateway_id.clone();
    let conf = MqttClientConfiguration {
        client_id: Some(&client_id),
        username: (!s.mqtt_user.is_empty()).then_some(s.mqtt_user.as_str()),
        password: (!s.mqtt_pass.is_empty()).then_some(s.mqtt_pass.as_str()),
        lwt: Some(LwtConfiguration { topic: &avail, payload: b"offline", qos: QoS::AtLeastOnce, retain: true }),
        keep_alive_interval: Some(Duration::from_secs(30)),
        // largest message: a climate discovery config of ~1.1 KB
        buffer_size: 2048,
        out_buffer_size: 2048,
        ..Default::default()
    };
    let (mut client, mut conn) = EspMqttClient::new(&s.mqtt_url, &conf)?;
    let birth = format!("{}/status", ha.discovery);
    let base = format!("{}/{}", ha.prefix, ha.gateway_id);
    let (in_tx, in_rx) = sync_channel::<(String, Vec<u8>)>(32);
    std::thread::Builder::new().name("mqtt-ev".into()).stack_size(6 * 1024).spawn(move || {
        while let Ok(ev) = conn.next() {
            match ev.payload() {
                EventPayload::Connected(_) => {
                    crate::crumb("mqtt-ev connected");
                    log::info!("mqtt connected");
                    CONNECTED.store(true, Ordering::SeqCst);
                    NEED_DISCOVERY.store(true, Ordering::SeqCst);
                    NEED_SUBSCRIBE.store(true, Ordering::SeqCst);
                }
                EventPayload::Disconnected => {
                    log::warn!("mqtt disconnected");
                    CONNECTED.store(false, Ordering::SeqCst);
                }
                EventPayload::Received { topic: Some(t), data, .. } => {
                    crate::crumb("mqtt-ev received");
                    if t.ends_with("/status") && data == b"online" {
                        NEED_DISCOVERY.store(true, Ordering::SeqCst);
                    } else if let Err(TrySendError::Full(_)) = in_tx.try_send((t.to_string(), data.to_vec())) {
                        log::warn!("mqtt command dropped");
                    }
                }
                _ => {}
            }
        }
    })?;

    let mut last: HashMap<String, String> = HashMap::new();
    let mut last_full = Instant::now();
    let mut last_topics = String::new();
    let mut last_fingerprint = 0u64;
    let mut last_topic_check: Option<Instant> = None;
    let mut last_tick = Instant::now();
    let handle = |topic: String, data: Vec<u8>| {
        if let Some(a) = gateway_action(&ha, &topic) {
            log::info!("mqtt gateway action {a}");
            match a {
                "restart" => crate::restart_later(),
                _ => shared.rediscover.store(true, Ordering::SeqCst),
            }
            return;
        }
        let topo = shared.topology.lock().unwrap().clone();
        if let Some(cmd) = command(&ha, &topo, &topic, &data) {
            apply_command(&shared, cmd);
        }
    };
    loop {
        // commands are handled as soon as they arrive; states go out every 2 s, or right away when the master
        // engine reports a change (a click used to take 2-6 s to come back, 2026-11-06)
        if let Ok((topic, data)) = in_rx.recv_timeout(Duration::from_millis(100)) {
            handle(topic, data);
            while let Ok((topic, data)) = in_rx.try_recv() {
                handle(topic, data);
            }
        }
        let dirty = shared.state_dirty.swap(false, Ordering::SeqCst);
        if !dirty && last_tick.elapsed() < Duration::from_secs(2) {
            continue;
        }
        if !dirty {
            last_tick = Instant::now();
        }
        if !CONNECTED.load(Ordering::SeqCst) {
            continue;
        }
        if NEED_SUBSCRIBE.swap(false, Ordering::SeqCst) {
            crate::crumb("mqtt subscribe");
            let mut ok = client.subscribe(&birth, QoS::AtMostOnce).is_ok();
            ok &= client.subscribe(&format!("{base}/gateway/set/+"), QoS::AtMostOnce).is_ok();
            if ha.commands {
                for t in [format!("{base}/+/zone/+/set/+"), format!("{base}/+/system/set/+"), format!("{base}/+/iaq/set/+")] {
                    ok &= client.subscribe(&t, QoS::AtMostOnce).is_ok();
                }
            }
            if !ok {
                NEED_SUBSCRIBE.store(true, Ordering::SeqCst);
            }
        }
        crate::crumb("mqtt topo lock");
        let topo = shared.topology.lock().unwrap().clone();
        crate::crumb("mqtt gw lock");
        let (info, mode, dev) = {
            let mut gw = shared.gw.lock().unwrap();
            crate::crumb("mqtt refresh_info");
            crate::refresh_info(&mut gw);
            crate::crumb("mqtt info clone");
            let mode = if shared.master_running.load(Ordering::SeqCst) { "master" } else { "listen" };
            (gw.info.clone(), mode, gw.dev_mode)
        };
        // the registry is read in place, under its lock (a copy is ~30 KB): never lock gw / listener inside
        // discovery (~61 KB for 6 zones) is never held at once: its topics are checked every 30 s, and the
        // messages are built and published one by one only when needed
        let need = NEED_DISCOVERY.swap(false, Ordering::SeqCst);
        if need || last_topic_check.map_or(true, |t| t.elapsed() >= Duration::from_secs(30)) {
            last_topic_check = Some(Instant::now());
            crate::crumb("mqtt discovery topics");
            // fingerprint of the topic set, computed message by message (nothing kept)
            let fingerprint = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for m in gateway_discovery(&ha, &info) {
                    m.topic.hash(&mut h);
                }
                if !topo.centrals.is_empty() {
                    shared.with_registry(|reg| discovery_each(&ha, &topo, reg, &mut |m| m.topic.hash(&mut h)));
                }
                h.finish()
            };
            if need || fingerprint != last_fingerprint {
                last_fingerprint = fingerprint;
                let mut topics: Vec<String> = gateway_discovery(&ha, &info).into_iter().map(|m| m.topic).collect();
                if !topo.centrals.is_empty() {
                    topics.extend(shared.with_registry(|reg| discovery_topics(&ha, &topo, reg)));
                }
                // QoS 0: with QoS 1 the client keeps every message in its outbox until acknowledged, and
                // ~110 discovery configs (~61 KB) exhausted the heap (panic loop, 2026-11-06). Retained
                // QoS 0 messages are kept by the broker all the same.
                let _ = client.publish(&avail, QoS::AtMostOnce, true, b"online");
                // entities that disappeared (outlet unlinked, sensor gone): delete their retained configs
                let previous: Vec<String> = last_topics.lines().map(String::from).collect();
                for m in removed_topics(&previous, &topics) {
                    let _ = client.publish(&m.topic, QoS::AtMostOnce, true, m.payload.as_bytes());
                }
                drop(previous);
                // one compact string instead of ~110 small allocations
                last_topics = topics.join("\n");
                drop(topics);
                let mut failed = false;
                let mut count = 0u32;
                let mut publish = |m: airzone_core::ha::Message| {
                    count += 1;
                    let mut step = [0u8; 32];
                    let n = {
                        use std::io::Write;
                        let mut c = std::io::Cursor::new(&mut step[..]);
                        let _ = write!(c, "mqtt discovery #{count}");
                        c.position() as usize
                    };
                    crate::crumb(std::str::from_utf8(&step[..n]).unwrap_or("mqtt discovery"));
                    if client.publish(&m.topic, QoS::AtMostOnce, m.retain, m.payload.as_bytes()).is_err() {
                        failed = true;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                };
                for m in gateway_discovery(&ha, &info) {
                    publish(m);
                }
                if !topo.centrals.is_empty() {
                    // holds the registry lock ~2 s (rare: connection, HA restart, entity change); the master
                    // engine only try_locks, the API waits
                    shared.with_registry(|reg| discovery_each(&ha, &topo, reg, &mut publish));
                }
                if failed {
                    NEED_DISCOVERY.store(true, Ordering::SeqCst);
                }
                last.clear();
            }
        }
        crate::crumb("mqtt states");
        let mut st = shared.with_registry(|reg| states(&ha, &topo, reg));
        st.push(gateway_state(&ha, &info, mode, dev));
        let full = last_full.elapsed() >= FULL_REFRESH;
        if full {
            last_full = Instant::now();
        }
        for m in st {
            if full || last.get(&m.topic) != Some(&m.payload) {
                if client.publish(&m.topic, QoS::AtMostOnce, false, m.payload.as_bytes()).is_ok() {
                    last.insert(m.topic, m.payload);
                }
            }
        }
    }
}
