# Open Airzone Gateway

[![ci](https://github.com/letherman255/open-airzone-gateway/actions/workflows/ci.yml/badge.svg)](https://github.com/letherman255/open-airzone-gateway/actions/workflows/ci.yml)
[![licence: MIT](https://img.shields.io/badge/licence-MIT-blue.svg)](LICENSE)
[![hardware: CERN-OHL-P-2.0](https://img.shields.io/badge/hardware-CERN--OHL--P--2.0-blue.svg)](hardware/LICENSE)

**Local, cloud-free control of Airzone HVAC zoning systems from Home Assistant.**

An ESP32 replaces the Airzone Cloud Webserver on the central's domotic bus. It talks to the Airzone
central directly (Modbus RTU over RS-485), publishes every zone to Home Assistant through MQTT discovery,
and gives you a local web page for the installer settings that the Airzone app hides behind its
installer screens. No cloud account, no internet connection, no custom integration in Home Assistant.

> **Unofficial project, not affiliated with or endorsed by Airzone.** It writes to your heating and
> cooling installation. It is young and has been tested on one installation: read
> [Status](#status) and [Known issues](docs/known-issues.md) first.

## Why

In May 2026 Airzone announced the **Airzone Cloud "Plan Cloud"** subscription, from October 2026: after six
free months, **€48 per year per installation**. Without it, remote control is reduced to **viewing only**
(no remote on/off or setpoint), history is limited to the current day, and routines and scenarios are
subscription features. Airzone states that local control and basic features stay free.

For owners of the **Airzone Cloud Webserver** (for example AZX6WSC5GER) that promise does not help much:
this webserver does not offer Airzone's documented local API (its port 3000 is closed), so every app, voice
assistant and Home Assistant integration goes through the cloud. Airzone's local-API option is a different
product, the Webserver HUB (AZX6WSPHUB), bought separately.

Yet everything the cloud does with your installation ends up as a few frames on a two-wire bus in your
house. This project speaks that bus itself, locally, and hands the result to Home Assistant, so that
controlling your own heating and cooling does not depend on a subscription, an account or a server you do
not own.

## Features

- **Home Assistant over MQTT discovery.** One `climate` entity per zone (on/off, mode, setpoint,
  temperature, humidity), sensors for temperatures, humidity, air quality (IAQ sensor) and errors, switches
  for the relays, everyday settings as configuration entities, and gateway health (memory, restart cause,
  bus errors, update available). Works with any MQTT consumer, not only Home Assistant.
- **Discovers the installation by itself**: centrals, zones, zone names, IAQ sensor, outlets, relays.
  Equipment that is not detected is not created.
- **Local installer page**: system, zone, outlet and IAQ sensor settings (Q-Adapt, setpoint limits,
  offsets, stages, VMC, thresholds…), live values, a change journal with undo, backup and restore of the
  whole configuration. Dangerous settings sit in an *Advanced* section and need a typed confirmation.
- **Safe by construction.** A setting is writable only if its write was confirmed on a real bus; every
  write is checked by reading the register back. The gateway listens before it transmits and refuses to
  become master if it hears another one.
- **Two modes.** *Listen*: receive only, next to the stock webserver, nothing is ever transmitted.
  *Master*: replaces the stock webserver.
- **Documented local API**: HTTP JSON + WebSocket ([docs/api.md](docs/api.md), OpenAPI file), a
  command-line client (`tools/gwctl.py`), raw register access in developer mode, a raw frame stream for
  recordings.
- **Maintenance**: Wi-Fi setup portal, OTA updates with automatic rollback, update notification,
  admin password and API tokens.

Cloud features (schedules, scenes, users, voice assistants) are not reproduced: Home Assistant does them
better.

## How it works

```
 Airzone central(s) ──RS-485 bus── ESP32 gateway ──Wi-Fi── MQTT broker ── Home Assistant
 thermostats, dampers,  19200 8E1   Rust firmware            │
 IAQ sensor behind it   + 12 V      (ESP-IDF)                └── browser: installer page, API
```

The gateway polls the central's register blocks like the stock webserver does (one cycle every ~2.3 s) and
turns them into typed values through a register catalogue (`catalogue/registers.toml`, 98 entries, 65
confirmed by write). Commands from Home Assistant or the page become Modbus writes, and the value is only
reported as applied once the central reads back the new value. Details: [protocol](docs/protocol.md),
[register catalogue](docs/registers.md), [how the registers were found](docs/reverse-engineering.md),
[design](docs/design.md).

## Hardware

| Part | Notes |
|---|---|
| **Waveshare ESP32-S3-RS485-CAN** (reference board) | 7-36 V input, powered by the bus; isolated RS-485. Termination jumper **off** |
| or any ESP32 / ESP32-S3 + RS-485 module | pins chosen at build time |
| **Phoenix Contact MC 1,5/5-ST-3,81** screw plug (Mouser 651-1803604) | plugs onto the central's 5-pin bus header (shield, A, −, B, +) |
| Phoenix Contact IMC 1,5/5-ST-3,81 (Mouser 651-1857919) | optional: keeps the stock webserver plugged in, for listen mode |

No soldering: four wires from the screw plug to the board's terminals. Wiring, electrical rules and pin
maps: [hardware/README.md](hardware/README.md).

**Never power the board from USB and from the bus at the same time**, and never apply more than 12 V.

## Getting started

1. **Build and flash** the firmware over USB, with the board **disconnected from the bus**
   ([docs/development.md](docs/development.md)). Prebuilt images will come with the first release.
2. **Wire it** to the central ([hardware/README.md](hardware/README.md)). Start next to the stock
   webserver in listen mode if you want to compare before switching.
3. **Set it up**: join the `airzone-gw-XXXX` Wi-Fi from a phone, enter your Wi-Fi and MQTT broker, then
   open `http://<gateway IP>` and **set the admin password right away**.
4. **Home Assistant**: with the MQTT integration configured, the gateway, the central and every zone
   appear by themselves.
5. **Take over**: unplug the stock webserver, then *Gateway → Bus mode → Switch to master mode*.

Full guide: [docs/install.md](docs/install.md).

Going back is simple: unplug the gateway and plug the stock webserver in again. Installation settings live
in the central, not in the webserver.

## Status

| | |
|---|---|
| Listen mode | runs on the author's installation since 2026-11-05 |
| Master mode | runs on the author's installation since 2026-11-06 (stock webserver unplugged): discovery, polling, Home Assistant control, every non-dangerous confirmed setting in both directions |
| Tested installation | Easyzone with IAQ sensor, Mitsubishi Electric indoor unit, one central, 6 zones |
| Not verified yet | a 7-day unattended run, other installations, several centrals, zones 17-32, radiant stages, other indoor units, prebuilt / signed release images |

If you have an Airzone installation, a report (even in listen mode only) is the most useful contribution:
use the *new installation* issue template.

## Documentation

| | |
|---|---|
| Users | [Install](docs/install.md) · [Hardware](hardware/README.md) · [Security](docs/security.md) · [Known issues](docs/known-issues.md) |
| Integrators | [API](docs/api.md) · [OpenAPI](docs/openapi.yaml) · [Register catalogue](docs/registers.md) |
| Contributors | [Contributing](CONTRIBUTING.md) · [Development](docs/development.md) · [Bus protocol](docs/protocol.md) · [Reverse engineering notes](docs/reverse-engineering.md) · [Design](docs/design.md) |

## Contributing

Recordings and diagnostics from other installations, newly confirmed registers, board support and PCBs
are all welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md), and in particular the rules for testing a
setting safely: some installer settings (alarm input, R32, filter counters, factory resets) can lock an
installation until it is power-cycled.

## Licence

Software: MIT ([LICENSE](LICENSE)). Hardware: CERN-OHL-P-2.0 ([hardware/LICENSE](hardware/LICENSE)).

Airzone is a trademark of its owner. This project is independent and contains no Airzone code, firmware
or documents; the bus was learned from observation and from Airzone's public Modbus integration manual.
