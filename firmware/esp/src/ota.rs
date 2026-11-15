//! OTA over Wi-Fi on TCP 7001 (protocol unchanged from the sniffer):
//!   client -> "AZOTA1 <token> <size>\n"; server -> "OK ready\n"; client -> image; server -> "OK done, rebooting\n" | "ERR …"
//! A new image boots "pending verify". It is confirmed only by a successful authenticated probe
//! (`AZOTA1 <token> 0`, sent by tools/ota_upload.py after an upload), which proves the OTA path of the
//! new image works; without it the board resets after OTA_CONFIRM_S and the bootloader rolls back.
//! An image the gateway downloaded itself (one-click update, `installer`) confirms itself instead once it
//! has Wi-Fi (`self_confirm`).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use esp_idf_svc::sys;

use crate::nvs_store::Store;

pub const OTA_PORT: u16 = 7001;
pub const OTA_CONFIRM_S: u64 = 180;
static APP_VALID: AtomicBool = AtomicBool::new(false);

pub fn pending_verify() -> bool {
    unsafe {
        let running = sys::esp_ota_get_running_partition();
        let mut st: sys::esp_ota_img_states_t = 0;
        sys::esp_ota_get_state_partition(running, &mut st) == sys::ESP_OK
            && st == sys::esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY
    }
}

/// Reset (→ bootloader rollback) if the image is not confirmed in time.
pub fn start_guard() -> Result<()> {
    if !pending_verify() {
        return Ok(());
    }
    log::warn!("new OTA image, pending verify: must confirm within {OTA_CONFIRM_S} s");
    std::thread::Builder::new().name("ota-guard".into()).stack_size(3 * 1024).spawn(|| {
        std::thread::sleep(Duration::from_secs(OTA_CONFIRM_S));
        if !APP_VALID.load(Ordering::SeqCst) {
            log::error!("image not confirmed, resetting -> bootloader rollback");
            unsafe { sys::esp_restart() };
        }
    })?;
    Ok(())
}

/// Healthy time before an image installed by the gateway itself confirms itself.
pub const SELF_CONFIRM_S: u64 = 60;

/// One-click updates: an image the gateway installed itself (NVS `self_update`) confirms itself once
/// Wi-Fi has an IP and SELF_CONFIRM_S have passed since boot. Without Wi-Fi it never confirms, so the
/// guard resets after OTA_CONFIRM_S and the bootloader rolls back. A flag left on a confirmed image
/// (rollback, or an install that never booted) is cleared; a rollback is returned so that main can show it.
pub fn self_confirm(store: Arc<Mutex<Store>>) -> Result<Option<RolledBack>> {
    if !store.lock().unwrap().self_update() {
        return Ok(None);
    }
    if !pending_verify() {
        let rb = rolled_back();
        match &rb {
            Some(r) => log::warn!("self-installed image {} did not start: rolled back", r.version),
            None => log::info!("self_update flag on a confirmed image (install never booted): cleared"),
        }
        // cleared either way, so the message is shown once, not after every restart
        if let Err(e) = store.lock().unwrap().set_self_update(false) {
            log::error!("self_update flag not cleared: {e}");
        }
        return Ok(rb);
    }
    log::warn!("self-installed image: confirms itself after {SELF_CONFIRM_S} s with Wi-Fi");
    std::thread::Builder::new().name("self-confirm".into()).stack_size(4 * 1024).spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        if crate::now_ms() / 1000 < SELF_CONFIRM_S || !wifi_has_ip() {
            continue;
        }
        match confirm() {
            Ok(_) => {
                if let Err(e) = store.lock().unwrap().set_self_update(false) {
                    log::error!("self_update flag not cleared: {e}");
                }
                log::info!("update confirmed");
            }
            Err(e) => log::error!("self-confirm failed: {e}"),
        }
        return;
    })?;
    Ok(None)
}

/// A self-installed image the bootloader rolled back (its version, from the image header; may be empty).
pub struct RolledBack {
    pub version: String,
}

/// The bootloader marks a pending image that reset before confirming as ABORTED (INVALID when the app
/// rejects itself) and boots the previous one. `esp_ota_get_last_invalid_partition` returns that slot only
/// if its otadata entry is INVALID/ABORTED and the image in it still validates, so an install interrupted
/// while writing (slot erased or half written, no new otadata entry) is not taken for a rollback.
fn rolled_back() -> Option<RolledBack> {
    unsafe {
        let bad = sys::esp_ota_get_last_invalid_partition();
        let running = sys::esp_ota_get_running_partition();
        if bad.is_null() || running.is_null() || (*bad).address == (*running).address {
            return None;
        }
        let mut d: sys::esp_app_desc_t = core::mem::zeroed();
        let version = if sys::esp_ota_get_partition_description(bad, &mut d) == sys::ESP_OK {
            let v = &d.version;
            let end = v.iter().position(|c| *c == 0).unwrap_or(v.len());
            String::from_utf8_lossy(&v[..end].iter().map(|c| *c as u8).collect::<Vec<u8>>()).into_owned()
        } else {
            String::new()
        };
        Some(RolledBack { version })
    }
}

fn wifi_has_ip() -> bool {
    unsafe {
        let netif = sys::esp_netif_get_handle_from_ifkey(c"WIFI_STA_DEF".as_ptr());
        if netif.is_null() || !sys::esp_netif_is_netif_up(netif) {
            return false;
        }
        let mut ip: sys::esp_netif_ip_info_t = core::mem::zeroed();
        sys::esp_netif_get_ip_info(netif, &mut ip) == sys::ESP_OK && ip.ip.addr != 0
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn server(token: String) {
    let listener = loop {
        match TcpListener::bind(("0.0.0.0", OTA_PORT)) {
            Ok(l) => break l,
            Err(e) => {
                log::warn!("OTA bind failed: {e}");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    };
    if !pending_verify() {
        APP_VALID.store(true, Ordering::SeqCst);
    }
    log::info!("OTA server on port {OTA_PORT} (image confirmed: {})", APP_VALID.load(Ordering::SeqCst));
    if token.len() < 16 {
        log::warn!("no OTA token configured: TCP OTA disabled");
    }
    for conn in listener.incoming() {
        let Ok(mut s) = conn else { continue };
        if token.len() < 16 {
            let _ = writeln!(s, "ERR OTA disabled");
            continue;
        }
        match session(&mut s, &token) {
            Ok(Outcome::Probe) => match confirm() {
                Ok(build) => {
                    let _ = writeln!(s, "OK probe build={build}");
                }
                Err(e) => {
                    let _ = writeln!(s, "ERR probe: {e}");
                }
            },
            Ok(Outcome::Updated) => {
                log::info!("OTA complete, rebooting");
                let _ = writeln!(s, "OK done, rebooting");
                let _ = s.flush();
                std::thread::sleep(Duration::from_millis(500));
                unsafe { sys::esp_restart() };
            }
            Err(e) => {
                log::warn!("OTA failed: {e}");
                let _ = writeln!(s, "ERR {e}");
            }
        }
    }
}

enum Outcome {
    Probe,
    Updated,
}

/// Confirm the running image after an authenticated probe (TCP 7001) or `POST /api/v1/ota/confirm`:
/// checks the OTA slot, cancels the rollback, returns the build id.
pub fn confirm() -> Result<String> {
    slot_ready()?;
    if !APP_VALID.load(Ordering::SeqCst) {
        let ok = unsafe { sys::esp_ota_mark_app_valid_cancel_rollback() } == sys::ESP_OK;
        APP_VALID.store(ok, Ordering::SeqCst);
        log::info!("OTA confirm: running image confirmed = {ok}");
        if !ok {
            anyhow::bail!("could not confirm the image");
        }
    }
    Ok(crate::BUILD_UNIX.to_string())
}

/// The next OTA slot exists and can hold an image as large as the running one (no erase here: the
/// slot holds the rollback image).
fn slot_ready() -> Result<()> {
    unsafe {
        let next = sys::esp_ota_get_next_update_partition(std::ptr::null());
        let running = sys::esp_ota_get_running_partition();
        if next.is_null() || running.is_null() {
            anyhow::bail!("no OTA slot");
        }
        if (*next).size < (*running).size {
            anyhow::bail!("OTA slot too small");
        }
    }
    Ok(())
}

fn session(s: &mut TcpStream, token: &str) -> Result<Outcome> {
    // short header timeout: an idle client must not hold the port during the confirm window
    s.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut hdr = Vec::with_capacity(96);
    let mut b = [0u8; 1];
    loop {
        s.read_exact(&mut b)?;
        if b[0] == b'\n' {
            break;
        }
        hdr.push(b[0]);
        if hdr.len() > 128 {
            anyhow::bail!("header too long");
        }
    }
    let hdr = String::from_utf8(hdr)?;
    let mut it = hdr.trim().split(' ');
    if it.next() != Some("AZOTA1") {
        anyhow::bail!("bad magic");
    }
    if !ct_eq(it.next().unwrap_or("").as_bytes(), token.as_bytes()) {
        std::thread::sleep(Duration::from_secs(2));
        anyhow::bail!("bad token");
    }
    let size: usize = it.next().ok_or_else(|| anyhow::anyhow!("missing size"))?.parse()?;
    if size == 0 {
        return Ok(Outcome::Probe);
    }
    s.set_read_timeout(Some(Duration::from_secs(20)))?;
    write_image(s, size)?;
    Ok(Outcome::Updated)
}

/// Copy `size` bytes produced by `read` into the next OTA slot and select it (HTTP upload path).
pub fn write_stream(size: usize, mut read: impl FnMut(&mut [u8]) -> Result<usize>) -> Result<()> {
    struct R<F>(F);
    impl<F: FnMut(&mut [u8]) -> Result<usize>> Read for R<F> {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            (self.0)(b).map_err(|e| std::io::Error::other(e.to_string()))
        }
    }
    impl<F> Write for R<F> {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    write_image(&mut R(&mut read), size)
}

/// Erase the next OTA slot, answer "OK ready", copy `size` bytes from the same connection into the
/// slot and select it for the next boot. (No socket cloning: lwIP on ESP-IDF does not support dup.)
/// Only one OTA writer at a time (one-click installer, HTTP upload, TCP 7001): two interleaved
/// begin/write sequences on the same slot would fail both and erase the rollback image.
static OTA_BUSY: AtomicBool = AtomicBool::new(false);
pub const OTA_BUSY_MSG: &str = "another firmware update is in progress";

struct BusyGuard;

impl BusyGuard {
    fn take() -> Result<Self> {
        OTA_BUSY
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| BusyGuard)
            .map_err(|_| anyhow::anyhow!(OTA_BUSY_MSG))
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        OTA_BUSY.store(false, Ordering::SeqCst);
    }
}

pub fn write_image<S: Read + Write>(s: &mut S, size: usize) -> Result<()> {
    // released on every exit path (errors, `?`, panics) when the guard drops
    let _busy = BusyGuard::take()?;
    unsafe {
        let part = sys::esp_ota_get_next_update_partition(std::ptr::null());
        if part.is_null() {
            anyhow::bail!("no OTA slot");
        }
        let slot = (*part).size as usize;
        if size > slot {
            anyhow::bail!("bad size {size} (slot {slot})");
        }
        let mut h: sys::esp_ota_handle_t = 0;
        sys::esp!(sys::esp_ota_begin(part, size, &mut h))?;
        if let Err(e) = writeln!(s, "OK ready") {
            sys::esp_ota_abort(h);
            return Err(e.into());
        }
        let mut buf = vec![0u8; 4096];
        let mut done = 0usize;
        let copied: Result<()> = (|| {
            while done < size {
                let want = buf.len().min(size - done);
                let n = s.read(&mut buf[..want])?;
                if n == 0 {
                    anyhow::bail!("connection closed at {done}/{size}");
                }
                sys::esp!(sys::esp_ota_write(h, buf.as_ptr() as *const _, n))?;
                done += n;
            }
            Ok(())
        })();
        if let Err(e) = copied {
            sys::esp_ota_abort(h);
            return Err(e);
        }
        // esp_ota_end verifies the image (header, checksum, SHA-256) before we switch to it.
        sys::esp!(sys::esp_ota_end(h))?;
        sys::esp!(sys::esp_ota_set_boot_partition(part))?;
    }
    Ok(())
}
