//! The local HTTP API (`/api/v1/…`) as one pure function. Reads need no token; writes need a bearer
//! token (session or API token); raw access also needs developer mode; bus writes need master mode.

use crate::catalogue::{param, Kind, Param, Scope, PARAMS};
use crate::frame::Request;
use crate::gateway::{Action, Gateway, Note, WriteState};
use crate::ha::{js, json_value};
use crate::json::{self, Value};
use crate::master::WriteResult;
use crate::model::{block_base, decode, encode, mask_of, params_at, EncodeError, Input, Target, Value as V};
use crate::rules::set_mode;
use crate::settings::Mode;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApiRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: String,
    pub bearer: Option<String>,
    /// `Content-Type` header (POST / DELETE must be `application/json`: forces a CORS preflight).
    pub content_type: Option<String>,
    /// `Host` and `Origin` headers: a cross-site `Origin` is refused on writes.
    pub host: Option<String>,
    pub origin: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

fn ok(body: String) -> ApiResponse {
    ApiResponse { status: 200, content_type: "application/json", body }
}

fn status(code: u16, body: String) -> ApiResponse {
    ApiResponse { status: code, content_type: "application/json", body }
}

fn error(code: u16, msg: &str) -> ApiResponse {
    status(code, format!(r#"{{"error":{}}}"#, js(msg)))
}

fn obj(fields: &[(&str, String)]) -> String {
    let f: Vec<String> = fields.iter().map(|(k, v)| format!("{}:{v}", js(k))).collect();
    format!("{{{}}}", f.join(","))
}

fn query(q: &str, key: &str) -> Option<u32> {
    q.split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).and_then(|(_, v)| v.parse().ok())
}

fn num(v: &Value, k: &str) -> Option<u32> {
    v.get(k).and_then(Value::as_f64).filter(|x| *x >= 0.0 && x.fract() == 0.0 && *x <= u32::MAX as f64).map(|x| x as u32)
}

fn values_of(g: &Gateway, scope: Scope, t: &Target) -> String {
    let mut f = Vec::new();
    for p in PARAMS.iter().filter(|p| p.scope == scope) {
        let Some(base) = block_base(p, t) else { continue };
        let v = g.registry.get_range(t.addr(), base + p.reg, p.length.max(1) as u16).map(|raw| decode(p, &raw)).unwrap_or(V::Unknown(0xFFFF));
        f.push(format!("{}:{}", js(p.id.split_once('.').map(|x| x.1).unwrap_or(p.id)), json_value(&v)));
    }
    format!("{{{}}}", f.join(","))
}

fn info(g: &Gateway) -> String {
    let i = &g.info;
    obj(&[
        ("version", js(&i.version)),
        ("build", js(&i.build)),
        ("board", js(&i.board)),
        ("mac", js(&i.mac)),
        ("ip", js(&i.ip)),
        ("rssi", i.rssi.map(|r| r.to_string()).unwrap_or_else(|| "null".into())),
        ("name", js(&g.settings.gateway_name)),
        ("mode", js(if g.master_active() { "master" } else { "listen" })),
        ("configured_mode", js(if g.settings.mode == Mode::Master { "master" } else { "listen" })),
        ("master_capable", i.master_capable.to_string()),
        ("master_running", i.master_running.to_string()),
        ("dev_mode", g.dev_mode.to_string()),
        ("password_set", g.auth.has_password().to_string()),
        ("mqtt_configured", (!g.settings.mqtt_url.is_empty()).to_string()),
        ("mqtt_connected", i.mqtt_connected.to_string()),
        ("uptime_s", i.uptime_s.to_string()),
        ("time", js(&i.time)),
        ("stream_dropped", i.stream_dropped.to_string()),
        ("listen_dropped", i.listen_dropped.to_string()),
        ("latest_version", i.latest_version.as_deref().map(js).unwrap_or_else(|| "null".into())),
        ("latest_url", i.latest_url.as_deref().map(js).unwrap_or_else(|| "null".into())),
        ("update_state", js(&i.update_state)),
        ("reset_reason", js(&i.reset_reason)),
        ("free_heap", i.free_heap.to_string()),
        ("min_free_heap", i.min_free_heap.to_string()),
        ("last_panic", js(&i.last_panic)),
        ("bus_read_errors", i.bus_read_errors.to_string()),
        ("write_failures", i.write_failures.to_string()),
    ])
}

fn systems(g: &Gateway) -> String {
    let cs: Vec<String> = g
        .topology
        .centrals
        .iter()
        .map(|c| {
            let zones: Vec<String> =
                c.zones.iter().map(|z| obj(&[("n", z.n.to_string()), ("name", js(&z.name)), ("master", z.master.to_string())])).collect();
            obj(&[("addr", c.addr.to_string()), ("iaq", c.iaq.to_string()), ("zones", format!("[{}]", zones.join(",")))])
        })
        .collect();
    format!("[{}]", cs.join(","))
}

fn settings_json(g: &Gateway) -> String {
    let f: Vec<(&str, String)> = g.settings.redacted().into_iter().map(|(k, v)| (k, js(&v))).collect();
    obj(&f)
}

fn input_for(p: &Param, v: &Value) -> Option<Input> {
    Some(match (p.kind, v) {
        (Kind::Number, Value::Num(x)) => Input::Num(*x as f32),
        (Kind::Bool, Value::Bool(b)) => Input::Bool(*b),
        (Kind::Enum, Value::Num(x)) if x.fract() == 0.0 && *x >= 0.0 => Input::Choice(*x as u16),
        (Kind::Enum, Value::Str(s)) => Input::Choice(p.values.iter().find(|(_, n)| n == s)?.0),
        (Kind::Text, Value::Str(s)) => Input::Text(s.clone()),
        (Kind::Command, _) => Input::Trigger,
        _ => return None,
    })
}

fn encode_error(e: EncodeError, p: &Param) -> ApiResponse {
    match e {
        EncodeError::ConfirmRequired => {
            let d = p.danger.expect("danger param");
            status(403, obj(&[("error", js("confirmation required")), ("warning", js(d.warning)), ("confirm", js(d.confirm))]))
        }
        EncodeError::NotWritable | EncodeError::NotConfirmed => error(403, "read-only parameter"),
        EncodeError::WrongTarget => error(404, "no such system / zone / outlet for this parameter"),
        EncodeError::OffStep => error(400, &format!("value must be a multiple of the step {}", p.scale)),
        other => error(400, &format!("{other:?}")),
    }
}

/// Journal notes for requests (old values from the registry).
fn notes(g: &Gateway, id: &str, reqs: &[Request]) -> Vec<Note> {
    reqs.iter()
        .filter_map(|r| match *r {
            Request::WriteSingle { addr, reg, .. } => Some((addr, reg, 0xFFFF)),
            Request::MaskWrite { addr, reg, and, .. } => Some((addr, reg, !and)),
            _ => None,
        })
        .map(|(addr, reg, mask)| Note { id: id.to_string(), addr, reg, old: g.registry.get(addr, reg), mask })
        .collect()
}

fn queued(g: &mut Gateway, id: &str, reqs: Vec<Request>) -> ApiResponse {
    let n = notes(g, id, &reqs);
    let wid = g.enqueue(reqs, n);
    status(202, obj(&[("queued", wid.to_string())]))
}

fn write_param(g: &mut Gateway, p: &'static Param, body: &Value) -> ApiResponse {
    let Some(addr) = num(body, "system").filter(|a| (1..=32).contains(a)).map(|a| a as u8) else {
        return error(400, "system (1-32) required");
    };
    let target = match p.scope {
        Scope::System | Scope::Iaq => Target::System { addr },
        Scope::Zone => match num(body, "zone") {
            Some(z) if z <= 255 => Target::Zone { addr, zone: z as u8 },
            _ => return error(400, "zone required"),
        },
        Scope::Outlet => match num(body, "outlet") {
            Some(o) if o <= 255 => Target::Outlet { addr, outlet: o as u8 },
            _ => return error(400, "outlet required"),
        },
    };
    let Some(input) = body.get("value").and_then(|v| input_for(p, v)) else { return error(400, "bad value for this parameter") };
    let confirm = body.get("confirm").and_then(Value::as_str);
    // the central acknowledges writes for absent or switched-off equipment without applying them;
    // switching something off stays allowed
    let Some(c) = g.topology.centrals.iter().find(|c| c.addr == addr) else {
        return error(409, "system not discovered yet");
    };
    if !matches!(input, Input::Bool(false)) && !crate::ha::requires_ok(p, c, &g.registry) {
        return error(409, "equipment absent or switched off for this parameter (e.g. VMC off)");
    }
    if p.id == "zone.mode" {
        let Some(c) = g.topology.centrals.iter().find(|c| c.addr == addr) else { return error(404, "no such system") };
        let Input::Choice(m) = input else { return error(400, "bad mode") };
        return match set_mode(c, &g.registry, m) {
            Ok(reqs) => queued(g, p.id, reqs),
            Err(e) => encode_error(e, p),
        };
    }
    match encode(p, &target, &input, confirm) {
        Ok(reqs) => queued(g, p.id, reqs),
        Err(e) => encode_error(e, p),
    }
}

fn raw_write(g: &mut Gateway, body: &Value) -> ApiResponse {
    let (Some(addr), Some(reg)) = (num(body, "addr"), num(body, "reg")) else { return error(400, "addr and reg required") };
    if !(1..=32).contains(&addr) || reg > 0xFFFF {
        return error(400, "bad address or register");
    }
    let (addr, reg) = (addr as u8, reg as u16);
    let (req, touched) = if let Some(v) = num(body, "value") {
        if v > 0xFFFF {
            return error(400, "value > 0xFFFF");
        }
        (Request::WriteSingle { addr, reg, value: v as u16 }, 0xFFFF)
    } else if let (Some(and), Some(or)) = (num(body, "and"), num(body, "or")) {
        if and > 0xFFFF || or > 0xFFFF {
            return error(400, "mask > 0xFFFF");
        }
        (Request::MaskWrite { addr, reg, and: and as u16, or: or as u16 }, !(and as u16))
    } else {
        return error(400, "value or and/or required");
    };
    let confirm = body.get("confirm").and_then(Value::as_str);
    if let Some(d) = danger_touched(reg, touched, confirm) {
        return d;
    }
    queued(g, "raw", vec![req])
}

/// 403 when bits `touched` of register `reg` belong to a dangerous parameter and `confirm` is not its word.
fn danger_touched(reg: u16, touched: u16, confirm: Option<&str>) -> Option<ApiResponse> {
    for (p, _) in params_at(reg) {
        if let Some(d) = p.danger {
            if mask_of(p) & touched != 0 && confirm != Some(d.confirm) {
                return Some(status(
                    403,
                    obj(&[("error", js("touches a dangerous setting")), ("param", js(p.id)), ("warning", js(d.warning)), ("confirm", js(d.confirm))]),
                ));
            }
        }
    }
    None
}

/// Writes must come as JSON (forces a CORS preflight that the gateway never grants) and, when the browser
/// says where it comes from, from the gateway's own origin.
fn cross_site(r: &ApiRequest) -> Option<ApiResponse> {
    if r.method != "POST" && r.method != "DELETE" {
        return None;
    }
    if !r.content_type.as_deref().unwrap_or("").to_ascii_lowercase().starts_with("application/json") {
        return Some(error(415, "Content-Type must be application/json"));
    }
    if let Some(origin) = r.origin.as_deref() {
        let o = origin.split_once("://").map(|x| x.1).unwrap_or(origin);
        if r.host.as_deref() != Some(o) {
            return Some(error(403, "cross-site request refused"));
        }
    }
    None
}

const BACKUP_FORMAT: &str = "airzone-gateway-backup";

/// Every configuration parameter (tiers 2-3, writable, confirmed) of every present target.
fn backup_json(g: &Gateway, now_ms: u64) -> String {
    let mut entries = Vec::new();
    for c in &g.topology.centrals {
        let mut targets: Vec<(Scope, Target, String)> = vec![(Scope::System, Target::System { addr: c.addr }, String::new())];
        if c.iaq {
            targets.push((Scope::Iaq, Target::System { addr: c.addr }, String::new()));
        }
        for z in &c.zones {
            targets.push((Scope::Zone, Target::Zone { addr: c.addr, zone: z.n }, format!(r#","zone":{}"#, z.n)));
        }
        for o in 1..=8u8 {
            targets.push((Scope::Outlet, Target::Outlet { addr: c.addr, outlet: o }, format!(r#","outlet":{o}"#)));
        }
        for (scope, t, extra) in &targets {
            for p in PARAMS.iter().filter(|p| {
                p.scope == *scope && p.writable && p.confidence == crate::catalogue::Confidence::Confirmed && p.tier >= 2 && p.kind != Kind::Command
            }) {
                if !crate::ha::requires_ok(p, c, &g.registry) {
                    continue;
                }
                let Some(base) = block_base(p, t) else { continue };
                let Some(raw) = g.registry.get_range(c.addr, base + p.reg, p.length.max(1) as u16) else { continue };
                let v = decode(p, &raw);
                if matches!(v, V::Unknown(_)) {
                    continue;
                }
                entries.push(format!(r#"{{"id":{},"system":{}{extra},"value":{}}}"#, js(p.id), c.addr, json_value(&v)));
            }
        }
    }
    format!(r#"{{"format":{},"version":1,"created_ms":{now_ms},"gateway":{},"entries":[{}]}}"#, js(BACKUP_FORMAT), js(&g.info.mac), entries.join(","))
}

/// Queue the writes needed to bring the installation to a backup; report every entry not applied.
fn restore(g: &mut Gateway, body: &Value) -> ApiResponse {
    if body.get("format").and_then(Value::as_str) != Some(BACKUP_FORMAT) {
        return error(400, "not an airzone-gateway backup");
    }
    let Some(Value::Arr(entries)) = body.get("entries") else { return error(400, "entries required") };
    let confirms: Vec<String> = match body.get("confirm") {
        Some(Value::Arr(a)) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
        _ => Vec::new(),
    };
    let mut queued_ids = Vec::new();
    let mut skipped = Vec::new();
    let skip = |skipped: &mut Vec<String>, id: &str, why: &str| skipped.push(obj(&[("id", js(id)), ("reason", js(why))]));
    for e in entries {
        let id = e.get("id").and_then(Value::as_str).unwrap_or("");
        let Some(p) = param(id).filter(|p| p.writable && p.confidence == crate::catalogue::Confidence::Confirmed && p.tier >= 2) else {
            skip(&mut skipped, id, "unknown or read-only parameter");
            continue;
        };
        let addr = num(e, "system").filter(|a| (1..=32).contains(a)).map(|a| a as u8);
        if p.kind == Kind::Command {
            skip(&mut skipped, id, "command, not restored");
            continue;
        }
        if p.id == "zone.master" {
            skip(&mut skipped, id, "master zone is set by hand");
            continue;
        }
        let Some(central) = addr.and_then(|a| g.topology.centrals.iter().find(|c| c.addr == a)).cloned() else {
            skip(&mut skipped, id, "absent system");
            continue;
        };
        if !crate::ha::requires_ok(p, &central, &g.registry) {
            skip(&mut skipped, id, "equipment absent or off");
            continue;
        }
        let target = match p.scope {
            Scope::System => Target::System { addr: central.addr },
            Scope::Iaq if central.iaq => Target::System { addr: central.addr },
            Scope::Zone => match num(e, "zone").filter(|z| central.zones.iter().any(|x| x.n as u32 == *z)) {
                Some(z) => Target::Zone { addr: central.addr, zone: z as u8 },
                None => {
                    skip(&mut skipped, id, "absent zone");
                    continue;
                }
            },
            Scope::Outlet => match num(e, "outlet")
                .filter(|o| (1..=8).contains(o) && g.registry.get(central.addr, 0x2900 + 0x100 * (*o as u16 - 1)).is_some_and(|w| w != 0xFFFF))
            {
                Some(o) => Target::Outlet { addr: central.addr, outlet: o as u8 },
                None => {
                    skip(&mut skipped, id, "absent outlet");
                    continue;
                }
            },
            Scope::Iaq => {
                skip(&mut skipped, id, "absent IAQ sensor");
                continue;
            }
        };
        let Some(v) = e.get("value") else {
            skip(&mut skipped, id, "no value");
            continue;
        };
        let Some(input) = input_for(p, v) else {
            skip(&mut skipped, id, "bad value type");
            continue;
        };
        // an outlet is only linked to a zone that exists here
        if p.id == "outlet.zone" {
            if let Input::Num(z) = input {
                if z != 0.0 && !central.zones.iter().any(|x| x.n as f32 == z) {
                    skip(&mut skipped, id, "absent zone");
                    continue;
                }
            }
        }
        // unchanged values are not written
        let base = block_base(p, &target).expect("target checked");
        if let Some(raw) = g.registry.get_range(central.addr, base + p.reg, p.length.max(1) as u16) {
            if json_value(&decode(p, &raw)) == v.to_string() {
                continue;
            }
        }
        let confirm = p.danger.and_then(|d| confirms.iter().find(|c| *c == d.confirm)).map(String::as_str);
        match encode(p, &target, &input, confirm) {
            Ok(reqs) => {
                let n = notes(g, p.id, &reqs);
                queued_ids.push(g.enqueue(reqs, n).to_string());
            }
            Err(EncodeError::ConfirmRequired) => skip(&mut skipped, id, "confirmation required (add its word to \"confirm\")"),
            Err(err) => skip(&mut skipped, id, &format!("{err:?}")),
        }
    }
    ok(obj(&[("queued", format!("[{}]", queued_ids.join(","))), ("skipped", format!("[{}]", skipped.join(",")))]))
}

fn write_state(g: &Gateway, id: u32) -> ApiResponse {
    let Some((st, _)) = g.writes.get(&id) else { return error(404, "unknown write") };
    let (state, extra) = match st {
        WriteState::Queued => ("queued", String::new()),
        WriteState::Done(WriteResult::Applied(v)) => {
            ("applied", format!(r#","values":[{}]"#, v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")))
        }
        WriteState::Done(WriteResult::Rejected { code, sent }) => ("rejected", format!(r#","code":{code},"sent":{sent}"#)),
        WriteState::Done(WriteResult::NoAnswer { sent }) => ("no_answer", format!(r#","sent":{sent}"#)),
        WriteState::Done(WriteResult::Unverified) => ("unverified", String::new()),
    };
    ok(format!(r#"{{"id":{id},"state":{}{extra}}}"#, js(state)))
}

fn registry_json(g: &Gateway) -> String {
    let mut out = Vec::new();
    for c in &g.topology.centrals {
        let regs: Vec<String> = g.registry.dump(c.addr).iter().map(|(r, v)| format!(r#""0x{r:04X}":{v}"#)).collect();
        out.push(format!(r#""{}":{{{}}}"#, c.addr, regs.join(",")));
    }
    format!("{{{}}}", out.join(","))
}

fn journal_json(g: &Gateway) -> String {
    let e: Vec<String> = g
        .journal
        .list()
        .iter()
        .enumerate()
        .map(|(i, e)| {
            obj(&[
                ("index", i.to_string()),
                ("id", js(&e.id)),
                ("addr", e.addr.to_string()),
                ("reg", e.reg.to_string()),
                ("old", e.old.map(|o| o.to_string()).unwrap_or_else(|| "null".into())),
                ("new", e.new.to_string()),
                ("mask", e.mask.to_string()),
                ("t_ms", e.t_ms.to_string()),
            ])
        })
        .collect();
    format!("[{}]", e.join(","))
}

fn apply_settings(g: &mut Gateway, body: &Value) -> ApiResponse {
    let Value::Obj(fields) = body else { return error(400, "object expected") };
    let mut kv: Vec<(String, String)> = g.settings.to_kv().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    let old_url = g.settings.mqtt_url.clone();
    const ALLOWED: [&str; 14] = [
        "wifi_ssid", "wifi_pass", "mqtt_url", "mqtt_user", "mqtt_pass", "mqtt_prefix", "gateway_name", "hostname", "renew_days",
        "fcool_months", "timezone", "heartbeat", "identification", "update_manifest",
    ];
    for (k, v) in fields {
        if !ALLOWED.contains(&k.as_str()) && k != "identity_name" {
            return error(400, &format!("unknown or read-only setting {k}"));
        }
        let text = match v {
            Value::Str(s) => s.clone(),
            Value::Bool(b) => (*b as u8).to_string(),
            Value::Num(x) => x.to_string(),
            Value::Arr(a) => a.iter().filter_map(Value::as_f64).map(|x| (x as i64).to_string()).collect::<Vec<_>>().join(","),
            _ => return error(400, &format!("bad value for {k}")),
        };
        if let Some(e) = kv.iter_mut().find(|(n, _)| n == k) {
            e.1 = text;
        }
    }
    let mut s = crate::settings::Settings::from_kv(&kv);
    if s.mqtt_url != old_url && !fields.iter().any(|(k, v)| k == "mqtt_pass" && v.as_str().is_some_and(|x| !x.is_empty())) {
        s.mqtt_pass.clear();
    }
    s.mode = g.settings.mode;
    g.settings = s;
    g.actions.push(Action::SaveSettings);
    ok(obj(&[("saved", "true".into()), ("restart_needed", "true".into())]))
}

/// Handle one API request. `random` is fresh entropy (session / token / salt generation).
pub fn handle(g: &mut Gateway, r: &ApiRequest, now_ms: u64, random: [u8; 32]) -> ApiResponse {
    let Some(path) = r.path.strip_prefix("/api/v1/") else { return error(404, "not found") };
    if let Some(refused) = cross_site(r) {
        return refused;
    }
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    let authed = r.bearer.as_deref().is_some_and(|t| g.auth.bearer_ok(t, now_ms));
    let body = if r.body.trim().is_empty() {
        Value::Obj(Vec::new())
    } else {
        match json::parse(&r.body) {
            Ok(v) => v,
            Err(e) => return error(400, &format!("invalid JSON: {e:?}")),
        }
    };
    let m = r.method.as_str();
    // public endpoints
    match (m, parts.as_slice()) {
        ("GET", ["info"]) => return ok(info(g)),
        ("GET", ["systems"]) => return ok(systems(g)),
        ("GET", ["catalogue"]) => return ok(g.catalogue_json.to_string()),
        ("GET", ["systems", c]) => {
            let Some(c) = c.parse::<u8>().ok().filter(|c| g.topology.centrals.iter().any(|x| x.addr == *c)) else { return error(404, "no such system") };
            let t = Target::System { addr: c };
            let outlets: Vec<String> = (1..=8u8)
                .map(|o| obj(&[("n", o.to_string()), ("values", values_of(g, Scope::Outlet, &Target::Outlet { addr: c, outlet: o }))]))
                .collect();
            return ok(obj(&[
                ("addr", c.to_string()),
                ("values", values_of(g, Scope::System, &t)),
                ("iaq", values_of(g, Scope::Iaq, &t)),
                ("outlets", format!("[{}]", outlets.join(","))),
            ]));
        }
        ("GET", ["systems", c, "zones", n]) => {
            let (Ok(c), Ok(n)) = (c.parse::<u8>(), n.parse::<u8>()) else { return error(404, "not found") };
            let Some(z) = g.topology.centrals.iter().find(|x| x.addr == c).and_then(|x| x.zones.iter().find(|z| z.n == n)) else {
                return error(404, "no such zone");
            };
            let (name, master) = (z.name.clone(), z.master);
            return ok(obj(&[
                ("n", n.to_string()),
                ("name", js(&name)),
                ("master", master.to_string()),
                ("values", values_of(g, Scope::Zone, &Target::Zone { addr: c, zone: n })),
            ]));
        }
        ("GET", ["params", id]) => {
            let Some(p) = param(id) else { return error(404, "unknown parameter") };
            let addr = query(&r.query, "system").unwrap_or(1) as u8;
            let t = match p.scope {
                Scope::Zone => Target::Zone { addr, zone: query(&r.query, "zone").unwrap_or(0) as u8 },
                Scope::Outlet => Target::Outlet { addr, outlet: query(&r.query, "outlet").unwrap_or(0) as u8 },
                _ => Target::System { addr },
            };
            let Some(base) = block_base(p, &t) else { return error(404, "zone / outlet required") };
            let raw = g.registry.get_range(addr, base + p.reg, p.length.max(1) as u16);
            let v = raw.as_ref().map(|r| decode(p, r)).unwrap_or(V::Unknown(0xFFFF));
            let raw_json = raw.map(|r| format!("[{}]", r.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","))).unwrap_or_else(|| "null".into());
            return ok(obj(&[("id", js(p.id)), ("value", json_value(&v)), ("raw", raw_json), ("writable", (p.writable && p.confidence == crate::catalogue::Confidence::Confirmed).to_string())]));
        }
        ("POST", ["password"]) => {
            let Some(new) = body.get("new").and_then(Value::as_str) else { return error(400, "new required") };
            if new.chars().count() < 8 {
                return error(400, "password must have at least 8 characters");
            }
            if g.auth.has_password() {
                let old = body.get("old").and_then(Value::as_str).unwrap_or("");
                match g.auth.check_password(old, now_ms) {
                    Ok(()) => {}
                    Err(crate::auth::AuthError::Locked { .. }) => return error(423, "locked, try again later"),
                    Err(_) => return error(401, "wrong password"),
                }
            }
            let mut salt = [0u8; 16];
            salt.copy_from_slice(&random[..16]);
            g.auth.set_password(new, salt);
            g.sync_auth_into_settings();
            g.actions.push(Action::SaveSettings);
            return ok(obj(&[("password_set", "true".into())]));
        }
        ("POST", ["login"]) => {
            let pw = body.get("password").and_then(Value::as_str).unwrap_or("");
            return match g.auth.login(pw, now_ms, random) {
                Ok(t) => ok(obj(&[("token", js(&t))])),
                Err(crate::auth::AuthError::NoPassword) => error(409, "no admin password set yet"),
                Err(crate::auth::AuthError::Locked { .. }) => error(423, "locked, try again later"),
                Err(_) => error(401, "wrong password"),
            };
        }
        _ => {}
    }
    if !authed {
        let known = matches!(
            parts.first().copied(),
            Some("params" | "writes" | "journal" | "settings" | "diagnostics" | "tokens" | "dev" | "raw" | "registry" | "mode" | "rediscover" | "restart" | "update" | "logout" | "backup" | "restore" | "log")
        );
        return if known { error(401, "authentication required") } else { error(404, "not found") };
    }
    match (m, parts.as_slice()) {
        ("POST", ["logout"]) => {
            if let Some(t) = &r.bearer {
                g.auth.logout(t);
            }
            ok(obj(&[("logged_out", "true".into())]))
        }
        ("POST", ["params", id]) => {
            let Some(p) = param(id) else { return error(404, "unknown parameter") };
            if !g.master_active() {
                return error(409, "listen mode: the gateway does not write on the bus");
            }
            write_param(g, p, &body)
        }
        ("GET", ["writes", id]) => id.parse().map(|id| write_state(g, id)).unwrap_or_else(|_| error(404, "unknown write")),
        ("GET", ["journal"]) => ok(journal_json(g)),
        ("GET", ["backup"]) => ok(backup_json(g, now_ms)),
        ("GET", ["log"]) => ok(obj(&[("level", js(&g.log_level))])),
        ("POST", ["log"]) => match body.get("level").and_then(Value::as_str) {
            Some(l @ ("error" | "warn" | "info" | "debug")) => {
                g.log_level = l.to_string();
                ok(obj(&[("level", js(l))]))
            }
            _ => error(400, "level must be error, warn, info or debug"),
        },
        ("POST", ["restore"]) => {
            if !g.master_active() {
                return error(409, "listen mode: the gateway does not write on the bus");
            }
            restore(g, &body)
        }
        ("POST", ["journal", i, "undo"]) => {
            if !g.master_active() {
                return error(409, "listen mode: the gateway does not write on the bus");
            }
            let Some(i) = i.parse::<usize>().ok().filter(|i| g.journal.get(*i).is_some()) else { return error(404, "no such journal entry") };
            let e = g.journal.get(i).cloned().expect("checked");
            if e.old.is_none() {
                return error(409, "the old value of this change is unknown: cannot undo");
            }
            // undoing a dangerous setting needs its confirmation again
            let confirm = body.get("confirm").and_then(Value::as_str);
            if let Some(d) = danger_touched(e.reg, e.mask, confirm) {
                return d;
            }
            match g.journal.undo(i) {
                Some(reqs) => queued(g, "undo", reqs),
                None => error(409, "cannot undo"),
            }
        }
        ("GET", ["settings"]) => ok(settings_json(g)),
        ("POST", ["settings"]) => apply_settings(g, &body),
        ("GET", ["diagnostics"]) => ok(obj(&[
            ("info", info(g)),
            ("settings", settings_json(g)),
            ("systems", systems(g)),
            ("registry", registry_json(g)),
            ("journal", journal_json(g)),
        ])),
        ("GET", ["tokens"]) => ok(format!("[{}]", g.auth.token_names().iter().map(|n| js(n)).collect::<Vec<_>>().join(","))),
        ("POST", ["tokens"]) => {
            let name = body.get("name").and_then(Value::as_str).unwrap_or("");
            match g.auth.create_token(name, random) {
                Some(t) => {
                    g.sync_auth_into_settings();
                    g.actions.push(Action::SaveSettings);
                    status(201, obj(&[("name", js(name)), ("token", js(&t))]))
                }
                None => error(400, "bad name, name taken or too many tokens"),
            }
        }
        ("DELETE", ["tokens", name]) => {
            if g.auth.revoke(name) {
                g.sync_auth_into_settings();
                g.actions.push(Action::SaveSettings);
                ok(obj(&[("revoked", js(name))]))
            } else {
                error(404, "no such token")
            }
        }
        ("POST", ["dev"]) => {
            g.dev_mode = body.get("on").and_then(Value::as_bool).unwrap_or(false);
            let state = if g.dev_mode { "developer mode on" } else { "developer mode off" };
            g.push_event("dev", state);
            ok(obj(&[("dev_mode", g.dev_mode.to_string())]))
        }
        ("POST", ["mode"]) => match body.get("mode").and_then(Value::as_str) {
            Some("listen") => {
                g.settings.mode = Mode::Listen;
                g.actions.extend([Action::SaveSettings, Action::Restart]);
                ok(obj(&[("mode", js("listen"))]))
            }
            Some("master") if g.info.master_capable => {
                g.settings.mode = Mode::Master;
                g.actions.extend([Action::SaveSettings, Action::Restart]);
                ok(obj(&[("mode", js("master"))]))
            }
            Some("master") => error(409, "this firmware is built without master mode"),
            _ => error(400, "mode must be listen or master"),
        },
        ("POST", ["rediscover"]) => {
            g.actions.push(Action::Rediscover);
            ok(obj(&[("rediscover", "true".into())]))
        }
        ("POST", ["update", "install"]) => {
            let st = g.info.update_state.as_str();
            if st.starts_with("downloading") || st == "installing" || st == "restarting" {
                return error(409, "update already in progress");
            }
            let (Some(version), Some(url)) = (g.info.latest_version.clone(), g.info.latest_url.clone()) else {
                return error(409, "no update available");
            };
            if !url.starts_with("https://") {
                return error(400, "update URL must be https");
            }
            g.info.update_state = "downloading 0%".into();
            g.actions.push(Action::InstallUpdate { version: version.clone(), url });
            status(202, obj(&[("installing", js(&version))]))
        }
        ("POST", ["restart"]) => {
            g.actions.push(Action::Restart);
            ok(obj(&[("restart", "true".into())]))
        }
        (_, ["raw", ..]) | (_, ["registry"]) if !g.dev_mode => error(403, "developer mode is off"),
        ("GET", ["registry"]) => ok(registry_json(g)),
        ("GET", ["raw", addr, reg]) => {
            let (Ok(addr), Some(reg)) = (addr.parse::<u8>(), parse_reg(reg)) else { return error(400, "bad address or register") };
            let count = query(&r.query, "count").unwrap_or(1).clamp(1, 125) as u16;
            let vals: Vec<String> =
                (0..count).map(|i| g.registry.get(addr, reg.wrapping_add(i)).map(|v| v.to_string()).unwrap_or_else(|| "null".into())).collect();
            ok(obj(&[("addr", addr.to_string()), ("reg", reg.to_string()), ("values", format!("[{}]", vals.join(",")))]))
        }
        ("POST", ["raw", "write"]) => {
            if !g.master_active() {
                return error(409, "listen mode: the gateway does not write on the bus");
            }
            raw_write(g, &body)
        }
        _ => error(404, "not found"),
    }
}

fn parse_reg(s: &str) -> Option<u16> {
    match s.strip_prefix("0x") {
        Some(h) => u16::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}
