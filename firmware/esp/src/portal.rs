//! First-time setup: open access point `airzone-gw-XXXX` with a captive portal (every DNS name answers
//! 192.168.4.1) serving one form. Saving writes NVS and reboots. Mode stays listen.

use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use airzone_core::settings::Settings;
use anyhow::Result;
use esp_idf_svc::http::server::{Configuration as HttpConf, EspHttpServer};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;
use esp_idf_svc::wifi::{AccessPointConfiguration, AuthMethod, BlockingWifi, Configuration, EspWifi};

use crate::nvs_store::Store;

pub fn decode_form(body: &str) -> Vec<(String, String)> {
    fn unescape(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'+' => out.push(b' '),
                b'%' if i + 2 < b.len() => {
                    match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                        Ok(v) => {
                            out.push(v);
                            i += 2;
                        }
                        Err(_) => out.push(b'%'),
                    }
                }
                c => out.push(c),
            }
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
    body.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (unescape(k), unescape(v)))
        .collect()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn page(s: &Settings) -> String {
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Airzone gateway setup</title><style>body{{font-family:sans-serif;max-width:28rem;margin:1rem auto;padding:0 1rem}}
label{{display:block;margin-top:.8rem}}input{{width:100%;padding:.4rem;box-sizing:border-box}}button{{margin-top:1rem;padding:.6rem 1rem}}</style></head>
<body><h1>Airzone gateway setup</h1><p>The gateway starts in <b>listen mode</b>: it never transmits on the bus.</p>
<form method="post" action="/save">
<label>Wi-Fi network (2.4 GHz)<input name="wifi_ssid" value="{}" maxlength="32" required></label>
<label>Wi-Fi password<input name="wifi_pass" type="password" maxlength="64"></label>
<label>MQTT broker URL (e.g. mqtt://192.168.1.10:1883)<input name="mqtt_url" value="{}"></label>
<label>MQTT user<input name="mqtt_user" value="{}"></label>
<label>MQTT password<input name="mqtt_pass" type="password"></label>
<button type="submit">Save and restart</button></form></body></html>"#,
        html_escape(&s.wifi_ssid),
        html_escape(if s.mqtt_url.contains('@') { "" } else { &s.mqtt_url }),
        html_escape(&s.mqtt_user)
    )
}

/// Answer every DNS query with 192.168.4.1 (captive portal detection on phones).
fn dns_catch_all() {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:53") else { return };
    let mut buf = [0u8; 512];
    loop {
        let Ok((n, peer)) = sock.recv_from(&mut buf) else { continue };
        if n < 12 {
            continue;
        }
        let mut r = buf[..n].to_vec();
        r[2] = 0x81;
        r[3] = 0x80;
        r[6] = 0;
        r[7] = 1;
        r[8..12].copy_from_slice(&[0, 0, 0, 0]);
        r.extend_from_slice(&[0xC0, 0x0C, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 168, 4, 1]);
        let _ = sock.send_to(&r, peer);
    }
}

/// Run the portal until the form is saved, then reboot. Never returns.
pub fn run(wifi: &mut BlockingWifi<EspWifi<'static>>, store: Arc<Mutex<Store>>, current: Settings, ap_name: &str) -> Result<()> {
    log::warn!("starting setup access point {ap_name}");
    let _ = wifi.stop();
    wifi.set_configuration(&Configuration::AccessPoint(AccessPointConfiguration {
        ssid: ap_name.try_into().map_err(|_| anyhow::anyhow!("AP name"))?,
        auth_method: AuthMethod::None,
        channel: 6,
        ..Default::default()
    }))?;
    wifi.start()?;
    wifi.wait_netif_up()?;
    std::thread::Builder::new().name("dns".into()).stack_size(4 * 1024).spawn(dns_catch_all)?;

    let mut server = EspHttpServer::new(&HttpConf { uri_match_wildcard: true, ..Default::default() })?;
    let cur = Arc::new(Mutex::new(current));
    {
        let cur = cur.clone();
        server.fn_handler("/save", Method::Post, move |mut req| -> anyhow::Result<()> {
            let mut body = vec![0u8; 2048];
            let mut n = 0;
            while n < body.len() {
                let r = req.read(&mut body[n..])?;
                if r == 0 {
                    break;
                }
                n += r;
            }
            let form = decode_form(&String::from_utf8_lossy(&body[..n]));
            let mut s = cur.lock().unwrap().clone();
            let old_url = s.mqtt_url.clone();
            for (k, v) in &form {
                match k.as_str() {
                    "wifi_ssid" if !v.is_empty() => s.wifi_ssid = v.clone(),
                    "wifi_pass" if !v.is_empty() => s.wifi_pass = v.clone(),
                    "mqtt_url" => s.mqtt_url = v.clone(),
                    "mqtt_user" => s.mqtt_user = v.clone(),
                    "mqtt_pass" if !v.is_empty() => s.mqtt_pass = v.clone(),
                    _ => {}
                }
            }
            // a stored MQTT password is never sent to a broker chosen by whoever fills the form
            if s.mqtt_url != old_url && !form.iter().any(|(k, v)| k == "mqtt_pass" && !v.is_empty()) {
                s.mqtt_pass.clear();
            }
            store.lock().unwrap().save(&s)?;
            req.into_ok_response()?.write_all(b"<p>Saved. The gateway restarts and joins your network.</p>")?;
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_secs(2));
                unsafe { esp_idf_svc::sys::esp_restart() };
            });
            Ok(())
        })?;
    }
    {
        let cur = cur.clone();
        server.fn_handler("/*", Method::Get, move |req| -> anyhow::Result<()> {
            let html = page(&cur.lock().unwrap());
            req.into_response(200, None, &[("Content-Type", "text/html; charset=utf-8")])?.write_all(html.as_bytes())?;
            Ok(())
        })?;
    }
    // give the network another chance: after 10 min without a saved form, restart in station mode
    std::thread::sleep(Duration::from_secs(600));
    log::warn!("setup portal timeout, restarting");
    unsafe { esp_idf_svc::sys::esp_restart() };
    #[allow(unreachable_code)]
    Ok(())
}
