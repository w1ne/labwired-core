# ESP32-S3-Zero smoke scope

- Board: Waveshare ESP32-S3-Zero, standard FH4R2 option (4 MB flash / 2 MB PSRAM).
- Live availability: [vendor-status.md](vendor-status.md), checked 2026-11-11.
- Smoke: UART0 marker, timed GPIO21 output, and a decoded one-pixel WS2812 frame.
- System: `configs/systems/esp32s3-zero.yaml`, using the 4 MB S3-Zero chip descriptor.
- Local onboarding smoke: LabWired's fast-load test path. The separate hosted verifier used its ROM-flash boot path; neither run verifies physical USB download. Wi-Fi, BLE, and PSRAM are outside both runs.
- Hosted oracle result: `model_verified` for the UART marker and GPIO21 transitions; the returned WS2812 artifact decoded one green pixel. The hosted diagram assumes ideal MCU VDD, so power-on and brownout behavior remain unverified.
- Remaining stubs outside the smoke path: S3 SYSTEM, RTC_CNTL, and eFuse register windows. Radio behavior is not part of this board profile.
- No physical-board capture was run.
