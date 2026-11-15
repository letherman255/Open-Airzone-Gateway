//! HTTP server: the installer page, the JSON API (core `api::handle`), the WebSocket live feed and the
//! HTTP OTA upload.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use airzone_core::api::{handle, ApiRequest};
use airzone_core::gateway::Action;
use airzone_core::ha::{js, states, HaConfig};
use airzone_core::update::{check_image, MAX_IMAGE, MIN_IMAGE};
use anyhow::Result;
use esp_idf_svc::http::server::ws::EspHttpWsDetachedSender;
use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;
use esp_idf_svc::sys;
use esp_idf_svc::ws::FrameType;
use airzone_core::ws::{live_messages, WsCursor};

use crate::Shared;

const INDEX: &str = include_str!("../../../web/index.html");
const APP: &str = include_str!("../../../web/app.js");
const STYLE: &str = include_str!("../../../web/style.css");
const MAX_BODY: usize = 16 * 1024;

/// WebSocket counters for remote diagnosis (reported on the frame stream).
pub static WS_CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static WS_CONNECTS:std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static WS_TICKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static WS_SENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static WS_SEND_ERR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn random32() -> [u8; 32] {
    let mut b = [0u8; 32];
    unsafe { sys::esp_fill_random(b.as_mut_ptr() as *mut _, b.len()) };
    b
}

/// Copy the live bus view into the gateway state, then run the API.
fn api_call(shared: &Arc<Shared>, req: ApiRequest) -> airzone_core::api::ApiResponse {
    let mut gw = shared.gw.lock().unwrap();
    // listen mode: copy the listener's view (the master engine keeps gw up to date itself)
    if !gw.master_active() || !gw.info.master_running {
        gw.registry = shared.listener.lock().unwrap().registry().clone();
        gw.topology = shared.topology.lock().unwrap().clone();
    }
    crate::refresh_info(&mut gw);
    let resp = handle(&mut gw, &req, crate::now_ms(), random32());
    apply_log_level(&gw.log_level);
    let actions = std::mem::take(&mut gw.actions);
    let settings = gw.settings.clone();
    drop(gw);
    for a in actions {
        match a {
            Action::SaveSettings => {
                if let Err(e) = shared.store.lock().unwrap().save(&settings) {
                    log::error!("settings not saved: {e}");
                }
            }
            Action::Restart => crate::restart_later(),
            Action::Rediscover => shared.rediscover.store(true, std::sync::atomic::Ordering::SeqCst),
            Action::InstallUpdate { version, url } => crate::installer::start(shared.clone(), version, url),
        }
    }
    resp
}

fn apply_log_level(level: &str) {
    log::set_max_level(match level {
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "debug" => log::LevelFilter::Debug,
        _ => log::LevelFilter::Info,
    });
}

fn header_bearer(v: Option<&str>) -> Option<String> {
    v.and_then(|h| h.strip_prefix("Bearer ")).map(|t| t.trim().to_string())
}

pub fn start(shared: Arc<Shared>, ha: HaConfig) -> Result<EspHttpServer<'static>> {
    let mut server = EspHttpServer::new(&Configuration { uri_match_wildcard: true, max_uri_handlers: 16, stack_size: 12 * 1024, ..Default::default() })?;
    for (uri, body, ctype) in [
        ("/", INDEX, "text/html; charset=utf-8"),
        ("/app.js", APP, "text/javascript; charset=utf-8"),
        ("/style.css", STYLE, "text/css; charset=utf-8"),
    ] {
        server.fn_handler(uri, Method::Get, move |req| -> Result<()> {
            req.into_response(200, None, &[("Content-Type", ctype), ("Cache-Control", "no-cache")])?.write_all(body.as_bytes())?;
            Ok(())
        })?;
    }

    // HTTP OTA: POST /api/v1/ota (bearer token, raw image body), then POST /api/v1/ota/confirm after reboot
    {
        let shared = shared.clone();
        server.fn_handler("/api/v1/ota", Method::Post, move |mut req| -> Result<()> {
            let bearer = header_bearer(req.header("Authorization"));
            let (ok, installing) = {
                let mut gw = shared.gw.lock().unwrap();
                let st = gw.info.update_state.as_str();
                let busy = st.starts_with("downloading") || st == "installing" || st == "restarting";
                (bearer.is_some_and(|t| gw.auth.bearer_ok(&t, crate::now_ms())), busy)
            };
            if !ok {
                req.into_status_response(401)?.write_all(br#"{"error":"authentication required"}"#)?;
                return Ok(());
            }
            // the one-click installer is writing the same OTA slot
            if installing {
                req.into_status_response(409)?.write_all(br#"{"error":"update already in progress"}"#)?;
                return Ok(());
            }
            // a missing / tiny size would make esp_ota_begin erase the whole rollback slot: refuse first
            let size: Option<usize> = req.header("Content-Length").and_then(|v| v.parse().ok());
            let Some(size) = size.filter(|s| (MIN_IMAGE..=MAX_IMAGE).contains(s)) else {
                req.into_status_response(411)?.write_all(br#"{"error":"Content-Length of a firmware image required"}"#)?;
                return Ok(());
            };
            let mut head = vec![0u8; 4096];
            let mut got = 0;
            while got < head.len() {
                let n = req.read(&mut head[got..]).map_err(|e| anyhow::anyhow!("{e:?}"))?;
                if n == 0 {
                    break;
                }
                got += n;
            }
            if let Err(e) = check_image(Some(size), head[0]).and_then(|()| if got < head.len() { Err("body too short") } else { Ok(()) }) {
                req.into_status_response(400)?.write_all(format!(r#"{{"error":{}}}"#, js(e)).as_bytes())?;
                return Ok(());
            }
            let mut offset = 0usize;
            let res = crate::ota::write_stream(size, |b| {
                if offset < head.len() {
                    let n = b.len().min(head.len() - offset);
                    b[..n].copy_from_slice(&head[offset..offset + n]);
                    offset += n;
                    return Ok(n);
                }
                req.read(b).map_err(|e| anyhow::anyhow!("{e:?}"))
            });
            match res {
                Ok(()) => {
                    // an uploaded image is confirmed by the uploader, never by itself
                    if let Err(e) = shared.store.lock().unwrap().set_self_update(false) {
                        log::error!("self_update flag not cleared: {e}");
                    }
                    req.into_ok_response()?.write_all(br#"{"updated":true,"restarting":true}"#)?;
                    crate::restart_later();
                }
                Err(e) => {
                    req.into_status_response(400)?.write_all(format!(r#"{{"error":{}}}"#, js(&e.to_string())).as_bytes())?;
                }
            }
            Ok(())
        })?;
    }
    {
        let shared = shared.clone();
        server.fn_handler("/api/v1/ota/confirm", Method::Post, move |req| -> Result<()> {
            let bearer = header_bearer(req.header("Authorization"));
            let ok = bearer.is_some_and(|t| shared.gw.lock().unwrap().auth.bearer_ok(&t, crate::now_ms()));
            let (code, body) = if !ok {
                (401, r#"{"error":"authentication required"}"#.to_string())
            } else {
                match crate::ota::confirm() {
                    Ok(b) => (200, format!(r#"{{"confirmed":true,"build":{}}}"#, js(&b))),
                    Err(e) => (500, format!(r#"{{"error":{}}}"#, js(&e.to_string()))),
                }
            };
            req.into_status_response(code)?.write_all(body.as_bytes())?;
            Ok(())
        })?;
    }

    // WebSocket live feed: one "values" message every 2 s to every open session
    let senders: Arc<Mutex<Vec<(EspHttpWsDetachedSender, WsCursor)>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let senders = senders.clone();
        server.ws_handler("/api/v1/ws", None, move |ws| -> Result<(), sys::EspError> {
            WS_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // ESP-IDF 5.5 does not call the handler after the handshake (only for frames and on close,
            // seen 2026-11-06): clients send one text frame after opening, and the first frame registers
            // the session
            if !ws.is_new() && !ws.is_closed() {
                let mut buf = [0u8; 64];
                let _ = ws.recv(&mut buf);
            }
            let fd = ws.session();
            let known = senders.lock().unwrap().iter().any(|(s, _)| s.session() == fd);
            if !ws.is_closed() && !known {
                WS_CONNECTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let s = ws.create_detached_sender()?;
                let mut v = senders.lock().unwrap();
                if v.len() >= 4 {
                    v.remove(0);
                }
                // a new client starts now: no full-registry dump on its first tick
                v.push((s, WsCursor { ms: crate::now_ms(), ..WsCursor::default() }));
            }
            Ok(())
        })?;
    }
    {
        let shared = shared.clone();
        std::thread::Builder::new().name("ws".into()).stack_size(8 * 1024).spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            WS_TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut v = senders.lock().unwrap();
            crate::stream::RECENT_ON.store(!v.is_empty(), std::sync::atomic::Ordering::Relaxed);
            if v.is_empty() {
                if let Ok(mut r) = crate::stream::RECENT.lock() {
                    r.clear();
                    r.shrink_to_fit();
                }
                if let Ok(mut gw) = shared.gw.try_lock() {
                    gw.frames.clear();
                    gw.frames.shrink_to_fit();
                }
                continue;
            }
            crate::crumb("ws tick");
            let values = {
                let topo = shared.topology.lock().unwrap().clone();
                let prefix = format!("{}/{}/", ha.prefix, ha.gateway_id);
                let items: Vec<String> = shared
                    .with_registry(|reg| states(&ha, &topo, reg))
                    .into_iter()
                    .map(|m| format!(r#"{{"topic":{},"state":{}}}"#, js(m.topic.trim_start_matches(&prefix)), m.payload))
                    .collect();
                format!(r#"{{"topic":"values","states":[{}]}}"#, items.join(","))
            };
            let recent: Vec<String> = crate::stream::RECENT.lock().map(|mut r| r.drain(..).collect()).unwrap_or_default();
            let listener_view =
                (!shared.master_running.load(std::sync::atomic::Ordering::SeqCst)).then(|| shared.listener.lock().unwrap().registry().clone());
            let now = crate::now_ms();
            // build every client's messages under the lock, send after releasing it (a slow client must not
            // block the API, MQTT or the updater)
            let batches: Vec<Vec<String>> = {
                let mut gw = shared.gw.lock().unwrap();
                for f in recent {
                    gw.push_frame(f);
                }
                if let Some(r) = listener_view {
                    gw.registry = r;
                }
                v.iter_mut().map(|(_, cursor)| live_messages(&mut gw, cursor, values.clone(), now)).collect()
            };
            let mut i = 0;
            v.retain_mut(|(s, _)| {
                let msgs = &batches[i];
                i += 1;
                !s.is_closed()
                    && msgs.iter().all(|m| match s.send(FrameType::Text(false), m.as_bytes()) {
                        Ok(()) => {
                            WS_SENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            true
                        }
                        Err(e) => {
                            WS_SEND_ERR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            log::warn!("ws send failed: {e:?}");
                            false
                        }
                    })
            });
        })?;
    }

    // the catalogue (~52 KB) is sent straight from flash: going through the API copied it twice on the heap,
    // and with the master engine running no 52 KB block was left (panic, 2026-11-06). Registered before
    // the wildcard so it takes precedence.
    server.fn_handler("/api/v1/catalogue", Method::Get, move |req| -> Result<()> {
        let mut resp = req.into_response(200, None, &[("Content-Type", "application/json"), ("Cache-Control", "no-cache")])?;
        for chunk in crate::CATALOGUE.as_bytes().chunks(4096) {
            resp.write_all(chunk)?;
        }
        Ok(())
    })?;

    for method in [Method::Get, Method::Post, Method::Delete] {
        let shared = shared.clone();
        server.fn_handler("/api/v1/*", method, move |mut req| -> Result<()> {
            let uri = req.uri().to_string();
            let (path, query) = uri.split_once('?').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((uri.clone(), String::new()));
            let bearer = header_bearer(req.header("Authorization"));
            let len: usize = req.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
            if len > MAX_BODY {
                req.into_status_response(413)?.write_all(br#"{"error":"body too large"}"#)?;
                return Ok(());
            }
            let mut body = vec![0u8; len];
            let mut n = 0;
            while n < len {
                let r = req.read(&mut body[n..]).map_err(|e| anyhow::anyhow!("{e:?}"))?;
                if r == 0 {
                    break;
                }
                n += r;
            }
            let content_type = req.header("Content-Type").map(String::from);
            let host = req.header("Host").map(String::from);
            let origin = req.header("Origin").map(String::from);
            let api_req = ApiRequest {
                content_type,
                host,
                origin,
                method: match method {
                    Method::Get => "GET",
                    Method::Post => "POST",
                    _ => "DELETE",
                }
                .into(),
                path,
                query,
                body: String::from_utf8_lossy(&body[..n]).into_owned(),
                bearer,
            };
            let resp = api_call(&shared, api_req);
            req.into_response(resp.status, None, &[("Content-Type", resp.content_type), ("Cache-Control", "no-store")])?
                .write_all(resp.body.as_bytes())?;
            Ok(())
        })?;
    }
    Ok(server)
}
