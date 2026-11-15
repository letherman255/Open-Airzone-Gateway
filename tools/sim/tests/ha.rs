use airzone_core::ha::*;
use airzone_core::listen::Listener;
use airzone_core::registry::Registry;
use airzone_core::topology::{build_central, Central, Topology, Zone};

const STARTUP: &str = include_str!("../fixtures/capture_startup.log");

fn home() -> (Topology, Registry) {
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
    let r = l.registry().clone();
    (Topology { centrals: vec![build_central(1, &r).unwrap()] }, r)
}

fn cfg(commands: bool) -> HaConfig {
    HaConfig {
        prefix: "airzone".into(),
        discovery: "homeassistant".into(),
        gateway_id: "az240ac4010203".into(),
        gateway_name: "Airzone gateway".into(),
        fw: "0.1.0".into(),
        commands,
    }
}

fn find<'a>(m: &'a [Message], needle: &str) -> Vec<&'a Message> {
    m.iter().filter(|x| x.topic.contains(needle)).collect()
}

#[test]
fn ha_discovery_streams_one_message_at_a_time() {
    // the whole set is ~61 KB: the ESP must publish it one message at a time (heap, 2026-11-06)
    let (t, r) = home();
    let all = discovery(&cfg(true), &t, &r);
    let mut streamed = Vec::new();
    discovery_each(&cfg(true), &t, &r, &mut |m| streamed.push(m));
    assert_eq!(streamed, all);
    let topics = discovery_topics(&cfg(true), &t, &r);
    assert_eq!(topics, all.iter().map(|m| m.topic.clone()).collect::<Vec<_>>());
}

#[test]
fn ha_number_without_bounds_gets_the_register_range() {
    // HA defaults a number to 0-100: the CO2 threshold (800 ppm, 8 bits x 50) showed "unknown"
    let (t, r) = home();
    let d = discovery(&cfg(true), &t, &r);
    let co2 = d.iter().find(|m| m.topic.contains("iaq_co2_gm")).expect("co2 threshold entity");
    assert!(co2.payload.contains(r#""min":0"#), "{}", co2.payload);
    assert!(co2.payload.contains(r#""max":12750"#), "{}", co2.payload);
}

#[test]
fn ha_zone_power_switch() {
    // a zone on/off switch next to the climate (dashboards and groups use one; the climate alone needs a mode)
    let (t, r) = home();
    let d = discovery(&cfg(true), &t, &r);
    let sw: Vec<_> = d.iter().filter(|m| m.topic.contains("/switch/") && m.topic.contains("zone_power")).collect();
    assert_eq!(sw.len(), 6);
    assert!(sw[0].payload.contains(r#"/set/power""#), "{}", sw[0].payload);
    // read-only gateway: a binary sensor instead
    let d = discovery(&cfg(false), &t, &r);
    assert!(!d.iter().any(|m| m.topic.contains("/switch/")));
    assert_eq!(d.iter().filter(|m| m.topic.contains("/binary_sensor/") && m.topic.contains("zone_power")).count(), 6);
    // the command is accepted
    let topic = sw[0].payload.split(r#""command_topic":""#).nth(1).unwrap().split('"').next().unwrap().to_string();
    assert!(command(&cfg(true), &t, &topic, b"OFF").is_some(), "{topic}");
}

#[test]
fn ha_zone_temperature_and_humidity_sensors_with_statistics() {
    // room curves need sensors (the climate attributes keep no long-term statistics)
    let (t, r) = home();
    let d = discovery(&cfg(true), &t, &r);
    let temp: Vec<_> = d.iter().filter(|m| m.topic.contains("/sensor/") && m.topic.contains("zone_temperature_z")).collect();
    let hum: Vec<_> = d.iter().filter(|m| m.topic.contains("/sensor/") && m.topic.contains("zone_humidity_z")).collect();
    assert_eq!((temp.len(), hum.len()), (6, 6));
    assert!(temp[0].payload.contains(r#""device_class":"temperature""#), "{}", temp[0].payload);
    assert!(temp[0].payload.contains(r#""state_class":"measurement""#), "{}", temp[0].payload);
    assert!(hum[0].payload.contains(r#""device_class":"humidity""#), "{}", hum[0].payload);
    assert!(hum[0].payload.contains(r#""state_class":"measurement""#), "{}", hum[0].payload);
    // the IAQ measurements too
    let co2 = d.iter().find(|m| m.topic.contains("/sensor/") && m.topic.ends_with("_iaq_co2/config")).expect("co2 sensor");
    assert!(co2.payload.contains(r#""device_class":"carbon_dioxide""#), "{}", co2.payload);
    assert!(co2.payload.contains(r#""state_class":"measurement""#), "{}", co2.payload);
}

#[test]
fn ha_removed_from_topics() {
    let gone = removed_topics(&["a".to_string(), "b".to_string()], &["b".to_string()]);
    assert_eq!(gone, vec![Message { topic: "a".into(), payload: String::new(), retain: true }]);
}

#[test]
fn ha_climate_per_zone() {
    let (t, r) = home();
    let d = discovery(&cfg(false), &t, &r);
    let climates: Vec<_> = d.iter().filter(|m| m.topic.starts_with("homeassistant/climate/")).collect();
    assert_eq!(climates.len(), 6);
    let salon = climates.iter().find(|m| m.topic.contains("_z2_")).unwrap();
    assert!(salon.retain);
    assert!(salon.payload.contains(r#""modes":["off","heat","cool","fan_only","dry"]"#), "{}", salon.payload);
    assert!(salon.payload.contains(r#""mode_state_topic":"airzone/az240ac4010203/1/zone/2/state""#));
    assert!(!salon.payload.contains("command_topic"));
    assert!(salon.payload.contains(r#""name":"Salon""#) || salon.payload.contains(r#""name":null"#));
}

#[test]
fn ha_commands_add_command_topics() {
    let (t, r) = home();
    let d = discovery(&cfg(true), &t, &r);
    let salon = d.iter().find(|m| m.topic == "homeassistant/climate/az240ac4010203_1_z2_climate/config").unwrap();
    assert!(salon.payload.contains(r#""temperature_command_topic":"airzone/az240ac4010203/1/zone/2/set/setpoint""#));
}

#[test]
fn ha_no_iaq_entities_without_sensor() {
    let (mut t, r) = home();
    assert!(!find(&discovery(&cfg(false), &t, &r), "_iaq_").is_empty());
    t.centrals[0].iaq = false;
    let d = discovery(&cfg(false), &t, &r);
    assert!(find(&d, "_iaq_").is_empty());
    assert!(find(&states(&cfg(false), &t, &r), "/iaq/").is_empty());
}

#[test]
fn ha_ionizer_needs_presence() {
    let (t, r) = home();
    assert!(find(&discovery(&cfg(false), &t, &r), "ionizer").is_empty());
}

#[test]
fn ha_tier3_never_published() {
    let (t, r) = home();
    let d = discovery(&cfg(true), &t, &r);
    for id in ["d1_input", "filter_reset", "easyzone_mode", "_master", "outlet_zone", "factory_reset", "filter_hours"] {
        assert!(find(&d, id).is_empty(), "{id}");
    }
}

#[test]
fn ha_state_unknown_is_null() {
    let (t, mut r) = home();
    r.update(1, 0x0208, &[0xFFFF], 1);
    let s = states(&cfg(false), &t, &r);
    let z2 = s.iter().find(|m| m.topic == "airzone/az240ac4010203/1/zone/2/state").unwrap();
    assert!(z2.payload.contains(r#""temperature":null"#), "{}", z2.payload);
    assert!(!z2.retain);
}

#[test]
fn ha_discovery_json_escapes_names() {
    let mut r = Registry::default();
    r.update(1, 0x0000, &[0; 0xD0], 0);
    let t = Topology {
        centrals: vec![Central { addr: 1, iaq: false, zones: vec![Zone { n: 1, name: "Séjour \"1\"".into(), master: true }] }],
    };
    let d = discovery(&cfg(false), &t, &r);
    let c = d.iter().find(|m| m.topic.contains("climate")).unwrap();
    assert!(c.payload.contains(r#"Séjour \"1\""#), "{}", c.payload);
}

#[test]
fn ha_zone_state_hvac_mode() {
    let (t, mut r) = home();
    let st = |r: &Registry| {
        states(&cfg(false), &t, r).into_iter().find(|m| m.topic.ends_with("/zone/2/state")).unwrap().payload
    };
    r.update(1, 0x0200, &[0x0504], 1);
    r.update(1, 0x0209, &[0x0080], 1);
    let p = st(&r);
    assert!(p.contains(r#""hvac_mode":"heat""#) && p.contains(r#""action":"heating""#), "{p}");
    r.update(1, 0x0200, &[0x0500], 2);
    let p = st(&r);
    assert!(p.contains(r#""hvac_mode":"off""#) && p.contains(r#""action":"off""#), "{p}");
}

#[test]
fn ha_listen_mode_has_no_writable_components() {
    let (t, r) = home();
    let d = discovery(&cfg(false), &t, &r);
    for comp in ["switch", "select", "number", "text"] {
        assert!(d.iter().all(|m| !m.topic.starts_with(&format!("homeassistant/{comp}/"))), "{comp}");
    }
    assert!(availability_topic(&cfg(false)).ends_with("/availability"));
}

#[test]
#[ignore]
fn print_home_messages() {
    let (t, r) = home();
    let d = discovery(&cfg(false), &t, &r);
    println!("discovery {}", d.len());
    for m in d.iter().take(3) { println!("{}\n{}", m.topic, m.payload); }
    for m in states(&cfg(false), &t, &r) { println!("{}\n{}", m.topic, m.payload); }
}

#[test]
fn ha_removed_configs_are_cleared() {
    let (t, mut r) = home();
    let before = discovery(&cfg(false), &t, &r);
    // outlet 7 linked to zone 3: a new outlet device appears, then disappears again
    r.update(1, 0x2F00, &[0x0703], 9);
    let linked = discovery(&cfg(false), &t, &r);
    assert_ne!(discovery_key(&before), discovery_key(&linked));
    let topics: Vec<String> = linked.iter().map(|m| m.topic.clone()).collect();
    let clear = removed(&topics, &before);
    assert_eq!(clear.len(), 1);
    assert!(clear[0].topic.contains("_o7"));
    assert!(clear.iter().all(|m| m.payload.is_empty() && m.retain));
    assert!(removed(&topics, &linked).is_empty());
}

#[test]
fn ha_command_setpoint() {
    let (t, _) = home();
    let c = command(&cfg(true), &t, "airzone/az240ac4010203/1/zone/2/set/setpoint", b"21.5").unwrap();
    match c {
        Command::Param { param, target, input } => {
            assert_eq!(param.id, "zone.setpoint");
            assert_eq!(target, airzone_core::model::Target::Zone { addr: 1, zone: 2 });
            assert_eq!(input, airzone_core::model::Input::Num(21.5));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn ha_command_mode_off_is_power() {
    let (t, _) = home();
    match command(&cfg(true), &t, "airzone/az240ac4010203/1/zone/3/set/hvac_mode", b"off").unwrap() {
        Command::Param { param, input, .. } => {
            assert_eq!(param.id, "zone.power");
            assert_eq!(input, airzone_core::model::Input::Bool(false));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        command(&cfg(true), &t, "airzone/az240ac4010203/1/zone/3/set/hvac_mode", b"cool"),
        Some(Command::Mode { addr: 1, zone: 3, mode: 9 })
    );
}

#[test]
fn ha_command_unknown_topic_none() {
    let (t, _) = home();
    for (topic, p) in [
        ("airzone/other/1/zone/2/set/setpoint", "21"),
        ("airzone/az240ac4010203/1/zone/9/set/setpoint", "21"),
        ("airzone/az240ac4010203/1/zone/2/set/nothing", "1"),
        ("airzone/az240ac4010203/1/zone/2/set/setpoint", "warm"),
    ] {
        assert!(command(&cfg(true), &t, topic, p.as_bytes()).is_none(), "{topic} {p}");
    }
    assert!(command(&cfg(false), &t, "airzone/az240ac4010203/1/zone/2/set/setpoint", b"21").is_none());
}

#[test]
fn ha_command_tier3_refused() {
    let (t, _) = home();
    assert!(command(&cfg(true), &t, "airzone/az240ac4010203/1/system/set/d1_input", b"bridge alarm").is_none());
    assert!(command(&cfg(true), &t, "airzone/az240ac4010203/1/system/set/filter_reset", b"ON").is_none());
}

#[test]
fn ha_gateway_entities() {
    let info = airzone_core::gateway::GatewayInfo { version: "0.3.0".into(), ..Default::default() };
    let d = gateway_discovery(&cfg(false), &info);
    assert!(d.iter().any(|m| m.topic == "homeassistant/sensor/az240ac4010203_gw_mode/config"));
    assert!(d.iter().any(|m| m.topic == "homeassistant/button/az240ac4010203_gw_restart/config"));
    assert!(d.iter().all(|m| m.payload.contains(r#""identifiers":["az240ac4010203"]"#)));
    let s = gateway_state(&cfg(false), &info, "listen", false);
    assert_eq!(s.topic, "airzone/az240ac4010203/gateway/state");
    assert!(s.payload.contains(r#""mode":"listen""#) && s.payload.contains(r#""version":"0.3.0""#));
    assert_eq!(gateway_action(&cfg(false), "airzone/az240ac4010203/gateway/set/restart"), Some("restart"));
    assert_eq!(gateway_action(&cfg(false), "airzone/az240ac4010203/gateway/set/format"), None);
}

#[test]
fn ha_gateway_health_entities() {
    // heap, restart cause, last crash, bus errors: diagnostic, disabled until the user enables them
    let info = airzone_core::gateway::GatewayInfo {
        version: "0.3.0".into(),
        free_heap: 81000,
        min_free_heap: 55000,
        reset_reason: "panic".into(),
        last_panic: r#"no Rust panic; last step: "mqtt states""#.into(),
        bus_read_errors: 3,
        write_failures: 1,
        ..Default::default()
    };
    let d = gateway_discovery(&cfg(false), &info);
    for k in ["free_heap", "min_free_heap", "reset_reason", "last_panic", "bus_read_errors", "write_failures"] {
        let m = d.iter().find(|m| m.topic == format!("homeassistant/sensor/az240ac4010203_gw_{k}/config")).unwrap_or_else(|| panic!("{k}"));
        assert!(m.payload.contains(r#""entity_category":"diagnostic""#), "{}", m.payload);
        assert!(m.payload.contains(r#""enabled_by_default":false"#), "{}", m.payload);
    }
    let s = gateway_state(&cfg(false), &info, "master", false);
    assert!(airzone_core::json::parse(&s.payload).is_ok(), "{}", s.payload);
    for frag in [r#""free_heap":81000"#, r#""min_free_heap":55000"#, r#""reset_reason":"panic""#, r#""bus_read_errors":3"#, r#""write_failures":1"#] {
        assert!(s.payload.contains(frag), "{frag} in {}", s.payload);
    }
}

#[test]
fn ha_payloads_are_valid_json() {
    let (t, r) = home();
    let info = airzone_core::gateway::GatewayInfo { version: "0.3.0".into(), ..Default::default() };
    let mut all = discovery(&cfg(true), &t, &r);
    all.extend(states(&cfg(true), &t, &r));
    all.extend(gateway_discovery(&cfg(true), &info));
    all.push(gateway_state(&cfg(true), &info, "master", true));
    for m in &all {
        assert!(airzone_core::json::parse(&m.payload).is_ok(), "{}: {}", m.topic, m.payload);
    }
    assert!(all.len() > 100);
}

#[test]
fn ha_update_entity() {
    let info = airzone_core::gateway::GatewayInfo { version: "0.3.0".into(), latest_version: Some("0.4.0".into()), ..Default::default() };
    let d = gateway_discovery(&cfg(false), &info);
    let u = d.iter().find(|m| m.topic == "homeassistant/update/az240ac4010203_gw_firmware/config").expect("update entity");
    assert!(u.payload.contains("latest_version_template") && !u.payload.contains("command_topic"));
    let s = gateway_state(&cfg(false), &info, "listen", false);
    assert!(s.payload.contains(r#""latest_version":"0.4.0""#), "{}", s.payload);
    let none = airzone_core::gateway::GatewayInfo { version: "0.3.0".into(), ..Default::default() };
    assert!(gateway_state(&cfg(false), &none, "listen", false).payload.contains(r#""latest_version":"0.3.0""#));
}
