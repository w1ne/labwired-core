# ESP32-S3-DevKitC-1-N16R8 UART0 smoke

This smoke boots firmware on the ESP32-S3-DevKitC-1-N16R8 and checks a UART0
transmit marker. The system profile is
[`configs/systems/esp32-s3-devkitc-1-n16r8.yaml`](../../../configs/systems/esp32-s3-devkitc-1-n16r8.yaml).

## Pin map

| Board connection | MCU signal | Pin | Scope |
| --- | --- | --- | --- |
| J3-2 TX | UART0 TX / U0TXD | GPIO43 | UART FIFO is exercised; USB bridge is omitted |
| J3-3 RX | UART0 RX / U0RXD | GPIO44 | Pin is mapped; receive path is not exercised |

Espressif's [current user guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
documents these header signals and the USB-to-UART bridge. The twin captures
the UART peripheral output; it does not model that bridge or claim physical
USB serial behavior.

## Scope and limits

- The narrow profile exposes the real S3 UART0 and GPIO models needed for this
  pin map. GPIO is declared for the pad mapping but not exercised. Firmware
  writes UART0 FIFO and checks the marker in the local simulator. A separate
  hosted Arduino sketch is checked with the serial oracle.
- PSRAM, Wi-Fi/BLE, buttons, LEDs, native USB and ESP-IDF boot-ROM startup are
  outside this smoke. The board's RGB LED pin changes between board revisions,
  so it is not claimed here.
- The hosted verifier assumes an ideal VDD rail. Power-on and brown-out reset
  behavior are outside this evidence.
- This is simulation evidence; no physical-board capture is recorded.

## Reproduce the local repository smoke

From the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-s3-devkitc-1-n16r8 \
  --script examples/esp32-s3-devkitc-1-n16r8/onboarding-smoke.yaml \
  --system configs/systems/esp32-s3-devkitc-1-n16r8.yaml \
  --firmware-build-script examples/esp32-s3-devkitc-1-n16r8/ci/build-firmware.sh \
  --out-dir examples/esp32-s3-devkitc-1-n16r8/evidence
```

The committed local build and simulator logs are in `evidence/logs/` and
`evidence/simulation-output/`.

## Evidence

- [`evidence/model-verification.json`](evidence/model-verification.json) is the
  raw hosted LabWired verification response from 2026-11-04 UTC.
- [`evidence/verify-request.json`](evidence/verify-request.json) records the
  hosted board, diagram, source and required marker oracle.
- [`evidence/vendor-status.md`](evidence/vendor-status.md) records live
  lifecycle and retailer availability evidence.
- [`firmware/uart_smoke.c`](firmware/uart_smoke.c) and `firmware/link.ld` are
  the local fast-boot firmware; [`src/main.ino`](src/main.ino) is the hosted
  Arduino verification source.
