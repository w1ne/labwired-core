# Onboarding Smoke: esp32-devkitc-32e-i2c1-bme280

- Demand rank: ESP32 rank 1 (38% reported MCU use).
- Work type: low-cost profile extension on an existing board; not a new
  board/MCU demand pick.
- Target: exact ESP32-DevKitC-32E / ESP32-WROOM-32E with a BME280 breakout on
  I2C1, SDA=GPIO16 / J3 pin 12 and SCL=GPIO17 / J3 pin 11.
- Behavior: firmware issues I2C1 command-list transactions and reads BME280
  ID `0x60` plus raw temperature `0x082DC0`.
- status: `pass`
- Local smoke: `PASS 2/2 checks`; exact ID and temperature markers asserted.
- Limits: uses direct I2C1 register commands; the Arduino `Wire1` path and
  physical hardware behavior are outside this run.
- Live lifecycle and stock: [`vendor-status.md`](vendor-status.md).
- Build, run logs, structured output, and metrics are committed alongside this
  summary.
- elapsed_seconds: `44`
- elapsed_ms: `43056`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `n/a`
- first_error_signature: `n/a`
- failure_hint: `n/a`
