# Known issues and open points

## Verified on a real installation (2026-11-06, Waveshare board, 1 central, 6 zones, IAQ sensor)

- Master mode with the stock webserver unplugged: discovery, polling, identity writes, every non-dangerous
  `confirmed` write in both directions (50+ round trips), HTTP API, WebSocket, MQTT discovery and commands from
  Home Assistant, OTA. Lessons: probe with the system-block read, 300 ms reply timeout, 20 ms turnaround, re-read
  after a lost echo, and keep the heap in mind (~80 KB free with MQTT; see `/api/v1/info`).
- Writes for switched-off equipment (continuous ventilation with the VMC off) are acknowledged and sometimes
  applied: the API refuses them (switching off stays allowed).

## Needs hardware (not verified yet)

- 7 days unattended in master mode.
- ESP-only code paths verified by build only: setup portal, button, update check (HTTPS certificate bundle,
  stack size), signed release images.
- Thread stacks (~100 KB in all) were never measured; they could be trimmed to win heap.
- DE pull-down during reset / bootloader on the Waveshare board (hardware question).
- Presence rules to confirm on other installations: ionizer per zone, missing IAQ sensor, radiant stages,
  several centrals, zones 17-32.

## Deferred minor issues (from the code reviews)

API and security
- Any API token has full admin power (no scopes); the master-mode switch confirmation is client-side only.
- The stored Wi-Fi password survives an SSID change; a password change keeps API tokens.
- Global login lockout resets after 60 s (no backoff).
- Two dangerous settings in one register with different confirmation words cannot be confirmed together by a
  raw write.
- `/log` needs a token but not developer mode; `gwctl login` takes the password as an argument.
- Update manifests may be plain http.

MQTT / Home Assistant
- Binary sensors render an unknown value as OFF instead of unknown.
- Command topics stay advertised after a refused master start (commands are dropped safely).
- Update entity without `device_class` / `release_url`.

WebSocket
- The `registers` topic compares capture timestamps: a change fed late can be missed.
- The `events` topic lacks discovery, error and MQTT connection events; the spec's `log` topic is not
  implemented.

Firmware
- Master fallback listener has no UART RX timeout (frames rely on the deframer only).
- Setup portal DNS catch-all keeps EDNS additional bytes and answers responses; it shows the MQTT URL.
- `ws.rs` length cast assumes 64-bit (host helper only).

Tools and tests
- Restore compares values as JSON text (`21` vs `21.0` causes a harmless extra write).
- No multi-iteration PBKDF2 test vector; `gwctl` error test has no 409 case.
- The release web-flasher manifest flashes only the app (no bootloader / partition table): blank boards need
  `espflash` the first time.
