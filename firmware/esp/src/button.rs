//! BOOT button held 5 s: forget the network settings (Wi-Fi, MQTT) and restart into the setup portal.
//! Installation settings live in the central and are not touched.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use esp_idf_svc::hal::gpio::{AnyIOPin, Input, PinDriver, Pull};

use crate::nvs_store::Store;

pub fn watch(pin: i32, store: Arc<Mutex<Store>>) -> anyhow::Result<()> {
    let b: PinDriver<'static, Input> = PinDriver::input(unsafe { AnyIOPin::steal(pin as u8) }, Pull::Up)?;
    std::thread::Builder::new().name("button".into()).stack_size(4 * 1024).spawn(move || {
        let mut held = 0u32;
        loop {
            std::thread::sleep(Duration::from_millis(100));
            held = if b.is_low() { held + 1 } else { 0 };
            if held == 50 {
                log::warn!("button held 5 s: network settings reset");
                if let Err(e) = store.lock().unwrap().reset_network() {
                    log::error!("reset failed: {e}");
                }
                crate::restart_later();
            }
        }
    })?;
    Ok(())
}
