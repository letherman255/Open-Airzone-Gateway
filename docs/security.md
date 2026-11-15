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
- Release images are signed; a gateway running a signed image refuses unsigned updates. Development builds
  are unsigned.
- Listen mode never transmits on the bus. Master mode refuses to start when it hears another master.
