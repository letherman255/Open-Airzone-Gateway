# The Airzone domotic bus

What the gateway implements, as observed on a real installation (2026-11-05/06) and cross-checked with the
public Airzone AZ6 Modbus integration manual. Register meanings live in the catalogue
(`catalogue/registers.toml`, rendered in `docs/registers.md`).

## Physical layer and roles

- RS-485, 2 wires (A, B) plus 12 V and ground on the same connector; the gateway is powered by the bus.
- Modbus RTU, 19200 baud, 8 data bits, even parity, 1 stop bit (8E1). A frame ends after ≥ 1.75 ms of silence.
- One master (the gateway) and one or more centrals (slaves) at addresses 1-32. Thermostats, dampers and the
  IAQ sensor sit behind the central; the gateway only talks to centrals.
- Two masters on the same bus corrupt each other's frames: the gateway listens 5 s before transmitting and
  stays in listen mode if it hears anyone.

## Frames

CRC-16/Modbus (polynomial 0xA001 reflected, initial value 0xFFFF), low byte first.

| Function | Use | Note |
|---|---|---|
| 0x03 | read N registers | standard |
| 0x06 | write one register | reply is the echo |
| 0x16 | mask write: `new = (old AND and) OR (or AND NOT and)` | reply is **not** the standard echo: `<addr> 16 <reg> 00 00 <and>` |
| 0x08 | gateway identification | `0003 0012` + 8 bytes + 10-character name; the central echoes it |

## Register layout

- System block 0x0000-0x00CF (read as 0x0000 ×80 and 0x0050 ×80).
- Zone n (1-32): 65 registers from n × 0x100. The zone mode (+0 bits 8-11) is global: write it on the master
  zone (+4 bit 0) and the central copies it to every zone. Codes seen: 0 stop, 3 fan, 5 heat, 6 dry, 9 cool.
- Zones present: bit n-1 of 0x0009 (zones 1-16), bit n-17 of 0x000A (zones 17-32).
- IAQ sensor 0x2100-0x2120 (read ×33); present when 0x2100 (firmware) is neither 0 nor 0xFFFF.
- Outlets 1-8: 12 registers from 0x2900 + 0x100 × (n − 1).

## What the stock webserver does

- Start-up: address scan (0x0030 on 214-245, no answer), broadcast identification, presence read of 0x0030 on
  central 1, a full poll, identification (08), its identity writes (MAC 0x004E-0x0050, IP 0x002E/0x002F,
  firmware 0x0059, network gateway 0x004C/0x004D), connection state 0x008B bits 8-10 (1 then 5), broadcasts to
  0x0045-0x0047 and a few start-up writes whose meaning is unknown (0x004A, 0x004B, 0x0052, 0x0074, 0x0075).
- Steady state: one poll cycle every 2.29 s; heartbeat (0x0001 bit 4 set, the central clears it) about every
  45 s; identification every 60 s.
- **VMC calendar:** renewal days and free-cooling months are not stored in the central. The webserver
  evaluates them and writes 0x2101 bit 13 (renewal allowed today) and bit 15 (free cooling allowed this month)
  about every 4.5 min and after each change. The gateway must do the same.
- Schedules, scenes, users and the rest of the cloud features never appear on the bus.

## Open points

- Whether a central actually requires the heartbeat, the identification and the identity writes: the gateway
  sends them (each is switchable) and master mode works with them; nobody has tried without yet.
- How to detect an ionizer per zone, a missing IAQ sensor, radiant stages, other indoor units.
