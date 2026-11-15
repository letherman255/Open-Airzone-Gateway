# Web flasher

`index.html` is the first-install page, published at https://letherman255.github.io/Open-Airzone-Gateway/.
It uses [ESP Web Tools](https://esphome.github.io/esp-web-tools/) (pinned exact version, loaded from unpkg)
to flash a blank Waveshare ESP32-S3-RS485-CAN over Web Serial from Chrome or Edge.

The page lists what you need, warns to unplug the board from the Airzone bus before connecting USB,
shows the install button (with fallbacks for unsupported browsers and non-HTTPS pages), and explains
the first boot: join `airzone-gw-XXXX`, enter Wi-Fi and MQTT, set the admin password; the gateway starts
in listen mode and never transmits until switched to master mode.

## How it is published

The file is a template. `{{VERSION}}` appears in it and is replaced with the release version by
`tools/build_site.py`, which the release workflow (`.github/workflows/release.yml`) runs on each tag.
The script writes the site: `index.html`, `manifest.json` (ESP Web Tools), `ota.json`, `SHA256SUMS`
and `firmware/<version>/*.bin`. The workflow deploys that folder to GitHub Pages.

To bump ESP Web Tools, change the version in the `<script>` tag (use a release at least two weeks old)
and check the slot names (`activate`, `unsupported`, `not-allowed`) still exist.
