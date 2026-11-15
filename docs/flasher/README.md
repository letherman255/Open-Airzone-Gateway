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
The script writes the site: `index.html`, `manifest.json` (ESP Web Tools), `ota.json`,
`firmware/<version>/*.bin` and `firmware/<version>/SHA256SUMS` (bare file names, so `sha256sum -c` works
there and next to the flat release downloads). The workflow deploys that folder to GitHub Pages.

One-time repository setup, or the `pages` job fails:

- Settings → Pages → Source = **GitHub Actions**;
- Settings → Environments → `github-pages` → Deployment branches and tags → add a rule for tags `v*`
  (by default the environment only accepts the default branch, and releases deploy from a tag).

To bump ESP Web Tools, change the version in the `<script>` tag (use a release at least two weeks old)
and check the slot names (`activate`, `unsupported`, `not-allowed`) still exist.
