# Development

Requirements: Rust stable (host), Python ≥ 3.11 with [uv](https://docs.astral.sh/uv/).

| What | Command |
|---|---|
| Rust tests (core + simulator) | `cargo test --workspace` |
| Catalogue tests | `uv run --project catalogue pytest catalogue -q` |
| Regenerate tables, JSON, docs | `cd catalogue && uv run python -m generator registers.toml --out ..` |
| Capture excerpt | `python -I tools/sim/extract_capture.py <capture> <from> <to> <out>` |

Layout:

- `catalogue/` — `registers.toml` (source of truth), `generator/`, tests.
- `firmware/core/` — crate `airzone-core`: `frame`, `deframe`, `catalogue` (+ generated table), `model`,
  `registry`, `topology`, `rules`, `poll`, `bus` (trait), `master`, `listen`. `std` only, no clock: time is
  passed in as `now_ms`.
- `tools/sim/` — crate `airzone-sim`: simulated central (`SimCentral`, `SimBus` with fault injection), fixtures
  (excerpts of a real capture, 2026-11-05/06), master-engine and replay tests.
- `web/catalogue.json`, `docs/registers.md` — generated.

## Firmware (ESP32)

Build (PowerShell, toolchain from `espup`). On Windows keep the target directory short: ESP-IDF's CMake
build breaks on long paths.

```powershell
$env:PATH = "$HOME\.cargo\bin;$env:PATH"; . $HOME\export-esp.ps1
$env:CARGO_TARGET_DIR = "C:\azt"   # Windows only; elsewhere the default target/ is fine
cd firmware\esp
$env:AZ_GW_ENV = "<path to your git-ignored env file>"   # optional build-time defaults
cargo build --release
espflash save-image --chip esp32s3 $env:CARGO_TARGET_DIR\xtensa-esp32s3-espidf\release\airzone-gateway $env:CARGO_TARGET_DIR\airzone-gateway.bin
python -I ..\..\tools\ota_upload.py <board ip> $env:CARGO_TARGET_DIR\airzone-gateway.bin --token-file <env file>
```

The uploader confirms the new image with a probe; an image that cannot answer the probe rolls back
after 180 s.

USB flash (first install, or recovery): **disconnect the board from the bus first** (never USB-C and
the bus / 12 V terminal at the same time), then

```powershell
$bl = (Get-ChildItem $env:CARGO_TARGET_DIR -Recurse -Filter bootloader.bin | Where-Object FullName -like '*release*' | Select-Object -First 1).FullName
espflash flash --port COMx --bootloader $bl --partition-table partitions.csv --erase-parts otadata $env:CARGO_TARGET_DIR\xtensa-esp32s3-espidf\release\airzone-gateway
```

Dev server (the page and the API on a simulated central, master mode): `cargo run -p airzone-sim --bin devserver`,
then open http://127.0.0.1:8780. Add `-- --fake-update 9.9.9` to show the update banner (installing is simulated).

Releases: push a tag `vX.Y.Z` equal to the version in `firmware/esp/Cargo.toml`; `.github/workflows/release.yml`
builds, publishes the release and deploys the Pages site. The `github-pages` environment must allow tags `v*`
(one-time setup in [flasher/README.md](flasher/README.md)).
