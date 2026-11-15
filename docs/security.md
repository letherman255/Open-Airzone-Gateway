# Security

- The page and the API are served over plain **HTTP on your local network**, like most local devices
  (ESPHome, Shelly). Keep the gateway on a network you trust, or on an isolated IoT network.
- Without login the page and the API are **read only**. Changes need the admin password (PBKDF2-HMAC-SHA256,
  10 000 iterations, salted) or an API token (stored as SHA-256). Five wrong passwords lock the login for
  60 s. Sessions expire after 30 min without use.
- The **first** admin password can be set by anyone who reaches the gateway before you: set it right after
  installation.
- Dangerous installer settings (DI1 alarm input, counter resets, outlets, master zone…) need a typed
  confirmation, also through the raw API. They are never exposed over MQTT.
- Passwords, the admin hash and token values never appear in responses, logs, MQTT messages or diagnostics.
- Anyone with access to your MQTT broker can control the climate entities and restart the gateway: protect
  the broker with a user and password.
- Listen mode never transmits on the bus. Master mode refuses to start when it hears another master.

## Firmware images and updates

- Release images are **not signed**. Each release publishes `SHA256SUMS` for its files; the web flasher is
  served from the project's GitHub Pages site over HTTPS.
- Installing firmware over the network always needs the **admin password or an API token**: the HTTP upload
  (`POST /api/v1/ota`), the TCP 7001 upload (token) and the one-click install. Anyone with that password or a
  token can install any firmware, so protect them like the password of the gateway itself.
- The **one-click install** downloads the application image named in the release manifest **over HTTPS
  only** (ESP-IDF certificate bundle), checks its size and image header, and writes the inactive slot. It is
  never automatic and not available over MQTT.
- A new image that is not confirmed within 180 s rolls back to the previous one. Only one firmware write
  runs at a time.
- Flashing over USB needs physical access to the board and is not protected.

## What the gateway contacts on the internet

By default, only the time: the gateway asks `pool.ntp.org` for the time (SNTP), which it needs for the
VMC renewal days and the free-cooling months. Everything else works without internet access.

The **update check is opt-in**: when you tick *Check for updates from the official releases* (Gateway →
Network), the gateway fetches `https://letherman255.github.io/Open-Airzone-Gateway/ota.json` within a minute
of turning it on, 5 minutes after each start, then once a day. The request carries no account and no data about your installation: only what
any HTTPS client sends (IP address, TLS handshake) and the user agent `open-airzone-gateway`. Untick it to
stop. A custom manifest URL can be entered instead; it is then your choice of server.

The one-click install (when you click it) downloads the image named in that manifest, from the same site.
