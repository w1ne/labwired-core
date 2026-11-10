# Onboarding Smoke: esp32-devkitc-32e-microsd

- Demand rank: ESP32 rank 1 (38% reported MCU use).
- Work type: low-cost integration on an existing board, not a new board/MCU demand pick.
- Target: exact Espressif ESP32-DevKitC-32E / ESP32-WROOM-32E with an Adafruit
  Micro SD SPI/SDIO breakout (PID 4682) on VSPI/SPI3.
- Behavior: after 80 startup clocks with CS high, firmware checks CMD0 idle
  response, CMD8/R7 echo, CMD55 application command prefix, and ACMD41 ready
  response; GPIO16 rises only on a full match.
- status: `pass`
- Local smoke: `PASS 2/2 checks`; logs and structured simulation output are committed.
- Hosted verifier: `status: model_verified`; one GPIO16 edge satisfied the oracle.
- Limits: card initialization only; the MicroSD model has no filesystem or
  persistent card image. Hosted verification assumes an ideal 3.3 V supply.
- Live lifecycle and stock: [`vendor-status.md`](vendor-status.md).
- Hosted oracle evidence: [`hosted-verification.md`](hosted-verification.md).
- Firmware and smoke entry points: [`microsd_smoke.cpp`](../../firmware/src/microsd_smoke.cpp),
  [`microsd-smoke.yaml`](../../microsd-smoke.yaml).
- elapsed_seconds: `5`
- elapsed_ms: `4570`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `n/a`
- first_error_signature: `n/a`
- failure_hint: `n/a`
