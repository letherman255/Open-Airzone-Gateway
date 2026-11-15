# Contributing

## Adding or changing a register

1. Edit `catalogue/registers.toml` (one `[[param]]` per setting or measurement).
2. Regenerate: `cd catalogue && uv run python -m generator registers.toml --out ..`
3. Run the tests (`docs/development.md`). The generator refuses: overlapping bits in one register, a `danger`
   entry without `warning` and `confirm`, a tier-3 entry with an `ha` block, a writable entry without
   `evidence` or not `confirmed`.

Confidence levels: `confirmed` (a write was tested and seen on the bus) is the only writable level;
`matched` (the value matches what an official tool shows) is read-only; `hypothesis` is never exposed.

## How to confirm a setting

The protocol used to build the catalogue, in short:

1. One parameter at a time. Note the current value first.
2. Change it with an official tool (app or cloud) while recording the bus (listen mode, or a sniffer).
3. Find the write in the recording, note the frame.
4. Put the original value back at once, and check that the register is back too.
5. Add the entry with `confidence = "confirmed"` and the frame as `evidence`.

**Never test blind:**

- the DI1 input type "acoustic alarm NO" with nothing wired: it raises a latched R32 leak alarm (every zone
  stops, thermostats beep, only a power cycle clears it);
- factory resets and counter resets (they cannot be undone);
- firmware updates, system detection, release / unpair actions.

## New installations

Diagnostics and listen-mode recordings from other installations (other indoor units, radiant floors, more
zones, several centrals, no IAQ sensor) are the most useful contribution. Open an issue with the recording.
