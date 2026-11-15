# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html): MQTT entity ids change only in a major version.

## [0.2.0] - Unreleased

First release with prebuilt firmware: no development tools needed to install the gateway.

### Added

- Firmware for the Waveshare ESP32-S3-RS485-CAN, built by CI on each release, with listen and master mode.
- Web flasher on GitHub Pages (Chrome / Edge): flashes a blank board in one click, and offers to keep or erase
  the settings when reinstalling.
- Release files: bootloader, partition table, OTA data, application image, a merged image for flashing by
  hand at offset 0, and `SHA256SUMS` (bare file names: `sha256sum -c` works next to the downloads).
- Opt-in update check from the gateway page (*Check for updates from the official releases*), once a day;
  changing it takes effect without a restart.
- One-click updates (admin only): from 0.2.0 the gateway page can install a newer release in one click. It
  will be tested for real with 0.2.1; the manual update (`gwctl ota`) is the fallback. A new version that
  does not start rolls back, and the page says so.
- The page and `/api/v1/info` (`update_check`, `update_checked`) show whether the update check is on and the
  result of the last check; a failed check is retried after an hour.
- Getting-started guide in four steps, compatibility table, wiring diagram.

### Changed

- Only one firmware write at a time (HTTP upload, TCP 7001 upload or update install).

### Removed

- Image signing: release images are not signed. Network updates still need the admin password or an API
  token, and releases publish SHA-256 checksums.

[0.2.0]: https://github.com/letherman255/Open-Airzone-Gateway/releases/tag/v0.2.0
