//! Home Assistant MQTT discovery and state messages, built from the catalogue, the topology and the
//! registry. Tier 3 (installer) settings are never published. Without `commands`, only read-only
//! components are announced (listen mode).

use crate::catalogue::{Param, Requires, Scope, PARAMS};
use crate::model::{decode, Value};
use crate::registry::Registry;
use crate::topology::{Central, Topology, Zone};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HaConfig {
    pub prefix: String,
    pub discovery: String,
    pub gateway_id: String,
    pub gateway_name: String,
    pub fw: String,
    pub commands: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub topic: String,
    pub payload: String,
    pub retain: bool,
}

pub fn availability_topic(c: &HaConfig) -> String {
    format!("{}/{}/availability", c.prefix, c.gateway_id)
}

/// JSON string literal.
pub fn js(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn num(x: f32) -> String {
    if !x.is_finite() {
        return "null".into();
    }
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// JSON value of a decoded register value (`null` when unknown).
pub fn json_value(v: &Value) -> String {
    match v {
        Value::Num(x) => num(*x),
        Value::Enum(_, name) => js(name),
        Value::Bool(b) => b.to_string(),
        Value::Text(t) => js(t),
        Value::Unknown(_) => "null".into(),
    }
}

/// Key of a parameter inside its device state object: the id without the scope prefix.
pub fn key(p: &Param) -> &'static str {
    p.id.split_once('.').map(|(_, k)| k).unwrap_or(p.id)
}

#[derive(Clone, Copy)]
enum Dev<'a> {
    System,
    Iaq,
    Zone(&'a Zone),
    Outlet(u8),
}

fn base(c: &HaConfig, addr: u8, d: Dev) -> String {
    match d {
        Dev::System => format!("{}/{}/{addr}/system", c.prefix, c.gateway_id),
        Dev::Iaq => format!("{}/{}/{addr}/iaq", c.prefix, c.gateway_id),
        Dev::Zone(z) => format!("{}/{}/{addr}/zone/{}", c.prefix, c.gateway_id, z.n),
        Dev::Outlet(n) => format!("{}/{}/{addr}/outlet/{n}", c.prefix, c.gateway_id),
    }
}

fn device_json(c: &HaConfig, addr: u8, d: Dev) -> String {
    let gw = &c.gateway_id;
    let central = format!("{gw}_c{addr}");
    let (id, name, model, via) = match d {
        Dev::System => (central.clone(), format!("Airzone central {addr}"), "Central", gw.clone()),
        Dev::Iaq => (format!("{central}_iaq"), format!("Air quality sensor {addr}"), "IAQ sensor", central.clone()),
        Dev::Zone(z) => (format!("{central}_z{}", z.n), z.name.clone(), "Zone", central.clone()),
        Dev::Outlet(n) => (format!("{central}_o{n}"), format!("Air outlet {n}"), "Air outlet", central.clone()),
    };
    format!(
        r#"{{"identifiers":[{}],"name":{},"manufacturer":"Airzone","model":{},"via_device":{}}}"#,
        js(&id), js(&name), js(model), js(&via)
    )
}

fn suffix(d: Dev) -> String {
    match d {
        Dev::Zone(z) => format!("_z{}", z.n),
        Dev::Outlet(n) => format!("_o{n}"),
        _ => String::new(),
    }
}

fn abs(p: &Param, d: Dev) -> u16 {
    match d {
        Dev::System | Dev::Iaq => p.reg,
        Dev::Zone(z) => ((z.n as u16) << 8) + p.reg,
        Dev::Outlet(n) => 0x2900 + 0x100 * (n as u16 - 1) + p.reg,
    }
}

fn value_of(p: &Param, d: Dev, addr: u8, reg: &Registry) -> Value {
    match reg.get_range(addr, abs(p, d), p.length.max(1) as u16) {
        Some(raw) => decode(p, &raw),
        None => Value::Unknown(0xFFFF),
    }
}

fn param_by(id: &str) -> Option<&'static Param> {
    crate::catalogue::param(id)
}

/// Whether the equipment `p` needs is present (IAQ sensor, VMC on, ionizer).
pub fn requires_ok(p: &Param, c: &Central, reg: &Registry) -> bool {
    match p.requires {
        None => true,
        Some(Requires::IaqPresent) => c.iaq,
        Some(Requires::VmcOn) => {
            c.iaq && param_by("iaq.vmc").is_some_and(|v| value_of(v, Dev::Iaq, c.addr, reg) == Value::Bool(true))
        }
        // no presence rule yet (spec open item): never published
        Some(Requires::Ionizer) => false,
    }
}

fn published(p: &Param) -> bool {
    p.tier <= 2 && !p.ha_component.is_empty() && p.ha_component != "climate"
}

fn devices<'a>(c: &'a Central, reg: &Registry) -> Vec<(Scope, Dev<'a>)> {
    let mut v = vec![(Scope::System, Dev::System)];
    if c.iaq {
        v.push((Scope::Iaq, Dev::Iaq));
    }
    v.extend(c.zones.iter().map(|z| (Scope::Zone, Dev::Zone(z))));
    for n in 1..=8u8 {
        let w0 = reg.get(c.addr, 0x2900 + 0x100 * (n as u16 - 1));
        if w0.is_some_and(|w| w != 0xFFFF && w & 0xFF != 0) {
            v.push((Scope::Outlet, Dev::Outlet(n)));
        }
    }
    v
}

const HA_MODES: [(u16, &str); 4] = [(5, "heat"), (9, "cool"), (3, "fan_only"), (6, "dry")];

fn hvac_mode(z: &Zone, addr: u8, reg: &Registry) -> Option<&'static str> {
    let w = reg.get(addr, (z.n as u16) << 8)?;
    if w == 0xFFFF {
        return None;
    }
    let mode = (w >> 8) & 0xF;
    if w & 0x0004 == 0 || mode == 0 {
        return Some("off");
    }
    HA_MODES.iter().find(|(k, _)| *k == mode).map(|(_, n)| *n)
}

fn action(z: &Zone, addr: u8, reg: &Registry) -> Option<&'static str> {
    let m = hvac_mode(z, addr, reg)?;
    if m == "off" {
        return Some("off");
    }
    let demand = reg.get(addr, ((z.n as u16) << 8) + 9).is_some_and(|w| w != 0xFFFF && w & 0x0080 != 0);
    Some(match (m, demand) {
        (_, false) => "idle",
        ("heat", true) => "heating",
        ("cool" | "dry", true) => "cooling",
        _ => "fan",
    })
}

fn entity(c: &HaConfig, central: &Central, p: &Param, d: Dev) -> Message {
    entity_as(c, central, p, d, p.ha_component)
}

/// `p` published as `component` (e.g. the zone power, part of the climate, also as a switch).
fn entity_as(c: &HaConfig, central: &Central, p: &Param, d: Dev, component: &'static str) -> Message {
    let addr = central.addr;
    let uid = format!("{}_{}_{}{}", c.gateway_id, addr, p.id.replace('.', "_"), suffix(d));
    let k = key(p);
    let comp = match (c.commands, component) {
        (false, "switch") => "binary_sensor",
        (false, "select" | "number" | "text") => "sensor",
        (_, other) => other,
    };
    let state = format!("{}/state", base(c, addr, d));
    let mut f = vec![
        format!(r#""name":{}"#, js(p.label)),
        format!(r#""unique_id":{}"#, js(&uid)),
        format!(r#""state_topic":{}"#, js(&state)),
        format!(r#""availability_topic":{}"#, js(&availability_topic(c))),
        format!(r#""device":{}"#, device_json(c, addr, d)),
    ];
    let bool_tpl = format!("{{{{ 'ON' if value_json.{k} else 'OFF' }}}}");
    match comp {
        "binary_sensor" | "switch" => f.push(format!(r#""value_template":{}"#, js(&bool_tpl))),
        _ => f.push(format!(r#""value_template":{}"#, js(&format!("{{{{ value_json.{k} }}}}")))),
    }
    if !p.unit.is_empty() && matches!(comp, "sensor" | "number") && p.values.is_empty() {
        f.push(format!(r#""unit_of_measurement":{}"#, js(p.unit)));
        let class = match p.ha_role {
            "temperature" | "current_temperature" => Some("temperature"),
            "humidity" | "current_humidity" => Some("humidity"),
            "co2" => Some("carbon_dioxide"),
            "pm2_5" => Some("pm25"),
            "pm10" => Some("pm10"),
            "tvoc" => Some("volatile_organic_compounds_parts"),
            "pressure" => Some("atmospheric_pressure"),
            _ if p.unit == "°C" => Some("temperature"),
            _ => None,
        };
        if let Some(c) = class {
            f.push(format!(r#""device_class":"{c}""#));
        }
        // measurements keep long-term statistics (room curves)
        if comp == "sensor" && p.kind == crate::catalogue::Kind::Number {
            f.push(r#""state_class":"measurement""#.into());
        }
    }
    if c.commands && matches!(comp, "switch" | "select" | "number" | "text") {
        f.push(format!(r#""command_topic":{}"#, js(&format!("{}/set/{k}", base(c, addr, d)))));
        match comp {
            "switch" => f.push(r#""payload_on":"ON","payload_off":"OFF""#.into()),
            "select" => {
                let opts: Vec<String> = p.values.iter().map(|(_, n)| js(n)).collect();
                f.push(format!(r#""options":[{}]"#, opts.join(",")));
            }
            "number" => {
                // without declared bounds, the range the register field can hold (HA would default to 0-100)
                let raw_max = ((1u32 << (p.hi - p.lo + 1)) - 1) as f32;
                f.push(format!(r#""min":{}"#, num(p.min.unwrap_or(p.offset))));
                f.push(format!(r#""max":{}"#, num(p.max.unwrap_or(p.offset + p.scale * raw_max))));
                f.push(format!(r#""step":{}"#, num(p.scale)));
            }
            "text" => f.push(format!(r#""max":{}"#, 2 * p.length as u16)),
            _ => {}
        }
    }
    Message {
        topic: format!("{}/{comp}/{uid}/config", c.discovery),
        payload: format!("{{{}}}", f.join(",")),
        retain: true,
    }
}

fn climate(c: &HaConfig, central: &Central, z: &Zone, reg: &Registry) -> Message {
    let addr = central.addr;
    let uid = format!("{}_{}_z{}_climate", c.gateway_id, addr, z.n);
    let b = base(c, addr, Dev::Zone(z));
    let st = js(&format!("{b}/state"));
    let mut f = vec![
        r#""name":null"#.to_string(),
        format!(r#""unique_id":{}"#, js(&uid)),
        format!(r#""availability_topic":{}"#, js(&availability_topic(c))),
        format!(r#""device":{}"#, device_json(c, addr, Dev::Zone(z))),
        r#""modes":["off","heat","cool","fan_only","dry"]"#.into(),
        format!(r#""mode_state_topic":{st}"#),
        r#""mode_state_template":"{{ value_json.hvac_mode }}""#.into(),
        format!(r#""action_topic":{st}"#),
        r#""action_template":"{{ value_json.action }}""#.into(),
        format!(r#""current_temperature_topic":{st}"#),
        r#""current_temperature_template":"{{ value_json.temperature }}""#.into(),
        format!(r#""temperature_state_topic":{st}"#),
        r#""temperature_state_template":"{{ value_json.setpoint }}""#.into(),
        format!(r#""current_humidity_topic":{st}"#),
        r#""current_humidity_template":"{{ value_json.humidity }}""#.into(),
        r#""temperature_unit":"C","temp_step":0.5,"precision":0.1"#.into(),
    ];
    for (id, field) in [("zone.setpoint_min", "min_temp"), ("zone.setpoint_max", "max_temp")] {
        if let Some(p) = param_by(id) {
            if let Value::Num(x) = value_of(p, Dev::Zone(z), addr, reg) {
                f.push(format!(r#""{field}":{}"#, num(x)));
            }
        }
    }
    if c.commands {
        f.push(format!(r#""mode_command_topic":{}"#, js(&format!("{b}/set/hvac_mode"))));
        f.push(format!(r#""temperature_command_topic":{}"#, js(&format!("{b}/set/setpoint"))));
    }
    Message { topic: format!("{}/climate/{uid}/config", c.discovery), payload: format!("{{{}}}", f.join(",")), retain: true }
}

/// Retained discovery messages for every entity of the installation, handed one at a time to `f` (the
/// whole set is ~61 KB for a 6-zone home: too much heap for an ESP32 to hold at once).
pub fn discovery_each(c: &HaConfig, t: &Topology, reg: &Registry, f: &mut dyn FnMut(Message)) {
    for central in &t.centrals {
        for (scope, d) in devices(central, reg) {
            if let Dev::Zone(z) = d {
                f(climate(c, central, z, reg));
                // zone on/off as its own switch (groups, dashboards): the climate needs a mode to switch on
                if let Some(p) = param_by("zone.power") {
                    f(entity_as(c, central, p, d, "switch"));
                }
                // room temperature and humidity as sensors too: climate attributes keep no statistics
                for id in ["zone.temperature", "zone.humidity"] {
                    if let Some(p) = param_by(id) {
                        f(entity_as(c, central, p, d, "sensor"));
                    }
                }
            }
            for p in PARAMS.iter().filter(|p| p.scope == scope && published(p) && requires_ok(p, central, reg)) {
                f(entity(c, central, p, d));
            }
        }
    }
}

/// Retained discovery messages for every entity of the installation (host tools and tests).
pub fn discovery(c: &HaConfig, t: &Topology, reg: &Registry) -> Vec<Message> {
    let mut out = Vec::new();
    discovery_each(c, t, reg, &mut |m| out.push(m));
    out
}

/// Topics of the discovery set, in publishing order (to detect entities appearing or disappearing
/// without keeping the payloads).
pub fn discovery_topics(c: &HaConfig, t: &Topology, reg: &Registry) -> Vec<String> {
    let mut out = Vec::new();
    discovery_each(c, t, reg, &mut |m| out.push(m.topic));
    out
}

/// Empty retained payloads deleting every topic of `previous` that is not in `now`.
pub fn removed_topics(previous: &[String], now: &[String]) -> Vec<Message> {
    previous
        .iter()
        .filter(|t| !now.contains(t))
        .map(|t| Message { topic: t.clone(), payload: String::new(), retain: true })
        .collect()
}

/// One state JSON object per device (not retained).
pub fn states(c: &HaConfig, t: &Topology, reg: &Registry) -> Vec<Message> {
    let mut out = Vec::new();
    for central in &t.centrals {
        for (scope, d) in devices(central, reg) {
            let mut f: Vec<String> = PARAMS
                .iter()
                .filter(|p| p.scope == scope && p.tier <= 2 && !p.ha_component.is_empty() && requires_ok(p, central, reg))
                .map(|p| format!("{}:{}", js(key(p)), json_value(&value_of(p, d, central.addr, reg))))
                .collect();
            if let Dev::Zone(z) = d {
                let opt = |x: Option<&str>| x.map(js).unwrap_or_else(|| "null".into());
                f.push(format!(r#""hvac_mode":{}"#, opt(hvac_mode(z, central.addr, reg))));
                f.push(format!(r#""action":{}"#, opt(action(z, central.addr, reg))));
            }
            out.push(Message { topic: format!("{}/state", base(c, central.addr, d)), payload: format!("{{{}}}", f.join(",")), retain: false });
        }
    }
    out
}

/// Identity of a discovery set (its topics), to detect entities appearing or disappearing.
pub fn discovery_key(msgs: &[Message]) -> String {
    let mut t: Vec<&str> = msgs.iter().map(|m| m.topic.as_str()).collect();
    t.sort_unstable();
    t.join("
")
}

/// Empty retained payloads that delete, in Home Assistant, every config of `previous_topics` that is not
/// in `now`.
pub fn removed(previous_topics: &[String], now: &[Message]) -> Vec<Message> {
    previous_topics
        .iter()
        .filter(|t| !now.iter().any(|m| &m.topic == *t))
        .map(|t| Message { topic: t.clone(), payload: String::new(), retain: true })
        .collect()
}

/// A command received on a `…/set/<key>` topic, already checked against the catalogue.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Param { param: &'static Param, target: crate::model::Target, input: crate::model::Input },
    /// Climate mode other than off: global machine mode through the master zone, and switch this zone on.
    Mode { addr: u8, zone: u8, mode: u16 },
}

/// Parse a command topic + payload. Only tier 1-2 writable entities with an `ha` block are accepted,
/// and only when `commands` is on.
pub fn command(c: &HaConfig, t: &Topology, topic: &str, payload: &[u8]) -> Option<Command> {
    use crate::catalogue::{Confidence, Kind};
    use crate::model::{Input, Target};
    if !c.commands {
        return None;
    }
    let rest = topic.strip_prefix(&format!("{}/{}/", c.prefix, c.gateway_id))?;
    let parts: Vec<&str> = rest.split('/').collect();
    let text = std::str::from_utf8(payload).ok()?.trim();
    let (addr, scope, number, key) = match parts.as_slice() {
        [a, "system", "set", k] => (a.parse::<u8>().ok()?, Scope::System, None, *k),
        [a, "iaq", "set", k] => (a.parse::<u8>().ok()?, Scope::Iaq, None, *k),
        [a, "zone", n, "set", k] => (a.parse::<u8>().ok()?, Scope::Zone, Some(n.parse::<u8>().ok()?), *k),
        _ => return None,
    };
    let central = t.centrals.iter().find(|x| x.addr == addr)?;
    let target = match (scope, number) {
        (Scope::Zone, Some(n)) => {
            central.zones.iter().find(|z| z.n == n)?;
            Target::Zone { addr, zone: n }
        }
        (Scope::Iaq, _) if !central.iaq => return None,
        _ => Target::System { addr },
    };
    if key == "hvac_mode" {
        let Target::Zone { zone, .. } = target else { return None };
        if text == "off" {
            return Some(Command::Param { param: param_by("zone.power")?, target, input: Input::Bool(false) });
        }
        let mode = HA_MODES.iter().find(|(_, n)| *n == text)?.0;
        return Some(Command::Mode { addr, zone, mode });
    }
    let p = PARAMS.iter().find(|p| p.scope == scope && super::ha::key(p) == key)?;
    if !(p.writable && p.confidence == Confidence::Confirmed && p.tier <= 2 && !p.ha_component.is_empty()) {
        return None;
    }
    let input = match p.kind {
        Kind::Number => Input::Num(text.parse::<f32>().ok().filter(|x| x.is_finite())?),
        Kind::Bool => match text {
            "ON" | "on" | "true" | "1" => Input::Bool(true),
            "OFF" | "off" | "false" | "0" => Input::Bool(false),
            _ => return None,
        },
        Kind::Enum => Input::Choice(p.values.iter().find(|(_, n)| *n == text)?.0),
        Kind::Text => Input::Text(text.to_string()),
        Kind::Command => return None,
    };
    Some(Command::Param { param: p, target, input })
}

fn gateway_device(c: &HaConfig, info: &crate::gateway::GatewayInfo) -> String {
    format!(
        r#"{{"identifiers":[{}],"name":{},"manufacturer":"open-airzone-gateway","model":{},"sw_version":{}}}"#,
        js(&c.gateway_id), js(&c.gateway_name), js(if info.board.is_empty() { "gateway" } else { &info.board }), js(&info.version)
    )
}

/// Gateway entities (state sensors, restart / rediscover buttons). Buttons act on the gateway only.
pub fn gateway_discovery(c: &HaConfig, info: &crate::gateway::GatewayInfo) -> Vec<Message> {
    let st = js(&format!("{}/{}/gateway/state", c.prefix, c.gateway_id));
    let dev = gateway_device(c, info);
    let av = js(&availability_topic(c));
    let mut out = Vec::new();
    for (k, name, extra) in [
        ("mode", "Bus mode", ""),
        ("version", "Firmware", r#","entity_category":"diagnostic""#),
        ("rssi", "Wi-Fi signal", r#","unit_of_measurement":"dBm","device_class":"signal_strength","entity_category":"diagnostic""#),
        // health: disabled by default, enabled from Home Assistant when needed
        ("free_heap", "Free memory", r#","unit_of_measurement":"B","device_class":"data_size","state_class":"measurement","entity_category":"diagnostic","enabled_by_default":false"#),
        ("min_free_heap", "Lowest free memory", r#","unit_of_measurement":"B","device_class":"data_size","state_class":"measurement","entity_category":"diagnostic","enabled_by_default":false"#),
        ("reset_reason", "Last restart cause", r#","entity_category":"diagnostic","enabled_by_default":false"#),
        ("last_panic", "Last crash", r#","entity_category":"diagnostic","enabled_by_default":false"#),
        ("bus_read_errors", "Bus read errors", r#","state_class":"total_increasing","entity_category":"diagnostic","enabled_by_default":false"#),
        ("write_failures", "Failed writes", r#","state_class":"total_increasing","entity_category":"diagnostic","enabled_by_default":false"#),
        ("uptime", "Uptime", r#","unit_of_measurement":"s","device_class":"duration","entity_category":"diagnostic""#),
    ] {
        let uid = format!("{}_gw_{k}", c.gateway_id);
        out.push(Message {
            topic: format!("{}/sensor/{uid}/config", c.discovery),
            payload: format!(
                r#"{{"name":{},"unique_id":{},"state_topic":{st},"value_template":"{{{{ value_json.{k} }}}}","availability_topic":{av},"device":{dev}{extra}}}"#,
                js(name), js(&uid)
            ),
            retain: true,
        });
    }
    let uid = format!("{}_gw_dev_mode", c.gateway_id);
    out.push(Message {
        topic: format!("{}/binary_sensor/{uid}/config", c.discovery),
        payload: format!(
            r#"{{"name":"Developer mode","unique_id":{},"state_topic":{st},"value_template":"{{{{ 'ON' if value_json.dev_mode else 'OFF' }}}}","availability_topic":{av},"device":{dev},"entity_category":"diagnostic"}}"#,
            js(&uid)
        ),
        retain: true,
    });
    // firmware update entity: shows the newer release, installing stays a manual action (page / API)
    let uid = format!("{}_gw_firmware", c.gateway_id);
    out.push(Message {
        topic: format!("{}/update/{uid}/config", c.discovery),
        payload: format!(
            r#"{{"name":"Firmware","unique_id":{},"state_topic":{st},"value_template":"{{{{ value_json.version }}}}","latest_version_topic":{st},"latest_version_template":"{{{{ value_json.latest_version }}}}","availability_topic":{av},"device":{dev},"entity_category":"diagnostic"}}"#,
            js(&uid)
        ),
        retain: true,
    });
    for (k, name) in [("restart", "Restart"), ("rediscover", "Rediscover installation")] {
        let uid = format!("{}_gw_{k}", c.gateway_id);
        out.push(Message {
            topic: format!("{}/button/{uid}/config", c.discovery),
            payload: format!(
                r#"{{"name":{},"unique_id":{},"command_topic":{},"availability_topic":{av},"device":{dev},"entity_category":"config"}}"#,
                js(name), js(&uid), js(&format!("{}/{}/gateway/set/{k}", c.prefix, c.gateway_id))
            ),
            retain: true,
        });
    }
    out
}

pub fn gateway_state(c: &HaConfig, info: &crate::gateway::GatewayInfo, mode: &str, dev_mode: bool) -> Message {
    Message {
        topic: format!("{}/{}/gateway/state", c.prefix, c.gateway_id),
        payload: format!(
            r#"{{"mode":{},"version":{},"latest_version":{},"rssi":{},"uptime":{},"dev_mode":{dev_mode},"free_heap":{},"min_free_heap":{},"reset_reason":{},"last_panic":{},"bus_read_errors":{},"write_failures":{}}}"#,
            js(mode),
            js(&info.version),
            js(info.latest_version.as_deref().unwrap_or(&info.version)),
            info.rssi.map(|r| r.to_string()).unwrap_or_else(|| "null".into()),
            info.uptime_s,
            info.free_heap,
            info.min_free_heap,
            js(&info.reset_reason),
            // HA keeps 255 characters of a state
            js(&info.last_panic.chars().take(250).collect::<String>()),
            info.bus_read_errors,
            info.write_failures
        ),
        retain: false,
    }
}

/// `restart` or `rediscover` for a gateway button topic.
pub fn gateway_action(c: &HaConfig, topic: &str) -> Option<&'static str> {
    match topic.strip_prefix(&format!("{}/{}/gateway/set/", c.prefix, c.gateway_id))? {
        "restart" => Some("restart"),
        "rediscover" => Some("rediscover"),
        _ => None,
    }
}
