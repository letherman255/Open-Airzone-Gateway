# Hardware

## Supported boards

| Board | Build | Notes |
|---|---|---|
| Waveshare ESP32-S3-RS485-CAN | `AZ_BOARD=waveshare-esp32s3-rs485` (default) | 7-36 V input, isolated RS-485, termination off. RX 18, TX 17, DE 21, BOOT button 0 |
| Any ESP32 / ESP32-S3 + RS-485 module (MAX485, MAX3485…) | `AZ_BOARD=generic-esp32-max485 AZ_PIN_RX=… AZ_PIN_TX=… AZ_PIN_DE=… [AZ_PIN_BUTTON=…]` | DE and /RE tied together on `AZ_PIN_DE`; pull DE low with a resistor (it floats during reset) |

## Connecting to the Airzone bus

The gateway takes the place of the Airzone Webserver on the central's domotic bus: Modbus RTU over RS-485,
19200 baud 8E1. The bus connector carries A, B, 12 V and ground (and the shield on the 5-pin version).

### Connector and wiring

The stock webserver plugs directly onto the central's domotic-bus header: 5 pins, **3.81 mm pitch**
(Phoenix Contact MINI COMBICON size), in this order: **shield, A, −, B, +** (+ = 12 V, − = ground). A
standard 5-position 3.81 mm screw plug fits the central's header, so no soldering is needed.

| Part | Phoenix Contact | Mouser | Use |
|---|---|---|---|
| Screw plug, female, 5 pos., 3.81 mm | MC 1,5/5-ST-3,81 (1803604) | 651-1803604 | plugs onto the central's header |
| Screw plug, male (inverted), 5 pos., 3.81 mm | IMC 1,5/5-ST-3,81 (1857919) | 651-1857919 | only to keep the stock webserver stacked on the bus (listen mode) |
| Waveshare ESP32-S3-RS485-CAN | — | — | the gateway |
| Stranded wire 0.25-0.5 mm², 4 colours | — | — | |

Master mode (the gateway replaces the webserver): one MC plug on the central's header, four wires to the
Waveshare terminals.

```
central header ── MC 1,5/5-ST-3,81
                     + ─────── Waveshare power terminal +  (7-36 V)
                     − ─────── Waveshare power terminal −  (and RS-485 GND if your board has one)
                     A ─────── Waveshare RS-485 A
                     B ─────── Waveshare RS-485 B
                     shield ── not connected
```

Listen mode (next to the stock webserver): MC plug on the central, five wires 1:1 to an IMC plug that the
webserver plugs into, and the four wires above taken from the same screw terminals (they take two thin wires
each, up to 1.5 mm²). Check that the webserver's connector mates with the IMC plug before wiring.

If A and B are swapped nothing is damaged, but no valid frame is received: swap them.

Electrical rules:

- Never more than **12 V** on the board side; the bus supplies 12 V DC, about 200 mA for the gateway.
- **Never power the board from USB and from the bus at the same time.** Unplug it from the bus before connecting USB.
- Keep the RS-485 termination **off** (the central end is terminated).
- Isolated RS-485 is recommended; otherwise share the bus ground.
- DE must be low (receive) whenever the firmware is not transmitting, including during reset: add a pull-down
  if your board has none.
- One master per bus: unplug the Airzone Webserver before switching the gateway to master mode (the firmware
  refuses to start as master when it hears another one).

## Contributing a PCB

PCBs are welcome under `hardware/<name>/` (sources, BOM, Gerbers, a photo, the pin map), licensed
CERN-OHL-P-2.0 (`hardware/LICENSE`). Follow the rules above and state the input voltage range, the
consumption and the isolation.
