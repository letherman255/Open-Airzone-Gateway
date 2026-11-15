//! Once a day: fetch the release manifest (settings `update_manifest`) and offer a newer version in
//! /api/v1/info, the page and the Home Assistant update entity. Never installs anything by itself
//! (installing is `installer`, started from the API only).

use std::sync::Arc;
use std::time::{Duration, Instant};

use airzone_core::update::{install_busy, newer, parse_manifest};
use anyhow::Result;
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use esp_idf_svc::http::Method;
use esp_idf_svc::sys;

use crate::Shared;

const MAX_MANIFEST: usize = 16 * 1024;

fn fetch(url: &str) -> Result<String> {
    let mut c = EspHttpConnection::new(&Configuration {
        crt_bundle_attach: Some(sys::esp_crt_bundle_attach),
        timeout: Some(Duration::from_secs(15)),
        ..Default::default()
    })?;
    c.initiate_request(Method::Get, url, &[("User-Agent", "open-airzone-gateway")])?;
    c.initiate_response()?;
    if c.status() != 200 {
        anyhow::bail!("HTTP {}", c.status());
    }
    let mut body = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = c.read(&mut buf)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
        if body.len() > MAX_MANIFEST {
            anyhow::bail!("manifest too large");
        }
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Always running: the manifest URL is read from the live settings every minute, so turning the check on
/// or changing the URL takes effect without a restart. A changed URL is checked at once; otherwise the
/// first check is 5 min after boot, then once a day (one hour after a failed check). No check runs while
/// a one-click install is busy. An empty URL means idle and withdraws any offer and the last result.
/// The result of the last check is `info.update_checked` (`<when> ok` / `<when> failed: <reason>`).
pub fn spawn(shared: Arc<Shared>) -> Result<()> {
    std::thread::Builder::new().name("update".into()).stack_size(10 * 1024).spawn(move || {
        let mut seen = shared.gw.lock().unwrap().settings.update_manifest.clone();
        let mut last_check: Option<(Instant, bool)> = None;
        loop {
            std::thread::sleep(Duration::from_secs(60));
            let (url, busy) = {
                let gw = shared.gw.lock().unwrap();
                (gw.settings.update_manifest.clone(), install_busy(&gw.info.update_state))
            };
            // the installer works from the offer: leave it (and any settings change, `seen` is kept) alone
            // until the install is over
            if busy {
                continue;
            }
            let changed = url != seen;
            seen = url.clone();
            if changed {
                clear_offer(&shared);
                shared.gw.lock().unwrap().info.update_checked.clear();
            }
            if url.is_empty() {
                last_check = None;
                continue;
            }
            let due = match last_check {
                _ if changed => true,
                None => crate::now_ms() >= FIRST_CHECK_MS,
                Some((t, true)) => t.elapsed() >= Duration::from_secs(24 * 3600),
                Some((t, false)) => t.elapsed() >= Duration::from_secs(RETRY_S),
            };
            if !due {
                continue;
            }
            let ok = check(&shared, &url);
            last_check = Some((Instant::now(), ok));
        }
    })?;
    Ok(())
}

const FIRST_CHECK_MS: u64 = 300_000;
/// Retry delay after a failed check (network down, manifest unreadable).
const RETRY_S: u64 = 3600;

fn clear_offer(shared: &Shared) {
    let mut gw = shared.gw.lock().unwrap();
    gw.info.latest_version = None;
    gw.info.latest_url = None;
}

/// `2026-11-15T10:00:00` once SNTP set the clock, `uptime 312 s` before.
fn when() -> String {
    if crate::clock::Clock::time_valid() {
        crate::clock::iso_now()
    } else {
        format!("uptime {} s", crate::now_ms() / 1000)
    }
}

/// One check; false when the manifest could not be fetched or read.
fn check(shared: &Shared, url: &str) -> bool {
    // network I/O without any lock held
    let res = fetch(url).and_then(|text| parse_manifest(&text, url).ok_or_else(|| anyhow::anyhow!("manifest unreadable")));
    let mut gw = shared.gw.lock().unwrap();
    // the setting may have changed during the fetch: an answer for an old URL is dropped
    if gw.settings.update_manifest != url {
        return true;
    }
    match res {
        Ok(u) if newer(crate::VERSION, &u.version) => {
            log::info!("update available: {}", u.version);
            if gw.info.latest_version.as_deref() != Some(u.version.as_str()) {
                gw.push_event("update", &format!("firmware {} available", u.version));
            }
            gw.info.latest_version = Some(u.version.clone());
            gw.info.latest_url = Some(u.url);
            gw.info.update_checked = format!("{} ok", when());
            true
        }
        Ok(_) => {
            log::info!("firmware up to date");
            gw.info.latest_version = None;
            gw.info.latest_url = None;
            gw.info.update_checked = format!("{} ok", when());
            true
        }
        Err(e) => {
            log::warn!("update check failed: {e}");
            gw.info.update_checked = format!("{} failed: {e}", when());
            false
        }
    }
}
