//! One-click update: download the release image over HTTPS straight into the next OTA slot (no whole-image
//! buffer), select it and restart. The new image confirms itself once it is healthy (`ota::self_confirm`);
//! otherwise the 180 s guard resets and the bootloader rolls back. Started only by the API
//! (`POST /api/v1/update/install`, admin token), never automatically, never over MQTT.

use std::sync::Arc;
use std::time::Duration;

use airzone_core::update::check_image;
use anyhow::Result;
use esp_idf_svc::http::client::{Configuration, EspHttpConnection, FollowRedirectsPolicy};
use esp_idf_svc::http::Method;
use esp_idf_svc::sys;

use crate::Shared;

const MAX_REDIRECTS: usize = 3;
const BUF: usize = 4096;

fn set_state(shared: &Shared, state: String) {
    shared.gw.lock().unwrap().info.update_state = state;
}

/// Start the install in its own thread. `update_state` is already `downloading 0%` (set by the API).
pub fn start(shared: Arc<Shared>, version: String, url: String) {
    let s = shared.clone();
    let spawned = std::thread::Builder::new().name("install".into()).stack_size(12 * 1024).spawn(move || {
        log::info!("installing firmware {version} from {url}");
        match install(&s, &url) {
            Ok(()) => {
                log::info!("firmware {version} installed, restarting");
                let mut gw = s.gw.lock().unwrap();
                gw.info.update_state = "restarting".into();
                gw.push_event("update", &format!("update installed {version}"));
                drop(gw);
                crate::restart_later();
            }
            Err(e) => {
                log::warn!("update {version} failed: {e}");
                if let Err(e) = s.store.lock().unwrap().set_self_update(false) {
                    log::error!("self_update flag not cleared: {e}");
                }
                let mut gw = s.gw.lock().unwrap();
                gw.info.update_state = format!("failed: {e}");
                gw.push_event("update", &format!("update {version} failed: {e}"));
            }
        }
    });
    if let Err(e) = spawned {
        set_state(&shared, format!("failed: {e}"));
    }
}

/// Open `url` (https only), following at most MAX_REDIRECTS redirects by hand (the client's own redirect
/// loop has no limit). Each hop gets a fresh connection so two TLS sessions never coexist on the heap.
fn open(url: &str) -> Result<EspHttpConnection> {
    let mut url = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        if !url.starts_with("https://") {
            anyhow::bail!("URL must be https");
        }
        let mut c = EspHttpConnection::new(&Configuration {
            crt_bundle_attach: Some(sys::esp_crt_bundle_attach),
            timeout: Some(Duration::from_secs(30)),
            follow_redirects_policy: FollowRedirectsPolicy::FollowNone,
            ..Default::default()
        })?;
        c.initiate_request(Method::Get, &url, &[("User-Agent", "open-airzone-gateway")])?;
        c.initiate_response()?;
        match c.status() {
            200 => return Ok(c),
            301 | 302 | 303 | 307 | 308 => {
                let loc = c.header("Location").ok_or_else(|| anyhow::anyhow!("redirect without Location"))?;
                url = resolve(&url, loc);
            }
            s => anyhow::bail!("HTTP {s}"),
        }
    }
    anyhow::bail!("too many redirects")
}

/// Absolute redirect target (`Location` may be relative to the current host).
fn resolve(base: &str, loc: &str) -> String {
    if loc.contains("://") {
        return loc.to_string();
    }
    let rest = base.split_once("://").map(|(_, r)| r).unwrap_or(base);
    let host_end = rest.find('/').map(|i| base.len() - rest.len() + i).unwrap_or(base.len());
    if loc.starts_with('/') {
        format!("{}{loc}", &base[..host_end])
    } else {
        let dir = base.rfind('/').filter(|i| *i >= host_end).map(|i| &base[..i]).unwrap_or(&base[..host_end]);
        format!("{dir}/{loc}")
    }
}

fn install(shared: &Shared, url: &str) -> Result<()> {
    let mut c = open(url)?;
    let size: Option<usize> = c.header("Content-Length").and_then(|v| v.parse().ok());
    // the first bytes are read before the OTA slot is touched: an HTML page or an empty body stops here
    let mut buf = vec![0u8; BUF];
    let mut head = 0;
    while head == 0 {
        let n = c.read(&mut buf).map_err(|e| anyhow::anyhow!("download: {e}"))?;
        if n == 0 {
            anyhow::bail!("empty download");
        }
        head = n;
    }
    check_image(size, buf[0]).map_err(|e| anyhow::anyhow!("{e}"))?;
    let size = size.unwrap_or_default();
    // the flag is set before the slot is written (and cleared on failure) so that a successful write is
    // never followed by a reboot into an image that cannot confirm itself
    shared.store.lock().unwrap().set_self_update(true).map_err(|e| anyhow::anyhow!("settings storage: {e}"))?;
    let mut offset = 0usize;
    let mut done = 0usize;
    let mut step = 0usize;
    crate::ota::write_stream(size, |b| {
        let n = if offset < head {
            let n = b.len().min(head - offset);
            b[..n].copy_from_slice(&buf[offset..offset + n]);
            offset += n;
            n
        } else {
            c.read(b).map_err(|e| anyhow::anyhow!("download: {e}"))?
        };
        done += n;
        let pct = (done.min(size) * 100 / size.max(1)) / 5 * 5;
        if done >= size {
            set_state(shared, "installing".into());
        } else if pct > step {
            step = pct;
            set_state(shared, format!("downloading {pct}%"));
        }
        Ok(n)
    })
    .map_err(|e| anyhow::anyhow!("{e}"))
}
