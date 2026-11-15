// Optional build-time defaults (git-ignored file, path in AZ_GW_ENV). Keys: WIFI_SSID, WIFI_PASS,
// OTA_TOKEN, MQTT_URL, MQTT_USER, MQTT_PASS. Values are never printed. Settings saved in NVS win.

use std::time::{SystemTime, UNIX_EPOCH};

const KEYS: [&str; 6] = ["WIFI_SSID", "WIFI_PASS", "OTA_TOKEN", "MQTT_URL", "MQTT_USER", "MQTT_PASS"];

fn main() {
    embuild::espidf::sysenv::output();
    println!("cargo:rerun-if-env-changed=AZ_GW_ENV");
    let mut vals: Vec<(String, String)> = KEYS.iter().map(|k| (k.to_string(), String::new())).collect();
    if let Ok(path) = std::env::var("AZ_GW_ENV") {
        println!("cargo:rerun-if-changed={path}");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("AZ_GW_ENV file {path} not readable"));
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.trim_start().starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                if let Some(e) = vals.iter_mut().find(|(n, _)| n == k.trim()) {
                    e.1 = v.to_string();
                }
            }
        }
    }
    for (k, v) in &vals {
        println!("cargo:rustc-env=DEF_{k}={v}");
    }
    board();
    // the build stamp tells images apart (OTA uploader, /api/v1/info): refresh it whenever the firmware or
    // the core changes, not only when the env file does
    for dir in ["src", "../core/src", "../../catalogue/registers.toml", "../../web"] {
        println!("cargo:rerun-if-changed={dir}");
    }
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    println!("cargo:rustc-env=BUILD_UNIX={t}");
}

/// Board pin map from AZ_BOARD (default Waveshare ESP32-S3-RS485-CAN).
fn board() {
    for k in ["AZ_BOARD", "AZ_PIN_RX", "AZ_PIN_TX", "AZ_PIN_DE", "AZ_PIN_BUTTON"] {
        println!("cargo:rerun-if-env-changed={k}");
    }
    let board = std::env::var("AZ_BOARD").unwrap_or_else(|_| "waveshare-esp32s3-rs485".into());
    let pin = |k: &str, d: Option<i32>| -> Option<i32> {
        std::env::var(k).ok().map(|v| v.parse().unwrap_or_else(|_| panic!("{k} must be a GPIO number"))).or(d)
    };
    let (rx, tx, de, button) = match board.as_str() {
        "waveshare-esp32s3-rs485" => (18, 17, 21, Some(0)),
        "generic-esp32-max485" => (
            pin("AZ_PIN_RX", None).expect("AZ_PIN_RX required for generic-esp32-max485"),
            pin("AZ_PIN_TX", None).expect("AZ_PIN_TX required"),
            pin("AZ_PIN_DE", None).expect("AZ_PIN_DE required"),
            pin("AZ_PIN_BUTTON", None),
        ),
        other => panic!("unknown AZ_BOARD {other}"),
    };
    let button = button.map(|b| format!("Some({b})")).unwrap_or_else(|| "None".into());
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("board_gen.rs");
    std::fs::write(
        out,
        format!("pub const BOARD: Board = Board {{ name: {board:?}, uart_rx: {rx}, uart_tx: {tx}, de: {de}, button: {button} }};
"),
    )
    .unwrap();
}
