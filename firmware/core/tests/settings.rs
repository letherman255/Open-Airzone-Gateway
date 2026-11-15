use airzone_core::settings::*;

fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn settings_default_is_listen() {
    let s = Settings::default();
    assert_eq!(s.mode, Mode::Listen);
    assert_eq!(s.mqtt_prefix, "airzone");
    assert_eq!(&s.identity_name, b"OPENAZWS01");
    assert!(s.heartbeat && s.identification);
    assert_eq!(s.timezone, "CET-1CEST,M3.5.0,M10.5.0/3");
}

#[test]
fn settings_roundtrip() {
    let mut s = Settings::default();
    s.wifi_ssid = "HomeNet".into();
    s.mqtt_url = "mqtt://192.168.1.10:1883".into();
    s.mqtt_pass = "p=a;ss".into();
    s.renew_days = vec![0, 2, 4];
    s.fcool_months = vec![5, 6];
    s.heartbeat = false;
    s.identity_name = *b"MYGATEWAY1";
    let back = Settings::from_kv(&s.to_kv().into_iter().map(|(k, v)| (k.to_string(), v)).collect::<Vec<_>>());
    assert_eq!(back, s);
}

#[test]
fn settings_garbage_tolerated() {
    let s = Settings::from_kv(&kv(&[
        ("mode", "banana"),
        ("heartbeat", "maybe"),
        ("renew_days", "x,1,,99"),
        ("identity_name", "TOO-LONG-NAME-HERE"),
        ("unknown_key", "1"),
        ("mqtt_prefix", ""),
    ]));
    assert_eq!(s.mode, Mode::Listen);
    assert!(s.heartbeat);
    assert_eq!(s.renew_days, vec![1]);
    assert_eq!(&s.identity_name, b"OPENAZWS01");
    assert_eq!(s.mqtt_prefix, "airzone");
}

#[test]
fn settings_days_months_parse() {
    let s = Settings::from_kv(&kv(&[("renew_days", "0,1,2,9"), ("fcool_months", "11,12,3,3")]));
    assert_eq!(s.renew_days, vec![0, 1, 2]);
    assert_eq!(s.fcool_months, vec![3, 11]);
}

#[test]
fn settings_master_only_explicit() {
    assert_eq!(Settings::from_kv(&kv(&[("mode", "master")])).mode, Mode::Master);
}

#[test]
fn settings_redacted_hides_secrets() {
    let mut s = Settings::default();
    s.wifi_pass = "secret1".into();
    s.mqtt_pass = "secret2".into();
    s.admin_hash = "abc".into();
    let r = s.redacted();
    let text = format!("{r:?}");
    assert!(!text.contains("secret1") && !text.contains("secret2") && !text.contains("abc"));
    assert!(r.iter().any(|(k, v)| *k == "wifi_pass" && v == "***"));
    assert!(r.iter().any(|(k, v)| *k == "mqtt_user" && v.is_empty()));
}
