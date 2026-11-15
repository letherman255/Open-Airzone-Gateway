//! Gateway settings as stored in NVS (string key/value pairs). Loading never fails: unknown keys are
//! ignored and bad values fall back to the default. Master mode is only ever set explicitly.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Listen,
    Master,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub wifi_ssid: String,
    pub wifi_pass: String,
    pub mqtt_url: String,
    pub mqtt_user: String,
    pub mqtt_pass: String,
    pub mqtt_prefix: String,
    pub hostname: String,
    pub gateway_name: String,
    pub mode: Mode,
    pub identity_name: [u8; 10],
    pub heartbeat: bool,
    pub identification: bool,
    /// Air renewal days, 0 = Monday … 6 = Sunday.
    pub renew_days: Vec<u8>,
    /// Free-cooling months, 0 = January … 11 = December.
    pub fcool_months: Vec<u8>,
    /// POSIX TZ string.
    pub timezone: String,
    /// Admin password hash (`pbkdf2$…`); empty = not set.
    pub admin_hash: String,
    /// API tokens as `name:sha256;…` (hashes only).
    pub api_tokens: String,
    /// URL of the release manifest checked once a day for updates (empty = off).
    pub update_manifest: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            wifi_ssid: String::new(),
            wifi_pass: String::new(),
            mqtt_url: String::new(),
            mqtt_user: String::new(),
            mqtt_pass: String::new(),
            mqtt_prefix: "airzone".into(),
            hostname: String::new(),
            gateway_name: "Airzone gateway".into(),
            mode: Mode::Listen,
            identity_name: *b"OPENAZWS01",
            heartbeat: true,
            identification: true,
            renew_days: (0..7).collect(),
            fcool_months: (0..12).collect(),
            timezone: "CET-1CEST,M3.5.0,M10.5.0/3".into(),
            admin_hash: String::new(),
            api_tokens: String::new(),
            update_manifest: String::new(),
        }
    }
}

const SECRET: [&str; 4] = ["wifi_pass", "mqtt_pass", "admin_hash", "api_tokens"];

fn list(v: &[u8]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
}

fn parse_list(s: &str, max: u8) -> Vec<u8> {
    let mut v: Vec<u8> = s.split(',').filter_map(|x| x.trim().parse::<u8>().ok()).filter(|x| *x <= max).collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

impl Settings {
    pub fn to_kv(&self) -> Vec<(&'static str, String)> {
        vec![
            ("wifi_ssid", self.wifi_ssid.clone()),
            ("wifi_pass", self.wifi_pass.clone()),
            ("mqtt_url", self.mqtt_url.clone()),
            ("mqtt_user", self.mqtt_user.clone()),
            ("mqtt_pass", self.mqtt_pass.clone()),
            ("mqtt_prefix", self.mqtt_prefix.clone()),
            ("hostname", self.hostname.clone()),
            ("gateway_name", self.gateway_name.clone()),
            ("mode", if self.mode == Mode::Master { "master" } else { "listen" }.into()),
            ("identity_name", String::from_utf8_lossy(&self.identity_name).into_owned()),
            ("heartbeat", (self.heartbeat as u8).to_string()),
            ("identification", (self.identification as u8).to_string()),
            ("renew_days", list(&self.renew_days)),
            ("fcool_months", list(&self.fcool_months)),
            ("timezone", self.timezone.clone()),
            ("admin_hash", self.admin_hash.clone()),
            ("api_tokens", self.api_tokens.clone()),
            ("update_manifest", self.update_manifest.clone()),
        ]
    }

    pub fn from_kv(kv: &[(String, String)]) -> Settings {
        let mut s = Settings::default();
        for (k, v) in kv {
            match k.as_str() {
                "wifi_ssid" if v.len() <= 32 => s.wifi_ssid = v.clone(),
                "wifi_pass" if v.len() <= 64 => s.wifi_pass = v.clone(),
                "mqtt_url" => s.mqtt_url = v.clone(),
                "mqtt_user" => s.mqtt_user = v.clone(),
                "mqtt_pass" => s.mqtt_pass = v.clone(),
                "mqtt_prefix" if !v.is_empty() && !v.contains(['#', '+', ' ']) => s.mqtt_prefix = v.clone(),
                "hostname" => s.hostname = v.clone(),
                "gateway_name" if !v.is_empty() => s.gateway_name = v.clone(),
                "mode" if v == "master" => s.mode = Mode::Master,
                "identity_name" if v.len() == 10 && v.bytes().all(|b| b.is_ascii_graphic()) => {
                    s.identity_name.copy_from_slice(v.as_bytes())
                }
                "heartbeat" => s.heartbeat = parse_bool(v).unwrap_or(s.heartbeat),
                "identification" => s.identification = parse_bool(v).unwrap_or(s.identification),
                "renew_days" => s.renew_days = parse_list(v, 6),
                "fcool_months" => s.fcool_months = parse_list(v, 11),
                "timezone" if !v.is_empty() => s.timezone = v.clone(),
                "admin_hash" => s.admin_hash = v.clone(),
                "api_tokens" => s.api_tokens = v.clone(),
                "update_manifest" if v.is_empty() || v.starts_with("https://") || v.starts_with("http://") => s.update_manifest = v.clone(),
                _ => {}
            }
        }
        s
    }

    /// The settings with every secret replaced by `***` (empty secrets stay empty).
    pub fn redacted(&self) -> Vec<(&'static str, String)> {
        self.to_kv()
            .into_iter()
            .map(|(k, v)| if SECRET.contains(&k) && !v.is_empty() { (k, "***".to_string()) } else { (k, v) })
            .collect()
    }
}
