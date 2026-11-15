use airzone_core::api::{handle, ApiRequest};
use airzone_core::frame::Request;
use airzone_core::gateway::{Action, Gateway, GatewayInfo};
use airzone_core::json::{parse, Value};
use airzone_core::listen::Listener;
use airzone_core::settings::{Mode, Settings};
use airzone_core::topology::{build_central, Topology};

const STARTUP: &str = include_str!("../fixtures/capture_startup.log");

fn gateway(mode: Mode) -> Gateway {
    let mut l = Listener::default();
    for (i, line) in STARTUP.lines().enumerate() {
        let mut it = line.split_whitespace();
        let (Some(_), Some(mono), Some(len)) = (it.next(), it.next(), it.next()) else { continue };
        if mono.starts_with('#') {
            continue;
        }
        let bytes: Vec<u8> = it.map(|b| u8::from_str_radix(b, 16).unwrap()).collect();
        if len.parse::<usize>().ok() == Some(bytes.len()) {
            l.feed(&bytes, 100_000, i as u64);
        }
    }
    let registry = l.registry().clone();
    let topology = Topology { centrals: vec![build_central(1, &registry).unwrap()] };
    let mut s = Settings::default();
    s.wifi_pass = "wifisecret".into();
    s.mqtt_pass = "mqttsecret".into();
    s.mode = mode;
    let info = GatewayInfo { version: "0.3.0".into(), build: "1".into(), board: "test".into(), mac: "24:0a:c4:01:02:03".into(), master_capable: true, master_running: mode == Mode::Master, ..Default::default() };
    Gateway::new(s, registry, topology, info, "{\"version\":1,\"params\":[]}")
}

fn req(method: &str, path: &str, body: &str, bearer: Option<&str>) -> ApiRequest {
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    ApiRequest {
        method: method.into(),
        path: path.into(),
        query: query.into(),
        body: body.into(),
        bearer: bearer.map(String::from),
        content_type: Some("application/json".into()),
        ..Default::default()
    }
}

fn call(g: &mut Gateway, method: &str, path: &str, body: &str, bearer: Option<&str>) -> (u16, Value) {
    let r = handle(g, &req(method, path, body, bearer), 1_000, [5; 32]);
    let v = parse(&r.body).unwrap_or_else(|e| panic!("{path}: invalid json {e:?}: {}", r.body));
    (r.status, v)
}

/// Sets the admin password and returns a session token.
fn login(g: &mut Gateway) -> String {
    let (st, _) = call(g, "POST", "/api/v1/password", r#"{"new":"longpassword"}"#, None);
    assert_eq!(st, 200);
    let (st, v) = call(g, "POST", "/api/v1/login", r#"{"password":"longpassword"}"#, None);
    assert_eq!(st, 200);
    v.get("token").unwrap().as_str().unwrap().to_string()
}

#[test]
fn api_info_and_zones() {
    let mut g = gateway(Mode::Listen);
    let (st, v) = call(&mut g, "GET", "/api/v1/info", "", None);
    assert_eq!(st, 200);
    assert_eq!(v.get("mode").unwrap().as_str(), Some("listen"));
    // remote crash diagnosis: why the board last restarted, and the heap
    for k in ["reset_reason", "free_heap", "min_free_heap", "last_panic"] {
        assert!(v.get(k).is_some(), "info lacks {k}: {v}");
    }
    let (_, v) = call(&mut g, "GET", "/api/v1/systems", "", None);
    let Value::Arr(cs) = v else { panic!() };
    let Some(Value::Arr(zones)) = cs[0].get("zones") else { panic!() };
    assert_eq!(zones.len(), 6);
    let (st, v) = call(&mut g, "GET", "/api/v1/systems/1/zones/2", "", None);
    assert_eq!(st, 200);
    assert_eq!(v.get("name").unwrap().as_str(), Some("Salon"));
    assert_eq!(v.get("values").unwrap().get("setpoint").unwrap().as_f64(), Some(21.0));
    let (st, v) = call(&mut g, "GET", "/api/v1/params/system.qadapt?system=1", "", None);
    assert_eq!(st, 200);
    assert_eq!(v.get("value").unwrap().as_str(), Some("power"));
    assert_eq!(call(&mut g, "GET", "/api/v1/systems/1/zones/9", "", None).0, 404);
}

#[test]
fn api_write_needs_token() {
    let mut g = gateway(Mode::Master);
    let body = r#"{"system":1,"zone":2,"value":21.5}"#;
    assert_eq!(call(&mut g, "POST", "/api/v1/params/zone.setpoint", body, None).0, 401);
    assert_eq!(call(&mut g, "POST", "/api/v1/params/zone.setpoint", body, Some("nope")).0, 401);
    assert_eq!(call(&mut g, "POST", "/api/v1/dev", r#"{"on":true}"#, None).0, 401);
    assert_eq!(call(&mut g, "GET", "/api/v1/settings", "", None).0, 401);
    assert!(g.queue.is_empty());
}

#[test]
fn api_listen_mode_refuses_writes() {
    let mut g = gateway(Mode::Listen);
    let t = login(&mut g);
    let (st, v) = call(&mut g, "POST", "/api/v1/params/zone.setpoint", r#"{"system":1,"zone":2,"value":21.5}"#, Some(&t));
    assert_eq!(st, 409, "{v}");
    call(&mut g, "POST", "/api/v1/dev", r#"{"on":true}"#, Some(&t));
    assert_eq!(call(&mut g, "POST", "/api/v1/raw/write", r#"{"addr":1,"reg":515,"value":215}"#, Some(&t)).0, 409);
    assert_eq!(call(&mut g, "POST", "/api/v1/journal/0/undo", "", Some(&t)).0, 409);
    assert!(g.queue.is_empty());
}

#[test]
fn api_param_write_queues_encoded_requests() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    let (st, v) = call(&mut g, "POST", "/api/v1/params/zone.setpoint", r#"{"system":1,"zone":2,"value":21.5}"#, Some(&t));
    assert_eq!(st, 202, "{v}");
    let id = v.get("queued").unwrap().as_f64().unwrap() as u32;
    assert_eq!(g.queue.last().unwrap().1, vec![Request::WriteSingle { addr: 1, reg: 0x0203, value: 215 }]);
    assert_eq!(call(&mut g, "GET", &format!("/api/v1/writes/{id}"), "", Some(&t)).1.get("state").unwrap().as_str(), Some("queued"));
    // enum by label, global mode through the master zone
    let (st, _) = call(&mut g, "POST", "/api/v1/params/zone.mode", r#"{"system":1,"zone":5,"value":"cool"}"#, Some(&t));
    assert_eq!(st, 202);
    assert_eq!(g.queue.last().unwrap().1, vec![Request::MaskWrite { addr: 1, reg: 0x0200, and: 0xF0FF, or: 0x0900 }]);
    assert_eq!(call(&mut g, "POST", "/api/v1/params/zone.setpoint", r#"{"system":1,"zone":2,"value":99}"#, Some(&t)).0, 400);
}

#[test]
fn api_write_refused_when_equipment_off_or_value_off_step() {
    // the capture has the VMC off: continuous ventilation is accepted by the central but never applied
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    let (st, v) = call(&mut g, "POST", "/api/v1/params/iaq.vent_continuous", r#"{"system":1,"value":true}"#, Some(&t));
    assert_eq!(st, 409, "{v}");
    let (st, v) = call(&mut g, "POST", "/api/v1/params/iaq.co2_gm", r#"{"system":1,"value":801}"#, Some(&t));
    assert_eq!(st, 400, "{v}");
    assert!(v.get("error").unwrap().as_str().unwrap().contains("step"), "{v}");
    assert!(g.queue.is_empty());
    // switching it off stays possible (it can be on while the VMC is off, seen 2026-11-06)
    let (st, v) = call(&mut g, "POST", "/api/v1/params/iaq.vent_continuous", r#"{"system":1,"value":false}"#, Some(&t));
    assert_eq!(st, 202, "{v}");
}

#[test]
fn api_write_refused_before_discovery() {
    // right after a restart the topology is empty: the equipment checks cannot run, so nothing is written
    let mut g = gateway(Mode::Master);
    g.topology.centrals.clear();
    let t = login(&mut g);
    let (st, v) = call(&mut g, "POST", "/api/v1/params/iaq.vent_continuous", r#"{"system":1,"value":true}"#, Some(&t));
    assert_eq!(st, 409, "{v}");
    let (st, v) = call(&mut g, "POST", "/api/v1/params/zone.setpoint", r#"{"system":1,"zone":2,"value":21}"#, Some(&t));
    assert_eq!(st, 409, "{v}");
    assert!(g.queue.is_empty());
}

#[test]
fn api_danger_needs_confirm() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    let (st, v) = call(&mut g, "POST", "/api/v1/params/system.d1_input", r#"{"system":1,"value":"bridge alarm"}"#, Some(&t));
    assert_eq!(st, 403);
    assert!(v.get("warning").unwrap().as_str().unwrap().contains("R32"));
    assert_eq!(v.get("confirm").unwrap().as_str(), Some("D1"));
    let (st, _) = call(&mut g, "POST", "/api/v1/params/system.d1_input", r#"{"system":1,"value":"bridge alarm","confirm":"D1"}"#, Some(&t));
    assert_eq!(st, 202);
}

#[test]
fn api_raw_needs_dev_mode() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    assert_eq!(call(&mut g, "GET", "/api/v1/raw/1/512?count=4", "", Some(&t)).0, 403);
    assert_eq!(call(&mut g, "GET", "/api/v1/registry", "", Some(&t)).0, 403);
    assert_eq!(call(&mut g, "POST", "/api/v1/dev", r#"{"on":true}"#, Some(&t)).0, 200);
    let (st, v) = call(&mut g, "GET", "/api/v1/raw/1/512?count=4", "", Some(&t));
    assert_eq!(st, 200);
    let Some(Value::Arr(vals)) = v.get("values") else { panic!() };
    assert_eq!(vals[0].as_f64(), Some(0x0504 as f64));
}

#[test]
fn api_raw_write_danger_bits_refused() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    call(&mut g, "POST", "/api/v1/dev", r#"{"on":true}"#, Some(&t));
    // 0x0048 holds Q-Adapt (bits 0-2) and the DI1 input (bits 9-11, danger)
    assert_eq!(call(&mut g, "POST", "/api/v1/raw/write", r#"{"addr":1,"reg":72,"value":1537}"#, Some(&t)).0, 403);
    assert_eq!(call(&mut g, "POST", "/api/v1/raw/write", r#"{"addr":1,"reg":72,"and":61951,"or":512}"#, Some(&t)).0, 403);
    assert_eq!(call(&mut g, "POST", "/api/v1/raw/write", r#"{"addr":1,"reg":72,"and":65528,"or":1}"#, Some(&t)).0, 202);
    assert_eq!(call(&mut g, "POST", "/api/v1/raw/write", r#"{"addr":1,"reg":72,"and":61951,"or":512,"confirm":"D1"}"#, Some(&t)).0, 202);
}

#[test]
fn api_no_secret_in_responses() {
    let mut g = gateway(Mode::Listen);
    let t = login(&mut g);
    let (_, tok) = call(&mut g, "POST", "/api/v1/tokens", r#"{"name":"claude"}"#, Some(&t));
    let api_token = tok.get("token").unwrap().as_str().unwrap().to_string();
    for path in ["/api/v1/info", "/api/v1/settings", "/api/v1/diagnostics", "/api/v1/tokens"] {
        let r = handle(&mut g, &req("GET", path, "", Some(&t)), 2_000, [6; 32]);
        assert_eq!(r.status, 200, "{path}");
        for secret in ["wifisecret", "mqttsecret", "longpassword", "pbkdf2", api_token.as_str(), t.as_str()] {
            assert!(!r.body.contains(secret), "{path} leaks {secret}");
        }
    }
}

#[test]
fn api_password_and_lockout() {
    let mut g = gateway(Mode::Listen);
    assert_eq!(call(&mut g, "POST", "/api/v1/login", r#"{"password":"x"}"#, None).0, 409);
    assert_eq!(call(&mut g, "POST", "/api/v1/password", r#"{"new":"short"}"#, None).0, 400);
    login(&mut g);
    assert_eq!(call(&mut g, "POST", "/api/v1/password", r#"{"new":"anotherpassword"}"#, None).0, 401);
    // the failed password change above already counts: 4 more failures reach the limit of 5
    for _ in 0..4 {
        assert_eq!(call(&mut g, "POST", "/api/v1/login", r#"{"password":"bad"}"#, None).0, 401);
    }
    assert_eq!(call(&mut g, "POST", "/api/v1/login", r#"{"password":"longpassword"}"#, None).0, 423);
    assert!(g.actions.contains(&Action::SaveSettings));
}

#[test]
fn api_mode_and_actions() {
    let mut g = gateway(Mode::Listen);
    let t = login(&mut g);
    g.info.master_capable = false;
    assert_eq!(call(&mut g, "POST", "/api/v1/mode", r#"{"mode":"master"}"#, Some(&t)).0, 409);
    g.info.master_capable = true;
    assert_eq!(call(&mut g, "POST", "/api/v1/mode", r#"{"mode":"master"}"#, Some(&t)).0, 200);
    assert_eq!(g.settings.mode, Mode::Master);
    assert!(g.actions.contains(&Action::Restart));
    assert_eq!(call(&mut g, "POST", "/api/v1/rediscover", "", Some(&t)).0, 200);
    assert!(g.actions.contains(&Action::Rediscover));
}

#[test]
fn api_settings_update_keeps_secrets_write_only() {
    let mut g = gateway(Mode::Listen);
    let t = login(&mut g);
    let (st, _) = call(&mut g, "POST", "/api/v1/settings", r#"{"mqtt_url":"mqtt://10.0.0.2:1883","renew_days":[0,1,2]}"#, Some(&t));
    assert_eq!(st, 200);
    assert_eq!(g.settings.mqtt_url, "mqtt://10.0.0.2:1883");
    assert_eq!(g.settings.mqtt_pass, "", "a new broker URL drops the stored password");
    assert_eq!(g.settings.renew_days, vec![0, 1, 2]);
    let (_, v) = call(&mut g, "GET", "/api/v1/settings", "", Some(&t));
    assert_eq!(v.get("wifi_pass").unwrap().as_str(), Some("***"));
}

#[test]
fn api_unknown_route_404_and_bad_json_400() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    assert_eq!(call(&mut g, "GET", "/api/v1/nothing", "", None).0, 404);
    assert_eq!(call(&mut g, "POST", "/api/v1/params/zone.setpoint", "{oops", Some(&t)).0, 400);
    assert_eq!(call(&mut g, "POST", "/api/v1/params/zone.nothing", r#"{"system":1}"#, Some(&t)).0, 404);
}

#[test]
fn api_write_result_and_journal_undo() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    let (_, v) = call(&mut g, "POST", "/api/v1/params/system.qadapt", r#"{"system":1,"value":"silent"}"#, Some(&t));
    let id = v.get("queued").unwrap().as_f64().unwrap() as u32;
    let (qid, reqs) = g.queue.remove(0);
    assert_eq!(qid, id);
    assert_eq!(reqs, vec![Request::MaskWrite { addr: 1, reg: 0x0048, and: 0xFFF8, or: 2 }]);
    g.complete(id, airzone_core::master::WriteResult::Applied(vec![0x0002]), 5_000);
    assert_eq!(call(&mut g, "GET", &format!("/api/v1/writes/{id}"), "", Some(&t)).1.get("state").unwrap().as_str(), Some("applied"));
    let (_, j) = call(&mut g, "GET", "/api/v1/journal", "", Some(&t));
    let Value::Arr(entries) = j else { panic!() };
    assert_eq!(entries.len(), 1);
    assert_eq!(call(&mut g, "POST", "/api/v1/journal/0/undo", "", Some(&t)).0, 202);
    assert_eq!(g.queue.last().unwrap().1, vec![Request::MaskWrite { addr: 1, reg: 0x0048, and: 0xFFF8, or: 1 }]);
}


#[test]
fn api_configured_master_not_running_409() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    g.info.master_running = false;
    let (st, _) = call(&mut g, "POST", "/api/v1/params/zone.setpoint", r#"{"system":1,"zone":2,"value":21.5}"#, Some(&t));
    assert_eq!(st, 409);
    assert!(g.queue.is_empty());
    assert_eq!(call(&mut g, "GET", "/api/v1/info", "", None).1.get("mode").unwrap().as_str(), Some("listen"));
}

#[test]
fn api_post_requires_json_and_same_origin() {
    let mut g = gateway(Mode::Listen);
    let mut r = req("POST", "/api/v1/password", r#"{"new":"attackerpw1"}"#, None);
    r.content_type = Some("text/plain".into());
    assert_eq!(handle(&mut g, &r, 1, [1; 32]).status, 415);
    let mut r = req("POST", "/api/v1/password", r#"{"new":"attackerpw1"}"#, None);
    r.host = Some("192.168.1.50".into());
    r.origin = Some("http://evil.example".into());
    assert_eq!(handle(&mut g, &r, 1, [1; 32]).status, 403);
    assert!(!g.auth.has_password());
    let mut r = req("POST", "/api/v1/password", r#"{"new":"ownerpassword"}"#, None);
    r.host = Some("192.168.1.50".into());
    r.origin = Some("http://192.168.1.50".into());
    assert_eq!(handle(&mut g, &r, 1, [1; 32]).status, 200);
}

#[test]
fn journal_undo_unknown_old_refused_and_danger_needs_confirm() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    call(&mut g, "POST", "/api/v1/dev", r#"{"on":true}"#, Some(&t));
    // register never read: old value unknown, undo must refuse
    let (_, v) = call(&mut g, "POST", "/api/v1/raw/write", r#"{"addr":1,"reg":4660,"value":1}"#, Some(&t));
    let id = v.get("queued").unwrap().as_f64().unwrap() as u32;
    g.queue.clear();
    g.complete(id, airzone_core::master::WriteResult::Applied(vec![1]), 2);
    assert_eq!(call(&mut g, "POST", "/api/v1/journal/0/undo", "", Some(&t)).0, 409);
    // DI1 change, confirmed: undo needs the confirmation again
    let (_, v) = call(&mut g, "POST", "/api/v1/raw/write", r#"{"addr":1,"reg":72,"and":61951,"or":512,"confirm":"D1"}"#, Some(&t));
    let id = v.get("queued").unwrap().as_f64().unwrap() as u32;
    g.queue.clear();
    g.complete(id, airzone_core::master::WriteResult::Applied(vec![0x0201]), 3);
    assert_eq!(call(&mut g, "POST", "/api/v1/journal/1/undo", "", Some(&t)).0, 403);
    assert_eq!(call(&mut g, "POST", "/api/v1/journal/1/undo", r#"{"confirm":"D1"}"#, Some(&t)).0, 202);
}

fn entries(v: &Value) -> Vec<Value> {
    match v.get("entries") {
        Some(Value::Arr(e)) => e.clone(),
        _ => panic!("no entries: {v}"),
    }
}

fn arr_len(v: &Value, k: &str) -> usize {
    match v.get(k) {
        Some(Value::Arr(a)) => a.len(),
        _ => panic!("no {k}: {v}"),
    }
}

#[test]
fn backup_roundtrip() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    let (st, b) = call(&mut g, "GET", "/api/v1/backup", "", Some(&t));
    assert_eq!(st, 200);
    assert_eq!(b.get("format").unwrap().as_str(), Some("airzone-gateway-backup"));
    let e = entries(&b);
    assert!(e.iter().any(|x| x.get("id").unwrap().as_str() == Some("system.qadapt") && x.get("value").unwrap().as_str() == Some("power")));
    assert_eq!(e.iter().filter(|x| x.get("id").unwrap().as_str() == Some("zone.name")).count(), 6);
    assert!(e.iter().all(|x| x.get("id").unwrap().as_str() != Some("zone.setpoint")), "tier 1 is not configuration");
    // restoring the same file changes nothing
    let (st, r) = call(&mut g, "POST", "/api/v1/restore", &b.to_string(), Some(&t));
    assert_eq!(st, 200, "{r}");
    assert_eq!(arr_len(&r, "queued"), 0);
    assert!(g.queue.is_empty());
    // one changed value is queued
    let edited = b.to_string().replace(r#""id":"system.qadapt","system":1,"value":"power""#, r#""id":"system.qadapt","system":1,"value":"silent""#);
    assert_ne!(edited, b.to_string());
    let (_, r) = call(&mut g, "POST", "/api/v1/restore", &edited, Some(&t));
    assert_eq!(arr_len(&r, "queued"), 1);
    assert_eq!(g.queue[0].1, vec![Request::MaskWrite { addr: 1, reg: 0x0048, and: 0xFFF8, or: 2 }]);
}

#[test]
fn restore_skips_absent_targets() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    let body = r#"{"format":"airzone-gateway-backup","version":1,"entries":[{"id":"zone.antifreeze","system":1,"zone":9,"value":true},{"id":"system.qadapt","system":2,"value":"silent"}]}"#;
    let (st, r) = call(&mut g, "POST", "/api/v1/restore", body, Some(&t));
    assert_eq!(st, 200);
    assert_eq!(arr_len(&r, "queued"), 0);
    assert_eq!(arr_len(&r, "skipped"), 2);
    assert!(r.to_string().contains("absent"));
    assert!(g.queue.is_empty());
}

#[test]
fn restore_reports_bad_entries() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    let body = r#"{"format":"airzone-gateway-backup","version":1,"entries":[
        {"id":"system.nothing","system":1,"value":1},
        {"id":"system.filter_hours","system":1,"value":50},
        {"id":"zone.antifreeze","system":1,"zone":2,"value":"maybe"},
        {"id":"zone.setpoint","system":1,"zone":2,"value":22},
        {"id":"zone.antifreeze","system":1,"zone":2,"value":true}]}"#;
    let (st, r) = call(&mut g, "POST", "/api/v1/restore", body, Some(&t));
    assert_eq!(st, 200, "{r}");
    assert_eq!(arr_len(&r, "skipped"), 4, "{r}");
    assert_eq!(arr_len(&r, "queued"), 1, "{r}");
    assert_eq!(call(&mut g, "POST", "/api/v1/restore", r#"{"format":"other","entries":[]}"#, Some(&t)).0, 400);
}

#[test]
fn restore_danger_needs_confirm_list() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    let e = r#"{"id":"system.d1_input","system":1,"value":"bridge alarm"}"#;
    let (_, r) = call(&mut g, "POST", "/api/v1/restore", &format!(r#"{{"format":"airzone-gateway-backup","version":1,"entries":[{e}]}}"#), Some(&t));
    assert_eq!(arr_len(&r, "queued"), 0);
    assert!(r.to_string().contains("confirmation"));
    let (_, r) = call(&mut g, "POST", "/api/v1/restore", &format!(r#"{{"format":"airzone-gateway-backup","version":1,"entries":[{e}],"confirm":["D1"]}}"#), Some(&t));
    assert_eq!(arr_len(&r, "queued"), 1);
}

#[test]
fn restore_listen_mode_409() {
    let mut g = gateway(Mode::Listen);
    let t = login(&mut g);
    assert_eq!(call(&mut g, "GET", "/api/v1/backup", "", Some(&t)).0, 200);
    assert_eq!(call(&mut g, "POST", "/api/v1/restore", r#"{"format":"airzone-gateway-backup","version":1,"entries":[]}"#, Some(&t)).0, 409);
}

#[test]
fn api_log_level() {
    let mut g = gateway(Mode::Listen);
    let t = login(&mut g);
    assert_eq!(call(&mut g, "GET", "/api/v1/log", "", Some(&t)).1.get("level").unwrap().as_str(), Some("info"));
    assert_eq!(call(&mut g, "POST", "/api/v1/log", r#"{"level":"debug"}"#, Some(&t)).0, 200);
    assert_eq!(g.log_level, "debug");
    assert_eq!(call(&mut g, "POST", "/api/v1/log", r#"{"level":"loud"}"#, Some(&t)).0, 400);
    assert_eq!(call(&mut g, "POST", "/api/v1/log", r#"{"level":"warn"}"#, None).0, 401);
}

#[test]
fn ws_messages_shape() {
    use airzone_core::ws::{live_messages, WsCursor};
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    let (_, v) = call(&mut g, "POST", "/api/v1/params/zone.setpoint", r#"{"system":1,"zone":2,"value":21.5}"#, Some(&t));
    let id = v.get("queued").unwrap().as_f64().unwrap() as u32;
    g.complete(id, airzone_core::master::WriteResult::Applied(vec![215]), 10);
    g.push_frame("01 03 00 00 00 50 44 36".into());
    let mut cursor = WsCursor::default();
    let msgs = live_messages(&mut g, &mut cursor, "{\"topic\":\"values\",\"states\":[]}".into(), 20);
    let topics: Vec<String> = msgs.iter().map(|m| parse(m).unwrap().get("topic").unwrap().as_str().unwrap().to_string()).collect();
    assert_eq!(topics, vec!["values", "events"], "no dev mode: no frames/registers");
    assert!(msgs[1].contains("applied"));
    assert_eq!(live_messages(&mut g, &mut cursor, "{\"topic\":\"values\",\"states\":[]}".into(), 30).len(), 1, "events are sent once");
    cursor.ms = 0;
    g.dev_mode = true;
    g.push_frame("01 03 02 00 05 78 47".into());
    let msgs = live_messages(&mut g, &mut cursor, "{\"topic\":\"values\",\"states\":[]}".into(), 40);
    let topics: Vec<String> = msgs.iter().map(|m| parse(m).unwrap().get("topic").unwrap().as_str().unwrap().to_string()).collect();
    assert_eq!(topics, vec!["values", "frames", "registers"]);
}

#[test]
fn api_info_latest_version_and_setting() {
    let mut g = gateway(Mode::Listen);
    g.info.latest_version = Some("9.0.0".into());
    g.info.latest_url = Some("https://example.org/fw.bin".into());
    let (_, v) = call(&mut g, "GET", "/api/v1/info", "", None);
    assert_eq!(v.get("latest_version").unwrap().as_str(), Some("9.0.0"));
    assert_eq!(v.get("latest_url").unwrap().as_str(), Some("https://example.org/fw.bin"));
    let t = login(&mut g);
    assert_eq!(call(&mut g, "POST", "/api/v1/settings", r#"{"update_manifest":"https://example.org/manifest.json"}"#, Some(&t)).0, 200);
    assert_eq!(g.settings.update_manifest, "https://example.org/manifest.json");
}

#[test]
fn restore_outlets_master_commands_and_equipment_checked() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    let body = r#"{"format":"airzone-gateway-backup","version":1,"confirm":["OUTLET","MASTER","RESET FILTER"],"entries":[
        {"id":"outlet.zone","system":1,"outlet":7,"value":9},
        {"id":"outlet.zone","system":1,"outlet":7,"value":3},
        {"id":"zone.master","system":1,"zone":1,"value":true},
        {"id":"system.filter_reset","system":1,"value":true},
        {"id":"iaq.renew_interval","system":1,"value":30}]}"#;
    let (st, r) = call(&mut g, "POST", "/api/v1/restore", body, Some(&t));
    assert_eq!(st, 200, "{r}");
    let text = r.to_string();
    assert_eq!(arr_len(&r, "queued"), 1, "{text}");
    assert_eq!(g.queue[0].1, vec![Request::MaskWrite { addr: 1, reg: 0x2F00, and: 0xFF00, or: 3 }]);
    assert!(text.contains("absent zone") && text.contains("set by hand") && text.contains("command") && text.contains("equipment"), "{text}");
}

#[test]
fn large_restore_keeps_every_queued_write() {
    let mut g = gateway(Mode::Master);
    let t = login(&mut g);
    let mut ids = Vec::new();
    for i in 0..80 {
        let v = if i % 2 == 0 { "silent" } else { "power" };
        let (_, r) = call(&mut g, "POST", "/api/v1/params/system.qadapt", &format!(r#"{{"system":1,"value":"{v}"}}"#), Some(&t));
        ids.push(r.get("queued").unwrap().as_f64().unwrap() as u32);
    }
    for id in &ids {
        assert_eq!(call(&mut g, "GET", &format!("/api/v1/writes/{id}"), "", Some(&t)).0, 200, "write {id} forgotten while queued");
    }
    g.complete(ids[0], airzone_core::master::WriteResult::Applied(vec![2]), 5);
    let Value::Arr(j) = call(&mut g, "GET", "/api/v1/journal", "", Some(&t)).1 else { panic!() };
    assert_eq!(j.len(), 1, "the first write was journaled");
}
