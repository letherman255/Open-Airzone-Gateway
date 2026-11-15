# Open Airzone gateway — design

Written 2026-11-06, before the implementation; kept as the reference for why things are the way they are.
Inputs: the bus findings in [reverse-engineering.md](reverse-engineering.md), the public Airzone AZ6 Modbus
integration manual, and a capture of the stock webserver on the author's installation.

## 1. Goal

An open-source replacement for the Airzone Cloud Webserver: an ESP32 that is the Modbus master of the
Airzone domotic bus, integrates with Home Assistant over MQTT discovery (no HACS, no cloud), and offers an
installer configuration web page that does what the Airzone app's installer section does, with the same
screens, settings and flow. Published on GitHub, usable on other installations than the author's, open to
software and hardware contributions.

Success: on the author's installation, with the stock webserver unplugged, every setting of the stock
installer screens can be read and (when confirmed) changed from the gateway page, every zone works from Home
Assistant, and the gateway runs 7 days unattended. Other installations are discovered without code changes.

Non-goals: cloud features (schedules, scenes, users, sharing, widgets, tariffs; Home Assistant covers them),
running next to the stock webserver as a second master, reusing the stock webserver firmware, a HACS
integration.

## 2. Decisions taken in the design discussion (2026-11-06)

| Topic | Decision |
|---|---|
| Home Assistant link | **MQTT discovery**; the ESP32 holds the register semantics ("autonomous device", like Zigbee2MQTT / ESPHome). Works with any MQTT consumer. |
| Hardware | Reference board Waveshare ESP32-S3-RS485-CAN + generic ESP32 with an RS-485 module; per-board pin files; hardware contributions (PCB) welcome. |
| Release | One release with all three tiers (control, everyday configuration, installer configuration). |
| Installer page | Same functional structure and flow as the stock installer screens; own name, look and texts (no Airzone logos, no "AirTools" name, no screenshots). |
| Dangerous settings | Exposed as in the stock tool, inside a collapsed "Advanced" section, each with an explicit warning and a typed confirmation. |
| Security | Admin password for the page (read-only without it); installer settings never on MQTT. |
| Languages | Everything in English; all UI strings in one table so languages can be added later. |
| Register description | One declarative catalogue (`catalogue/registers.toml`) generates firmware tables, MQTT discovery, web forms, docs and the listen-mode decoder. |
| Firmware stack | Rust, ESP-IDF (std), existing toolchain, Wi-Fi OTA with automatic rollback (already verified). |
| Extra API | Documented HTTP JSON API + WebSocket (live values, registers, frames, logs), API tokens, developer mode for raw access. |
| Optional equipment | IAQ sensor, ionizers, relays, extra centrals, radiant stages: nothing is created unless detected. |
| Unknown commands | A register is writable only if its catalogue entry is `confirmed` (tested, evidence recorded). |

## 3. Architecture

```
Central(s) ──RS-485── ESP32 firmware (Rust, ESP-IDF) ──Wi-Fi── MQTT broker ── Home Assistant
addresses 1-32         bus · registry · model · rules        └─ browser (config page, API, WebSocket)
                       mqtt · web/api · system
```

| Component | Role | Depends on |
|---|---|---|
| `bus` | Modbus RTU frames (03, 06, 16 mask write with Airzone's non-standard reply, 08), CRC, timing, DE pin; **master** and **listen** modes | board file |
| `registry` | last value of every register, change flags, timestamps | — |
| `model` | registers ↔ typed values through the generated catalogue | registry, catalogue |
| `rules` | hand-written behaviour the catalogue cannot express (discovery, global mode, power-before-mode, per-mode setpoints, fan forces 15 °C, heating_available side effect, presence detection) | model |
| `mqtt` | HA discovery, state, commands (tiers 1-2 only) | model |
| `web` / `api` | config page, HTTP API, WebSocket, traffic recording, diagnostics | model |
| `system` | provisioning, Wi-Fi, password, tokens, OTA, settings in NVS | — |

`firmware/core` (bus protocol, registry, model, rules) has no ESP32 dependency and is built and tested on the
PC against the central simulator. `firmware/esp` holds the drivers, network, MQTT, web and OTA.

## 4. Bus engine

Master start-up:
1. Listen 5 s. Any other master's traffic → refuse to transmit, switch to listen mode, show it in red.
2. Discover centrals: read `0x0030` on addresses 1-32.
3. Per central: zones present (`0x0009` / `0x0010`), zone names (+14…+19), configuration (+4), outlets.
4. Identity, mimicking the stock sequence with our values: configurable 10-character name (default
   `OPENAZWS01`), real MAC / IP, firmware version, connection state (`0x008B`). Heartbeat (`0x0001` bit 4,
   ~45 s) and periodic identification (`08`, 60 s) on by default, each switchable in Advanced, so the first
   master test shows what the central requires.

Steady state: poll the same blocks as the stock webserver (system 0x0000/0x0050 ×80, IAQ 0x2100 ×33, outlets
0x2900-0x3000 ×12, zones ×65, F7/F8), only blocks that exist, cycle 2.3 s by default. Writes jump the queue;
the affected block is re-read at once and only the re-read value is published. Re-discovery every 10 min,
when a zone or central stops answering, and on demand.

Errors: timeout / bad CRC → 2 retries; 3 failed cycles → central unavailable. Modbus exception → write
rejected, logged, reported. A write is never retried blindly; the re-read decides.

Listen mode: never transmits (DE held low in code), decodes the stock webserver's traffic, publishes
read-only values, can record traffic for contributors.

## 5. Catalogue

One `[[param]]` entry per setting or measurement: `id`, `scope` (system, zone, iaq, outlet, relay), `reg`,
`bits`, `scale`, `unit`, `values`, `range` (fixed or from another register), `access` (r / rw), `write`
(06 / 16), `tier` (1 control, 2 everyday config, 3 installer), `confidence` (confirmed / matched /
hypothesis), `requires` (presence condition), `danger` (`level`, `warning`, `confirm`), `ha` (component,
role), `ui` (screen, section, order, label), `evidence` (test that proved it).

Exposure by confidence: `confirmed` → writable; `matched` → read-only; `hypothesis` → diagnostics only.

Generator (Python, run at build): `firmware/core/src/catalogue_gen.rs`, `web/catalogue.json`,
`docs/registers.md`, listen-mode decoder tables. Build fails on: overlapping bits in one register, `danger`
without warning and confirmation, a tier-3 entry with an `ha` block, a writable entry without evidence,
non-deterministic output.

Seed: every confirmed / matched entry of [reverse-engineering.md](reverse-engineering.md).

## 6. Home Assistant over MQTT

Devices (linked with `via_device`): gateway; one per central; one per zone (named from the central);
IAQ sensor if present.

| HA entity | Examples | Tier |
|---|---|---|
| `climate` | per zone: on/off, mode, setpoint, current temperature, humidity, action | 1 |
| `sensor` / `binary_sensor` | IAQ, machine temperatures, demand, errors, warnings, firmware | 1 |
| `switch` | relays O1 / O2 | 1 |
| `select` / `number` / `switch` (entity_category config) | Q-Adapt, limits, offsets, IAQ thresholds / weights, VMC, antifreeze, basic mode | 2 |
| gateway `update`, `button` (rediscover, restart), diagnostics sensors | | — |

Global mode: a mode change from any zone is written to the master zone; every zone follows in HA. Zone off
uses the zone power bit only.

Topics (prefix configurable, default `airzone`): `<gw>/availability` (LWT), `<gw>/<central>/zone/<n>/state`
(one JSON per device, on change + full refresh every 5 min), `<gw>/<central>/zone/<n>/set/<field>`,
`homeassistant/<component>/<unique_id>/config` (retained). Unique ids from gateway MAC + central address +
zone number; "take over identity" for a replacement ESP32. Rejected command → state stays at the real value,
error in a "last error" sensor and in the page log. A vanished zone's discovery is removed.

## 7. Web page, provisioning, security

Provisioning: AP `airzone-gw-XXXX` + captive portal → Wi-Fi, MQTT broker, admin password, mode (listen by
default). AP returns after 10 min without Wi-Fi. Long button press resets network settings only.

Screens (stock installer structure): Gateway; System N (Information, Configuration: System, Stages, Air
flow, Inputs/Outputs); Zones (list, zone page: thermostat, firmware, offsets, cool/heat outlets, master
zone, basic mode, antifreeze); IAQ sensor if present (VMC, thresholds, weights, defaults). Live values; a
change shows "applied" only after the bus re-read; every installer change journaled (old, new, time) with
undo; backup / restore of the whole installation configuration to a file. Dangerous settings in a collapsed
Advanced section with warning + typed confirmation.

Security: read-only without password; session with timeout, lockout after 5 failures; password hashed in NVS;
passwords never shown nor in diagnostics; OTA needs the password; release images are signed and unsigned
images are refused; HTTP on the LAN (documented).

## 8. Local API

HTTP JSON + WebSocket; the page uses only this API. Read without token; writes need an API token (created
with the admin password, several, revocable); raw access needs developer mode (off by default, journaled,
orange banner in the page and a sensor in HA).

| Group | Endpoints | Access |
|---|---|---|
| State | `GET /api/v1/info`, `/systems`, `/systems/{c}/zones/{n}` | read |
| Catalogue | `GET /api/v1/catalogue`, `GET /api/v1/params/{id}?system=&zone=` | read |
| Commands | `POST /api/v1/params/{id}` → answers after re-read (`applied` or error) | token |
| Config | journal, undo, backup, restore | token |
| System | restart, rediscover, listen/master, OTA | token |
| Debug | `GET /api/v1/raw/{addr}/{reg}?count=`, `POST /api/v1/raw/write` (06 / 16), `GET /api/v1/registry`, traffic recording, log level | token + developer mode |

WebSocket `/api/v1/ws`, topics `values`, `registers`, `frames`, `log`, `events`. A raw write that touches the
bits of a `danger` entry is refused without that entry's confirmation. OpenAPI file generated, `docs/api.md`.
CLI `tools/gwctl` (get, set, watch, raw, ota) for development.

## 9. Repository, boards, releases, contributions

```
catalogue/   registers.toml + generator/ + tests
firmware/    core/ (host-testable)  esp/  boards/
web/         page (small HTML/JS, embedded at build)
tools/       central simulator, listen decoder, recorder, gwctl
hardware/    compatible boards, wiring, contributed PCBs
docs/        install, registers (generated), protocol, API, security, contributing
```

Boards: `waveshare-esp32s3-rs485`, `generic-esp32-max485`; one image per board in each release. Hardware
guide: 12 V max, consumption, RS-485 isolation, termination, 5-pin connector pinout.
Install: web flasher (ESP Web Tools) first, then OTA from GitHub releases (offered, never automatic), rollback.
CI: catalogue + generator tests, core tests against the simulator, build per board, signed release images.
SemVer; MQTT ids change only in a major version.
Contributions: issue templates (new installation with diagnostics + recording, register found, board);
"how to confirm a setting" guide (one parameter at a time, restore, evidence) with the never-test-blind list.
Licences: MIT (software), CERN-OHL-P (hardware).

## 10. Testing and release criteria

Without hardware: catalogue / generator tests; core unit tests (frames, CRC, 0x16 reply, masks, every
catalogue conversion); **central simulator** built from the capture (values, central copies, global mode,
fan 15 °C, heating_available effect, refusals) with variants (1 zone, 8 zones, two centrals, no IAQ, zones
without ionizer) and faults (silent central, bad CRC, late reply, second master); capture replay through the
listen decoder must give the values and commands recorded on 2026-11-05; MQTT and page against a local
Mosquitto + simulator, page in an automated browser.

On the installation: listen mode next to the stock webserver, then master mode with the stock webserver
unplugged and the author present (30 min read-only, then every confirmed write with restore); 7 days
unattended. Progress: [known-issues.md](known-issues.md).

Release 1.0 when all three tiers are done, all tests above pass, no writable entry is below `confirmed`
(untested ones ship read-only), install docs are ready, and the README states the limits (unofficial, tested
on one installation: Easyzone IAQ + Mitsubishi, 6 zones, one central).

## 11. Open items

Current list: [known-issues.md](known-issues.md). Still open from the design:

- Ionizer presence indicator per zone (zone +57 bits).
- IAQ sensor presence rule (0x2100 firmware ≠ 0 / 0xFFFF, or block not answering): confirm on an installation
  without a sensor.
- Renew-air days / free-cooling months numbering (0 = Monday / January assumed).
- Installation type and central address registers; eco values manual / a_p / a_pp (1 / 3 / 4 assumed).
- Meaning of the start-up writes 0x004A / 0x004B / 0x0052 / 0x0074 / 0x0075 (the gateway does not send them and master mode works so far).
