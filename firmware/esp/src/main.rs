//! Open Airzone gateway firmware.
//!
//! Listen mode (always, and the only mode of builds without the `master` feature): the RS-485 driver
//! enable (DE) is driven low before anything else and held low, the UART is created without a TX pin,
//! and no code path transmits on the bus.
//! Master mode needs the `master` feature AND `mode = master` in the settings AND no other master heard
//! during the 5 s listen at start; otherwise the gateway stays a listener.

mod board;
mod button;
mod clock;
mod http;
#[cfg(feature = "master")]
mod master_run;
mod mqtt;
mod nvs_store;
mod ota;
mod portal;
mod stream;
mod uart_rx;
mod updater;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use airzone_core::gateway::{Gateway, GatewayInfo};
use airzone_core::ha::HaConfig;
use airzone_core::listen::Listener;
use airzone_core::registry::Registry;
use airzone_core::settings::{Mode, Settings};
use airzone_core::topology::{build_central, Topology};
use anyhow::Result;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::{AnyIOPin, AnyOutputPin, PinDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::uart::config;
use esp_idf_svc::hal::units::Hertz;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};

use crate::board::BOARD;
use crate::uart_rx::Chunk;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_UNIX: &str = env!("BUILD_UNIX");
pub const BAUD: u32 = 19200;
/// Idle time (character times) after which the UART driver hands bytes over.
pub const RX_TIMEOUT_SYMBOLS: u8 = 3;
pub const CATALOGUE: &str = include_str!("../../../web/catalogue.json");

/// State shared between the bus, MQTT, the HTTP API and the supervisor.
pub struct Shared {
    /// The listen-mode decoder; its registry is the bus view in listen mode.
    pub listener: Mutex<Listener>,
    pub topology: Mutex<Topology>,
    pub gw: Mutex<Gateway>,
    pub store: Arc<Mutex<nvs_store::Store>>,
    /// Set by the master engine once it runs (its registry is then in `gw.registry`).
    pub master_running: AtomicBool,
    pub rediscover: AtomicBool,
    /// Registry or write results changed: MQTT publishes the states now instead of at its next 2 s tick.
    pub state_dirty: AtomicBool,
}

impl Shared {
    /// Run `f` on the current bus view (master engine mirror when it runs, listener registry otherwise),
    /// under its lock and without copying it (a copy is ~30 KB). `f` must not lock `gw` or `listener`.
    pub fn with_registry<R>(&self, f: impl FnOnce(&Registry) -> R) -> R {
        if self.master_running.load(Ordering::SeqCst) {
            f(&self.gw.lock().unwrap().registry)
        } else {
            f(self.listener.lock().unwrap().registry())
        }
    }
}

pub fn now_ms() -> u64 {
    (stream::now_us() / 1000) as u64
}

pub fn restart_later() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(2));
        unsafe { sys::esp_restart() };
    });
}

/// Live gateway facts for /api/v1/info and MQTT.
pub fn refresh_info(gw: &mut Gateway) {
    gw.info.uptime_s = now_ms() / 1000;
    gw.info.mqtt_connected = mqtt::CONNECTED.load(Ordering::Relaxed);
    gw.info.stream_dropped = stream::STREAM_DROPPED.load(Ordering::Relaxed);
    gw.info.listen_dropped = stream::LISTEN_DROPPED.load(Ordering::Relaxed);
    gw.info.time = clock::iso_now();
    unsafe {
        gw.info.free_heap = sys::esp_get_free_heap_size();
        gw.info.min_free_heap = sys::esp_get_minimum_free_heap_size();
        gw.info.reset_reason = match sys::esp_reset_reason() {
            sys::esp_reset_reason_t_ESP_RST_POWERON => "power_on",
            sys::esp_reset_reason_t_ESP_RST_SW => "software",
            sys::esp_reset_reason_t_ESP_RST_PANIC => "panic",
            sys::esp_reset_reason_t_ESP_RST_INT_WDT => "interrupt_watchdog",
            sys::esp_reset_reason_t_ESP_RST_TASK_WDT => "task_watchdog",
            sys::esp_reset_reason_t_ESP_RST_WDT => "watchdog",
            sys::esp_reset_reason_t_ESP_RST_BROWNOUT => "brownout",
            sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP => "deep_sleep",
            sys::esp_reset_reason_t_ESP_RST_EXT => "external",
            _ => "other",
        }
        .into();
        gw.info.last_panic = LAST_PANIC.get().cloned().unwrap_or_default();
        let mut ap: sys::wifi_ap_record_t = core::mem::zeroed();
        gw.info.rssi = (sys::esp_wifi_sta_get_ap_info(&mut ap) == sys::ESP_OK).then_some(ap.rssi as i32);
    }
}

/// Last Rust panic message, kept across the restart in RTC memory (no serial console on the bus).
const PANIC_MAGIC: u32 = 0x5041_4E43;
#[link_section = ".rtc_noinit"]
static mut PANIC_BUF: (u32, [u8; 200]) = (0, [0; 200]);
static LAST_PANIC: std::sync::OnceLock<String> = std::sync::OnceLock::new();
/// Breadcrumb: last step reached, kept across a crash that is not a Rust panic (stack overflow, OOM).
#[link_section = ".rtc_noinit"]
static mut CRUMB_BUF: (u32, [u8; 96]) = (0, [0; 96]);

/// Record the step the firmware is in (cheap, no allocation).
pub fn crumb(step: &str) {
    let heap = unsafe { sys::esp_get_free_heap_size() };
    unsafe {
        let (magic, buf) = &mut *core::ptr::addr_of_mut!(CRUMB_BUF);
        let mut w = Wr { buf, n: 0 };
        let _ = core::fmt::write(&mut w, format_args!("{step} heap={heap}"));
        let n = w.n.min(buf.len() - 1);
        buf[n] = 0;
        *magic = PANIC_MAGIC;
    }
}

struct Wr<'a> {
    buf: &'a mut [u8; 96],
    n: usize,
}

impl core::fmt::Write for Wr<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for b in s.bytes() {
            if self.n < self.buf.len() - 1 {
                self.buf[self.n] = b;
                self.n += 1;
            }
        }
        Ok(())
    }
}

fn previous_crumb() -> String {
    unsafe {
        let (magic, buf) = &mut *core::ptr::addr_of_mut!(CRUMB_BUF);
        let s = if *magic == PANIC_MAGIC {
            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..end]).into_owned()
        } else {
            String::new()
        };
        *magic = 0;
        s
    }
}

fn install_panic_recorder() {
    // read (and clear) what the previous run left
    let prev = unsafe {
        let (magic, buf) = &mut *core::ptr::addr_of_mut!(PANIC_BUF);
        let s = if *magic == PANIC_MAGIC {
            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..end]).into_owned()
        } else {
            String::new()
        };
        *magic = 0;
        s
    };
    let crumb = previous_crumb();
    // the breadcrumb is written all the time: it means something only after a crash, not after a restart
    // asked for (update, button) or a power cut
    let crashed = !matches!(
        unsafe { sys::esp_reset_reason() },
        sys::esp_reset_reason_t_ESP_RST_SW | sys::esp_reset_reason_t_ESP_RST_POWERON | sys::esp_reset_reason_t_ESP_RST_EXT
    );
    let crumb = if crashed { crumb } else { String::new() };
    let prev = match (prev.is_empty(), crumb.is_empty()) {
        (true, true) => String::new(),
        (false, true) => prev,
        (true, false) => format!("no Rust panic; last step: {crumb}"),
        (false, false) => format!("{prev}; last step: {crumb}"),
    };
    let _ = LAST_PANIC.set(prev);
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = format!("{info}");
        unsafe {
            let (magic, buf) = &mut *core::ptr::addr_of_mut!(PANIC_BUF);
            let n = msg.len().min(buf.len() - 1);
            buf[..n].copy_from_slice(&msg.as_bytes()[..n]);
            buf[n] = 0;
            *magic = PANIC_MAGIC;
        }
        default(info);
    }));
}

fn build_defaults() -> Settings {
    let mut s = Settings::default();
    s.wifi_ssid = env!("DEF_WIFI_SSID").into();
    s.wifi_pass = env!("DEF_WIFI_PASS").into();
    s.mqtt_url = env!("DEF_MQTT_URL").into();
    s.mqtt_user = env!("DEF_MQTT_USER").into();
    s.mqtt_pass = env!("DEF_MQTT_PASS").into();
    s
}

fn main() {
    sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    install_panic_recorder();
    // Any error ends in a restart: returning from main would leave the board half-alive.
    if let Err(e) = run() {
        log::error!("fatal: {e}, restarting in 5 s");
        std::thread::sleep(Duration::from_secs(5));
    }
    unsafe { sys::esp_restart() };
}

fn run() -> Result<()> {
    let p = Peripherals::take()?;

    // SAFETY FIRST: keep the RS-485 driver disabled. The driver is leaked so that no drop can ever reset
    // the pin to a floating input; pull-down and hold keep it low. Only the master engine releases it.
    let mut de = PinDriver::output(unsafe { AnyOutputPin::steal(BOARD.de as u8) })?;
    de.set_low()?;
    unsafe {
        sys::gpio_pulldown_en(BOARD.de);
        sys::gpio_hold_en(BOARD.de);
    }
    core::mem::forget(de);
    log::info!("airzone-gateway {VERSION} build {BUILD_UNIX} board {}: DE (GPIO{}) held LOW", BOARD.name, BOARD.de);
    ota::start_guard()?;

    let nvs = EspDefaultNvsPartition::take()?;
    let store = Arc::new(Mutex::new(nvs_store::Store::open(nvs.clone())?));
    let settings = store.lock().unwrap().load(&build_defaults());
    let want_master = cfg!(feature = "master") && settings.mode == Mode::Master;
    log::info!("settings: mode={:?} (master built in: {}) mqtt={} wifi={}", settings.mode, cfg!(feature = "master"), !settings.mqtt_url.is_empty(), !settings.wifi_ssid.is_empty());

    let mac = sta_mac();
    let gateway_id = format!("az{}", mac.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let ap_name = format!("airzone-gw-{:02X}{:02X}", mac[4], mac[5]);
    let info = GatewayInfo {
        version: VERSION.into(),
        build: BUILD_UNIX.into(),
        board: BOARD.name.into(),
        mac: mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":"),
        master_capable: cfg!(feature = "master"),
        ..Default::default()
    };
    let gw = Gateway::new(settings.clone(), Registry::default(), Topology::default(), info, CATALOGUE);
    let shared = Arc::new(Shared {
        listener: Mutex::new(Listener::default()),
        topology: Mutex::new(Topology::default()),
        gw: Mutex::new(gw),
        store: store.clone(),
        master_running: AtomicBool::new(false),
        rediscover: AtomicBool::new(false),
        state_dirty: AtomicBool::new(false),
    });

    let uart_cfg = config::Config::new()
        .baudrate(Hertz(BAUD))
        .data_bits(config::DataBits::DataBits8)
        .parity_even()
        .stop_bits(config::StopBits::STOP1);
    let (to_stream, stream_rx) = sync_channel::<String>(32);

    if want_master {
        #[cfg(feature = "master")]
        master_run::start(p.uart1, &uart_cfg, shared.clone(), to_stream.clone(), &settings)?;
    } else {
        let uart = esp_idf_svc::hal::uart::UartRxDriver::new(
            p.uart1,
            unsafe { AnyIOPin::steal(BOARD.uart_rx as u8) },
            Option::<AnyIOPin>::None,
            Option::<AnyIOPin>::None,
            &uart_cfg,
        )?;
        unsafe {
            sys::esp!(sys::uart_set_rx_timeout(uart.port(), RX_TIMEOUT_SYMBOLS))?;
        }
        let (to_listener, listener_rx) = sync_channel::<Chunk>(256);
        {
            let to_stream = to_stream.clone();
            std::thread::Builder::new().name("uart".into()).stack_size(6 * 1024).spawn(move || uart_rx::run(uart, to_stream, to_listener))?;
        }
        let shared = shared.clone();
        std::thread::Builder::new().name("listen".into()).stack_size(12 * 1024).spawn(move || {
            let mut last = 0i64;
            while let Ok(c) = listener_rx.recv() {
                let gap = if last == 0 { 100_000 } else { (c.t_us - last).max(0) as u64 };
                last = c.t_us;
                if let Ok(mut l) = shared.listener.lock() {
                    l.feed(&c.bytes, gap, (c.t_us / 1000) as u64);
                }
            }
        })?;
    }

    if let Some(pin) = BOARD.button {
        button::watch(pin, store.clone())?;
    }

    let sysloop = EspSystemEventLoop::take()?;
    let mut wifi = BlockingWifi::wrap(EspWifi::new(p.modem, sysloop.clone(), Some(nvs))?, sysloop)?;
    if settings.wifi_ssid.is_empty() {
        portal::run(&mut wifi, store.clone(), settings.clone(), &ap_name)?;
    }
    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: settings.wifi_ssid.as_str().try_into().map_err(|_| anyhow::anyhow!("SSID too long"))?,
        password: settings.wifi_pass.as_str().try_into().map_err(|_| anyhow::anyhow!("password too long"))?,
        auth_method: if settings.wifi_pass.is_empty() { AuthMethod::None } else { AuthMethod::WPA2Personal },
        ..Default::default()
    }))?;
    wifi.start()?;
    unsafe {
        sys::esp!(sys::esp_wifi_set_ps(sys::wifi_ps_type_t_WIFI_PS_NONE))?;
    }
    if !connect_wifi(&mut wifi, Duration::from_secs(600)) {
        portal::run(&mut wifi, store.clone(), settings.clone(), &ap_name)?;
    }
    if let Ok(ip) = wifi.wifi().sta_netif().get_ip_info() {
        shared.gw.lock().unwrap().info.ip = ip.ip.to_string();
    }
    let clock = clock::Clock::start(&settings.timezone)?;

    let mode_word = if want_master { "master" } else { "listen" };
    let banner = format!("airzone-gateway {VERSION} build {BUILD_UNIX} {mode_word} {BAUD} 8E1");
    std::thread::Builder::new().name("stream".into()).stack_size(8 * 1024).spawn(move || stream::server(stream_rx, banner))?;
    let token = env!("DEF_OTA_TOKEN").to_string();
    std::thread::Builder::new().name("ota".into()).stack_size(8 * 1024).spawn(move || ota::server(token))?;
    let ha = HaConfig {
        prefix: settings.mqtt_prefix.clone(),
        discovery: "homeassistant".into(),
        gateway_id: gateway_id.clone(),
        gateway_name: settings.gateway_name.clone(),
        fw: VERSION.into(),
        commands: want_master,
    };
    let _http = http::start(shared.clone(), HaConfig { commands: false, ..ha.clone() })?;
    {
        let (shared, s, ha) = (shared.clone(), settings.clone(), ha.clone());
        std::thread::Builder::new().name("mqtt".into()).stack_size(16 * 1024).spawn(move || {
            if let Err(e) = mqtt::run(shared, s, ha) {
                log::error!("mqtt stopped: {e}");
            }
        })?;
    }
    updater::spawn(shared.clone(), settings.update_manifest.clone())?;
    stream::send(&to_stream, format!("# gateway id={gateway_id} mode={mode_word} mqtt={}\n", if settings.mqtt_url.is_empty() { "disabled" } else { "configured" }));

    let mut time_reported = false;
    let mut tick = 0u32;
    loop {
        std::thread::sleep(Duration::from_secs(10));
        tick += 1;
        if !wifi.is_connected().unwrap_or(false) {
            log::warn!("Wi-Fi lost, reconnecting");
            connect_wifi(&mut wifi, Duration::from_secs(u64::MAX / 4));
        }
        if !time_reported && clock.synced() {
            time_reported = true;
            stream::send(&to_stream, format!("# time {}\n", clock::iso_now()));
        }
        if !shared.master_running.load(Ordering::SeqCst) {
            let (topo, nregs) = {
                let l = shared.listener.lock().unwrap();
                let r = l.registry();
                let centrals = (1..=32u8).filter_map(|a| build_central(a, r)).collect();
                (Topology { centrals }, r.dump(1).len())
            };
            if tick % 6 == 0 {
                let zones: usize = topo.centrals.iter().map(|c| c.zones.len()).sum();
                let iaq = topo.centrals.iter().filter(|c| c.iaq).count();
                stream::send(&to_stream, format!("# decoded centrals={} zones={zones} iaq={iaq} regs={nregs}\n", topo.centrals.len()));
            }
            *shared.topology.lock().unwrap() = topo;
        }
        if tick % 6 == 0 {
            stream::send(&to_stream, format!("# mqtt connected={}\n", mqtt::CONNECTED.load(Ordering::Relaxed)));
            stream::send(
                &to_stream,
                format!(
                    "# ws calls={} connects={} ticks={} sent={} errors={} heap={} min_heap={}\n",
                    http::WS_CALLS.load(Ordering::Relaxed),
                    http::WS_CONNECTS.load(Ordering::Relaxed),
                    http::WS_TICKS.load(Ordering::Relaxed),
                    http::WS_SENT.load(Ordering::Relaxed),
                    http::WS_SEND_ERR.load(Ordering::Relaxed),
                    unsafe { sys::esp_get_free_heap_size() },
                    unsafe { sys::esp_get_minimum_free_heap_size() }
                ),
            );
        }
    }
}

/// Connect, retrying every 5 s until `limit`; false when the limit is reached.
fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>, limit: Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        log::info!("Wi-Fi: connecting");
        match wifi.connect().and_then(|_| wifi.wait_netif_up()) {
            Ok(()) => {
                if let Ok(ip) = wifi.wifi().sta_netif().get_ip_info() {
                    log::info!("Wi-Fi up, IP {}", ip.ip);
                }
                return true;
            }
            Err(e) => {
                log::warn!("Wi-Fi connect failed: {e}, retry in 5 s");
                let _ = wifi.disconnect();
                if start.elapsed() >= limit {
                    return false;
                }
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

pub fn sta_mac() -> [u8; 6] {
    let mut mac = [0u8; 6];
    unsafe {
        sys::esp_read_mac(mac.as_mut_ptr(), sys::esp_mac_type_t_ESP_MAC_WIFI_STA);
    }
    mac
}
