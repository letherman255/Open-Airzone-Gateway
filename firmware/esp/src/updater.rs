//! Once a day: fetch the release manifest (settings `update_manifest`) and offer a newer version in
//! /api/v1/info, the page and the Home Assistant update entity. Never installs anything by itself
//! (installing is `installer`, started from the API only).

use std::sync::Arc;
use std::time::{Duration, Instant};

use airzone_core::update::{newer, parse_manifest};
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
/// first check is 5 min after boot, then once a day. An empty URL means idle and withdraws any offer.
pub fn spawn(shared: Arc<Shared>) -> Result<()> {
    std::thread::Builder::new().name("update".into()).stack_size(10 * 1024).spawn(move || {
        let mut seen = shared.gw.lock().unwrap().settings.update_manifest.clone();
        let mut last_check: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_secs(60));
            let url = shared.gw.lock().unwrap().settings.update_manifest.clone();
            let changed = url != seen;
            seen = url.clone();
            if changed {
                clear_offer(&shared);
            }
            if url.is_empty() {
                last_check = None;
                continue;
            }
            let due = match last_check {
                _ if changed => true,
                None => crate::now_ms() >= FIRST_CHECK_MS,
                Some(t) => t.elapsed() >= Duration::from_secs(24 * 3600),
            };
            if !due {
                continue;
            }
            last_check = Some(Instant::now());
            check(&shared, &url);
        }
    })?;
    Ok(())
}

const FIRST_CHECK_MS: u64 = 300_000;

fn clear_offer(shared: &Shared) {
    let mut gw = shared.gw.lock().unwrap();
    gw.info.latest_version = None;
    gw.info.latest_url = None;
}

fn check(shared: &Shared, url: &str) {
    // network I/O without any lock held
    match fetch(url) {
        Ok(text) => match parse_manifest(&text, url) {
            Some(u) if newer(crate::VERSION, &u.version) => {
                log::info!("update available: {}", u.version);
                let mut gw = shared.gw.lock().unwrap();
                // the setting may have changed during the fetch: an answer for an old URL is dropped
                if gw.settings.update_manifest != url {
                    return;
                }
                if gw.info.latest_version.as_deref() != Some(u.version.as_str()) {
                    gw.push_event("update", &format!("firmware {} available", u.version));
                }
                gw.info.latest_version = Some(u.version.clone());
                gw.info.latest_url = Some(u.url);
            }
            Some(_) => {
                log::info!("firmware up to date");
                clear_offer(shared);
            }
            None => log::warn!("update manifest unreadable"),
        },
        Err(e) => log::warn!("update check failed: {e}"),
    }
}
