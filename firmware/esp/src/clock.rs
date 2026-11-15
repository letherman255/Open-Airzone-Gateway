//! Network time (SNTP) and local calendar, needed for the VMC renewal days / free-cooling months.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use esp_idf_svc::sntp::{EspSntp, SyncStatus};

pub struct Clock {
    sntp: EspSntp<'static>,
}

extern "C" {
    fn tzset();
}

impl Clock {
    pub fn start(tz: &str) -> Result<Self> {
        std::env::set_var("TZ", tz);
        unsafe { tzset() };
        Ok(Clock { sntp: EspSntp::new_default()? })
    }

    /// The system clock holds a real date (SNTP done at least once).
    pub fn time_valid() -> bool {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() > 1_700_000_000).unwrap_or(false)
    }

    pub fn synced(&self) -> bool {
        self.sntp.get_sync_status() == SyncStatus::Completed
            || SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() > 1_700_000_000).unwrap_or(false)
    }
}

/// Local broken-down time: (year, month 1-12, day, hour, minute, second, weekday 0 = Monday).
pub fn local_now() -> (i32, u32, u32, u32, u32, u32, u32) {
    unsafe {
        let mut t: esp_idf_svc::sys::time_t = 0;
        esp_idf_svc::sys::time(&mut t);
        let mut tm: esp_idf_svc::sys::tm = core::mem::zeroed();
        esp_idf_svc::sys::localtime_r(&t, &mut tm);
        let wd_mon0 = ((tm.tm_wday + 6) % 7) as u32;
        (tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32, tm.tm_hour as u32, tm.tm_min as u32, tm.tm_sec as u32, wd_mon0)
    }
}

pub fn iso_now() -> String {
    let (y, mo, d, h, mi, s, _) = local_now();
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}")
}
