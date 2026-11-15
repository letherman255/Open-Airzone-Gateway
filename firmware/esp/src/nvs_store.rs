//! Settings in NVS (namespace "azgw", one string per key). Saved values override build-time defaults.

use airzone_core::settings::Settings;
use anyhow::Result;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};

const NAMESPACE: &str = "azgw";

pub struct Store {
    nvs: EspNvs<NvsDefault>,
}

impl Store {
    pub fn open(part: EspDefaultNvsPartition) -> Result<Self> {
        Ok(Store { nvs: EspNvs::new(part, NAMESPACE, true)? })
    }

    /// Defaults overlaid with every key present in NVS.
    pub fn load(&self, defaults: &Settings) -> Settings {
        let mut kv: Vec<(String, String)> = defaults.to_kv().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        let mut buf = vec![0u8; 1024];
        for (k, v) in kv.iter_mut() {
            if let Ok(Some(s)) = self.nvs.get_str(k, &mut buf) {
                *v = s.to_string();
            }
        }
        Settings::from_kv(&kv)
    }

    pub fn save(&self, s: &Settings) -> Result<()> {
        for (k, v) in s.to_kv() {
            self.nvs.set_str(k, &v)?;
        }
        Ok(())
    }

    /// Forget network settings (Wi-Fi, MQTT) only; installation settings live in the central.
    pub fn reset_network(&self) -> Result<()> {
        for k in ["wifi_ssid", "wifi_pass", "mqtt_url", "mqtt_user", "mqtt_pass"] {
            self.nvs.remove(k)?;
        }
        Ok(())
    }
}
