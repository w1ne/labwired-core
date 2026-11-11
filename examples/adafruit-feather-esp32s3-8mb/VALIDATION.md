# Validation record

Demand rank: **ESP32 #1 (38%)**. This is a board profile for existing ESP32-S3
silicon.

The PlatformIO firmware uses the official Arduino Feather no-PSRAM variant and
8 MB flash layout. The simulation smoke checks its UART0 marker, at least 48
GPIO33 transitions for the built-in NeoPixel frame, and the bounded
`max_steps` stop. Build and simulator output are committed in `evidence/`.
The recorded NeoPixel artifact contains GRB `[64, 0, 0]`, with
`pixels_decoded: 1` and `lit_pixels: 1`.
The tested ELF SHA-256 is
`b808365038c1f0646ed96e2b8f3e85ee5b0dc1a07578a1ce58e853d0d5a11b19`.

This is digital-twin evidence. It does not validate a physical board or its
electrical behavior. GPIO21 is driven high by the firmware, but the system model
does not electrically couple that power gate to the NeoPixel device. USB,
Wi-Fi, BLE, charger/monitor, and STEMMA QT power-switch behavior are outside
this profile. The direct GPIO-timed smoke exercises the WS2812 protocol;
Arduino library transmission through RMT is not claimed by this profile.

The hosted `labwired_verify` catalog does not accept this Feather board ID, so
this result is a passing local CLI smoke with committed evidence, not a hosted
`model_verified` verdict.

`cargo check -p labwired-wasm` also completed successfully for the browser
runner changes.
