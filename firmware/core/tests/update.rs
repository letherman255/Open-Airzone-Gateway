use airzone_core::update::*;

const MANIFEST: &str = r#"{"name":"open-airzone-gateway","version":"v0.4.0","builds":[{"chipFamily":"ESP32-S3","parts":[{"path":"airzone-gateway-waveshare-esp32s3-rs485-master.bin","offset":131072}]}]}"#;

#[test]
fn update_manifest_parse() {
    let u = parse_manifest(MANIFEST, "https://example.org/releases/latest/download/manifest.json").unwrap();
    assert_eq!(u.version, "0.4.0");
    assert_eq!(u.url, "https://example.org/releases/latest/download/airzone-gateway-waveshare-esp32s3-rs485-master.bin");
    assert!(parse_manifest("not json", "https://x/m.json").is_none());
    assert!(parse_manifest(r#"{"version":"v1"}"#, "https://x/m.json").is_none());
    assert!(parse_manifest(r#"{"version":"v1.0.0","builds":[]}"#, "https://x/m.json").is_none());
    let abs = MANIFEST.replace("\"path\":\"", "\"path\":\"https://cdn.example/");
    assert_eq!(parse_manifest(&abs, "https://x/m.json").unwrap().url, "https://cdn.example/airzone-gateway-waveshare-esp32s3-rs485-master.bin");
}

#[test]
fn update_version_compare() {
    assert!(newer("0.3.0", "v0.4.0"));
    assert!(newer("0.3.0", "0.3.1"));
    assert!(newer("0.9.9", "1.0.0"));
    assert!(!newer("0.4.0", "v0.4.0"));
    assert!(!newer("0.4.1", "0.4.0"));
    assert!(!newer("0.4.0", "garbage"));
    assert!(!newer("0.4.0", "0.4.0-rc1"));
}

#[test]
fn image_check() {
    assert_eq!(check_image(Some(MIN_IMAGE), 0xE9), Ok(()));
    assert!(check_image(None, 0xE9).is_err());
    assert!(check_image(Some(MAX_IMAGE + 1), 0xE9).is_err());
    assert!(check_image(Some(MIN_IMAGE - 1), 0xE9).is_err());
    assert_eq!(check_image(Some(MIN_IMAGE), b'<'), Err("not an ESP app image"));
}
