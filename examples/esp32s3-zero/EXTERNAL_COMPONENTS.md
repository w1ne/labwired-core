# ESP32-S3-Zero external components

| Component | Board connection | Model and exercised behavior |
| --- | --- | --- |
| Onboard WS2812 RGB LED | GPIO21 data input | `neopixel` component; decodes the timed GRB frame `[64, 0, 0]` |

The board's USB-C port, Wi-Fi, BLE, and 2 MB PSRAM are not modeled by this smoke profile.
