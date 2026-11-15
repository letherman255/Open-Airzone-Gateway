# Installing the gateway

1. **Flash** the board: web flasher (`docs/flasher/`, Chrome or Edge, USB) or `espflash` (see
   [development.md](development.md)). The board must not be connected to the Airzone bus while it is on USB.
2. **Connect it to the bus** in place of (or, for listen mode, next to) the Airzone Webserver: see
   [hardware/README.md](../hardware/README.md).
3. **Set it up**: on a phone, join the open Wi-Fi `airzone-gw-XXXX`; the setup page opens (or browse to
   `http://192.168.4.1`). Enter your 2.4 GHz Wi-Fi and, for Home Assistant, the MQTT broker URL
   (`mqtt://<broker>:1883`) with its user and password. Save: the gateway restarts on your network.
4. **Open the gateway page** at `http://<gateway IP>` and set the admin password right away.
5. **Home Assistant**: with the MQTT integration configured, the gateway, the central, every zone (climate
   entity) and the IAQ sensor appear by themselves (MQTT discovery). Nothing to install on Home Assistant.
6. **Master mode**: the gateway starts in listen mode (read only). To control the installation, unplug the
   Airzone Webserver, then open Gateway → Bus mode → Switch to master mode (needs a firmware built with
   master mode; release images are).

Reset the network settings: hold the BOOT button 5 s; the gateway restarts in setup mode. Installation
settings are stored in the central and are not affected.

Updates: `POST /api/v1/ota` then `POST /api/v1/ota/confirm` (see [api.md](api.md)) or `tools/ota_upload.py`;
a new image that is not confirmed rolls back after 180 s.

Update notification: set `update_manifest` (Gateway page, Network and MQTT) to the release `manifest.json`
URL; the gateway checks it once a day and shows a newer version on the page and in the Home Assistant
update entity. It never installs anything by itself.
