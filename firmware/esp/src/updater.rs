//! Once a day: fetch the release manifest (settings `update_manifest`) and offer a newer version in
//! /api/v1/info, the page and the Home Assistant update entity. Never installs anything by itself.

use std::sync::Arc;
use std::time::Duration;

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

pub fn spawn(shared: Arc<Shared>, url: String) -> Result<()> {
    if url.is_empty() {
        return Ok(());
    }
    std::thread::Builder::new().name("update".into()).stack_size(10 * 1024).spawn(move || loop {
        // first check a few minutes after boot, then once a day
        std::thread::sleep(Duration::from_secs(300));
        match fetch(&url) {
            Ok(text) => match parse_manifest(&text, &url) {
                Some(u) if newer(crate::VERSION, &u.version) => {
                    log::info!("update available: {}", u.version);
                    let mut gw = shared.gw.lock().unwrap();
                    gw.info.latest_version = Some(u.version.clone());
                    gw.info.latest_url = Some(u.url);
                    gw.push_event("update", &format!("firmware {} available", u.version));
                }
                Some(_) => log::info!("firmware up to date"),
                None => log::warn!("update manifest unreadable"),
            },
            Err(e) => log::warn!("update check failed: {e}"),
        }
        std::thread::sleep(Duration::from_secs(24 * 3600 - 300));
    })?;
    Ok(())
}
