# Reverse engineering notes: how the registers were found

This is the evidence behind `catalogue/registers.toml` (the `evidence` field of each entry points here).
The catalogue and [registers.md](registers.md) are the reference; this page explains how each value was
established and what is still unknown. Everything below was observed on **one installation**: Easyzone with
IAQ sensor, Mitsubishi Electric indoor unit, one central at address 1, 6 zones, stock Airzone Cloud
Webserver (firmware 4.23), November 2026.

> **Safety first.** One test latched a refrigerant (R32) leak alarm on the whole installation (every zone
> switched off, buzzers on every thermostat, only a power cycle of the central cleared it): DI1 input
> configuration = 3 ("acoustic alarm NA") with nothing wired. Never test the DI1 input, alarm, R32, filter
> counters, factory resets or firmware settings blind. See [contributing.md](contributing.md).

## Sources and method

- **Official reference**: Airzone *AZ6 Modbus integration manual* (MI_AZ6_MODBUS_A4_MUL, on
  doc.airzone.es) for the central's domotic bus: same bus (19200 8E1), same addressing (zone n = n × 256 +
  offset, zone 127 = write broadcast), function 22 (0x16) = mask write. It covers control and part of the
  configuration. It does **not** cover the installer settings (outputs, Q-Adapt, offsets, IAQ sensor, VMC,
  filter, inputs), nor the webserver's own traffic.
- **Passive capture**: an ESP32 with an isolated RS-485 transceiver, receive only, stacked on the bus next to
  the stock webserver, streaming every frame with a timestamp. Excerpts are in `tools/sim/fixtures/`.
- **Active tests**: change **one** parameter at a time from Home Assistant (stock cloud integration), the
  Airzone app installer screens, or the Airzone Cloud web app's device API (same parameter model as the
  public Airzone Cloud Open API); note the frame that appears on the bus 1-2 s later; restore the original
  value at once and check it read back identical.
- Confidence levels used in the catalogue:
  - **confirmed**: a commanded change produced exactly this write, and the restore produced the inverse;
  - **matched**: the value in the register matches what the app / API shows, but it was never written;
  - **hypothesis**: behaviour observed, meaning guessed.

Only `confirmed` entries are writable in the gateway.

## Frames

- Functions 0x03 (read), 0x06 (write one register), 0x16 (mask write), 0x08 (identification).
- The central's reply to 0x16 is **not** the standard echo: it answers `<addr> 16 <reg> 00 00 <and mask>`
  (for example request `01 16 0200 FFFB 0004`, reply `01 16 02 00 00 00 FF FB`).
- Mask write semantics: `new = (old AND and) OR (or AND NOT and)`.

## Zones

Zone n block starts at n × 0x100; the webserver reads 65 registers per zone (`01 03 0n00 0041`).

| Offset | Meaning | How it was established |
|---|---|---|
| +0 bit 2 | zone on/off | confirmed: `01 16 0n00 FFFB 0000` / `… 0004` |
| +0 bits 4-5 | fan speed | manual |
| +0 bits 6-7 | sleep, × 30 min | confirmed: `01 16 0n00 FF3F 0040` |
| +0 bits 8-11 | mode: 0 stop, 3 fan, 5 heat, 6 dry, **9 cool** | confirmed for every mode. Cool is 9 here, not 1 as the manual suggests. The mode is global: written on the master zone, the central copies it to system 0x0000 and every zone |
| +0 bit 15 | basic mode | confirmed |
| +1 / +2 | setpoint limits, × 0.1 °C | copies of system 0x0019 / 0x001A |
| +3 | setpoint, × 0.1 °C | confirmed: `01 06 0n03 00D7` = 21.5 °C. One setpoint per mode is kept by the central and swapped into +3 on a mode change. Fan mode forces 15.0 °C, the previous value comes back with heat |
| +4 bit 0 | master zone | confirmed (re-write on the current master; the cloud refuses to move it) |
| +4 bit 1 | heat stage: air | matched (the mask of the heat-stage write also clears bit 9, probably the radiant stage) |
| +4 bit 4 | antifreeze | confirmed |
| +8 / +10 | probe / thermostat temperature, × 0.1 °C | manual, matches the app |
| +9 bit 7 / bit 5 | air demand / radiant demand | manual |
| +12 | thermostat firmware (0x0369 = 3.69) | matched |
| +13, +30 | errors, warnings | manual |
| +14…+19 | zone name, 12 ASCII bytes, 2 per register, zero padded | confirmed: six 0x06 writes |
| +22 bits 5-6 | cool stage (1 air, 0 off) | confirmed |
| +31 | humidity, % | manual |
| +32 | thermostat offsets: low nibble cool, high nibble heat, offset = (n − 5) × 0.5 °C | confirmed (app) |
| +34…+41, +44/+45 | per-usage-mode and dual setpoints | manual; +44/+45 follow +3 |
| +57 bits 0-1 | ionizer mode: 0 off, 1 on, 2 auto | confirmed |
| +57 bits 2-3 / bit 4 | air quality level (1 good, 2 regular) / ionizer running | live values, hypothesis |
| +59 / +60 | copies of the IAQ PM2.5 / PM10 values | live values |

## System block (0x0000-0x009F)

| Register | Meaning | How |
|---|---|---|
| 0x0000 | machine mode copy | central copies the master zone mode |
| 0x0001 bit 4 | webserver heartbeat: the webserver sets it every ~45 s, the central clears it | passive |
| 0x0003 bits 2-4 | eco (0 off, 2 "a"; 1 / 3 / 4 assumed for manual / a_p / a_pp) | confirmed for 0 and 2, master zone only |
| 0x0003 bit 13 | proportional opening | confirmed (app) |
| 0x0004 | clock, +1 per minute; bit 15 = automatic DST | DST confirmed (app) |
| 0x0007 bit 0 | centralized controller | confirmed (API) |
| 0x0009 / 0x000A | zones present, bit n-1 (zones 1-16 / 17-32) | manual |
| 0x000C bit 10 | Easyzone mode (the central also flips 0x0073 bit 15) | confirmed |
| 0x000C bit 15 / 0x000E bit 13 | relay O1 / O2 closed (relay mode) | confirmed |
| 0x0011 | central firmware (bit 15 flag; 0x8377 = 3.77) | matched |
| 0x0019 / 0x001A | min cool / max heat setpoint, × 0.1 °C, copied to every zone +1 / +2 | confirmed |
| 0x0038 bit 13 | CAI priority | confirmed |
| 0x003C bit 3 | radio channel | confirmed |
| 0x003C bit 8 | reset filter counter (one-shot command) | confirmed |
| 0x003C bit 9 | filter maintenance warning enabled | confirmed |
| 0x0048 bits 0-2 | Q-Adapt: 0 standard, 1 power, 2 silent, 3 minimum, 4 maximum | confirmed (app) |
| 0x0048 bits 9-11 | **DI1 input**: 0 disabled, 1 bridge alarm, 2 acoustic alarm NC, 3 acoustic alarm NA | confirmed. **Value 3 with nothing wired latched an R32 alarm.** 1 and 2 were harmless for 12 s with nothing wired |
| 0x0055 | modes available (bit mask) | matched |
| 0x005F | system working temperature | matched (hypothesis) |
| 0x006E | real setpoint: follows the highest zone setpoint | matched |
| 0x0073 bit 0 / 1 | standby in cool / heat | confirmed (app) |
| 0x009B | filter interval, hours | confirmed |

Relay output configuration (O1 / O2: relay, ACS, VMC, demand) is spread over 0x0024 bits 13-14, 0x000C bit 14,
0x001D bit 9 and 0x000E bit 13; the combinations are in the catalogue. "Heating available = false" has no bit
of its own: the webserver only rewrites 0x001A, and the central then put the machine and every zone in stop.

## IAQ sensor block (0x2100-0x2121)

| Register | Meaning | How |
|---|---|---|
| 0x2100 | sensor firmware (0x0105 = 1.0.5); absent when 0 or 0xFFFF | matched |
| 0x2101 bits 2-3 | ventilation mode: 1 on, 2 auto, 0 off | confirmed |
| 0x2101 bit 6 | VMC enabled (0x2103 low byte then shows the VMC running) | confirmed |
| 0x2101 bit 7 / 8 / 12 / 14 | continuous ventilation / sensor LEDs / air renewal / free cooling | confirmed |
| 0x2101 bit 9 | sensor factory reset (one-shot; turns the VMC on) | confirmed |
| 0x2101 bit 13 / 15 | **air renewal allowed today / free cooling allowed this month** | confirmed, see below |
| 0x2102 bits 0-6 / 7-13 | VMC min / max voltage, × 0.1 V | confirmed |
| 0x2105-0x210B | live PM2.5, PM10, CO₂ ppm, TVOC ppb, RH %, T × 0.1 °C, pressure hPa | matched with the API |
| 0x210C-0x210F low | weights, %: RH, CO₂, TVOC, PM10, PM2.5, T (one byte each) | confirmed (app) |
| 0x210F bits 8-11 / 12-15 | heat / cool temperature band, °C | confirmed |
| 0x2110 / 0x2111 | RH thresholds low side / high side (one byte each, %) | confirmed |
| 0x2112 | CO₂ thresholds, × 50 ppm | confirmed |
| 0x2113 / 0x2114 | PM2.5 thresholds, µg/m³ | confirmed |
| 0x2115 / 0x2116 | PM10 / TVOC thresholds, × 10 | confirmed |
| 0x2117-0x211D | IAQ score and sub-scores (value / 8 = the API score) | matched |
| 0x2119 bit 11 | humidity control | confirmed |
| 0x211F | air renewal interval (low byte) / work time (high byte), minutes | confirmed |
| 0x2120 | free cooling min temperature (low byte) / difference (high byte), °C | confirmed |
| 0x2121 bit 0 | sensor night mode (outside the 33 polled registers) | confirmed |

**Calendar duty.** Air renewal days and free-cooling months are **not stored in the central**. The webserver
evaluates them itself and writes 0x2101 bit 13 (renewal allowed today) and bit 15 (free cooling allowed this
month) after each change and about every 4.5 min. A replacement master must store both lists and drive these
bits (the gateway does, with SNTP time). Numbering 0 = Monday / 0 = January is a hypothesis.

## Outlets (0x2900-0x3000)

Outlet n: 12 registers from 0x2900 + 0x100 × (n − 1).

| Word | Meaning | How |
|---|---|---|
| +0 | high byte = outlet number, low byte = linked zone (0 = none) | confirmed |
| +1 bit 4 / 5 / 6 | heat stage / cool stage / stage setup fancoil (0 = DX) | confirmed |
| +6 | warnings (bit 2 = outlet disconnected, latched) | observed after linking an outlet without damper |
| +11 | counter, +1 about every minute | live value |

## What the stock webserver does on the bus

Start-up (after a power cycle):

1. Address scan `NN 03 0030 0001` for NN = 214-245 (no answer).
2. Broadcast identification (function 0x08 to address 0).
3. Presence read of 0x0030 on central 1, a full poll, `F7` / `F8 03 0030 0001`.
4. Identification `01 08 0003 0012 …` with its 10-character name, then its identity with 0x06 writes: MAC
   0x004E-0x0050, IP 0x002E/0x002F, firmware 0x0059, network gateway 0x004C/0x004D, and 0x0052 (unknown).
5. Connection state 0x008B bits 8-10: 1 then 5 (hypothesis: network up, then cloud connected).
6. Broadcasts to 0x0045-0x0047 (they look like "configuration changed, re-read"); they also follow
   configuration changes.
7. Writes to 0x004A, 0x004B, 0x0074, 0x0075, sent twice; meaning unknown (not the clock).

Steady state: one poll cycle every 2.29 s (`0000 ×80`, `0050 ×80`, `2100 ×33`, outlets `2900…3000 ×12`, zones
`×65`, `F7` / `F8 0030`); heartbeat about every 45 s; identification every 60 s; IAQ calendar bits about every
4.5 min. Opening screens in the app adds no reads: every value the installer screens show is in the polled
blocks.

## Cloud-only features

Schedules, scenes, eco / usage mode actions, users, sharing, widgets and voice assistants are cloud objects.
Creating, enabling and deleting a schedule produced **no bus traffic**; scenes and installation-wide actions
are expanded by the cloud into the per-zone writes above. Home Assistant covers them.

## Not mapped yet

Auto mode (not available on this installation), fan speed writes, eco values other than off / "a",
installation type and central address, the meaning of the start-up writes listed above, and the presence
rules for ionizers, radiant stages, a missing IAQ sensor and several centrals. Reports from other
installations are the way forward: see [contributing.md](contributing.md).
