# ESP32-S3-DevKitC-1 v1.1 N16R8 board smoke

The base smoke boots firmware on the ESP32-S3-DevKitC-1 v1.1 N16R8, drives its
onboard WS2812 RGB LED red through GPIO38, and sends a UART0 marker. The I²C
extension also reads an Adafruit BME280 breakout through I²C0. The system
profiles are
[`configs/systems/esp32-s3-devkitc-1-n16r8.yaml`](../../../configs/systems/esp32-s3-devkitc-1-n16r8.yaml).
The sensor profile is
[`configs/systems/esp32-s3-devkitc-1-n16r8-bme280.yaml`](../../../configs/systems/esp32-s3-devkitc-1-n16r8-bme280.yaml).

## Pin map

| Board connection | MCU signal | Pin | Scope |
| --- | --- | --- | --- |
| Onboard RGB LED DIN (v1.1) | GPIO output | GPIO38 | Bit-banged WS2812 GRB frame is decoded by the attached LED model |
| J3-2 TX | UART0 TX / U0TXD | GPIO43 | UART FIFO is exercised; USB bridge is omitted |
| J3-3 RX | UART0 RX / U0RXD | GPIO44 | Pin is mapped; receive path is not exercised |
| Header IO8 | I²C0 SDA | GPIO8 | BME280 register reads in the I²C extension |
| Header IO9 | I²C0 SCL | GPIO9 | BME280 register reads in the I²C extension |

Espressif's [current user guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
documents the header signals and the LED pin. The v1.1 LED is wired to GPIO38;
the v1.0 LED is wired to GPIO48, so this profile explicitly models v1.1. The
twin captures UART0 output directly; it does not model the USB-to-UART bridge
or claim physical USB serial behavior.

## Scope and limits

- The base profile uses the S3 GPIO and UART0 models plus the WS2812 component
  model. It checks UART output, at least 48 GPIO38 edges, and a decoded red pixel.
- The BME280 profile adds the S3 I²C0 model and the BME280 component model. It
  checks the `0x60` chip ID and the modeled raw temperature register sample.
- The frame is bit-banged through GPIO38. RMT, PSRAM, Wi-Fi/BLE, buttons,
  native USB, the USB-to-UART bridge, and ESP-IDF boot-ROM startup are outside
  both smokes.
- Other S3 blocks remain outside this profile; the runtime builder supplies
  generic MMIO placeholders for unmodeled addresses. The smoke does not access
  or claim behavior for those blocks.
- This profile covers board revision v1.1 only. The separate v1.0 GPIO48 LED
  wiring is not claimed.
- The simulation assumes an ideal VDD rail. Power-on and brown-out reset
  behavior are outside this evidence.
- This is local simulation evidence; no physical-board capture is recorded.
- The copy-paste Arduino sketch compiles on the hosted ESP32-S3 toolchain, but
  its hosted run currently stalls in the UART0 status wait before producing
  either BME280 marker. See
  [`evidence/bme280/hosted-verification.md`](evidence/bme280/hosted-verification.md);
  this result is not a hosted model-verification pass.

## BME280 I²C extension

This extension models the board with an Adafruit BME280 STEMMA QT breakout at
I²C address `0x77`. Connect its VIN/3V and GND to the board's 3V3 and GND,
then SDA to IO8 and SCL to IO9. The firmware reads register `0xD0` and the
three raw temperature bytes beginning at `0xFA` using real I²C0 transactions.
The local smoke log and structured result are in `evidence/bme280/`.

Reproduce it from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-s3-devkitc-1-n16r8-bme280 \
  --script examples/esp32-s3-devkitc-1-n16r8/bme280-smoke.yaml \
  --system configs/systems/esp32-s3-devkitc-1-n16r8-bme280.yaml \
  --firmware-build-script examples/esp32-s3-devkitc-1-n16r8/ci/build-bme280-firmware.sh \
  --out-dir examples/esp32-s3-devkitc-1-n16r8/evidence/bme280
```

The copy-paste Arduino example is
[`firmware/arduino-bme280/main.ino`](firmware/arduino-bme280/main.ino). The
local smoke uses the fast-boot register-level harness in
[`firmware/bme280_smoke.c`](firmware/bme280_smoke.c).
The checked-in availability receipt is
[`evidence/bme280/vendor-status.md`](evidence/bme280/vendor-status.md).

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
`evidence/simulation-output/`. The expected transcript is
`ESP32_S3_DEVKITC1_V11_N16R8_OK`; the smoke also checks that the LED model
decodes a red pixel.

## Evidence

- [`evidence/vendor-status.md`](evidence/vendor-status.md) records the live
  Espressif catalog, revision pin map, and retailer availability check.
- [`evidence/model-verification.json`](evidence/model-verification.json) records
  the hosted UART and GPIO38 edge oracle. It assumes ideal VDD; it does not
  assert the WS2812 pixel color.
- [`evidence/model-verification-2026-11-04.json`](evidence/model-verification-2026-11-04.json)
  and [`evidence/verify-request-2026-11-04.json`](evidence/verify-request-2026-11-04.json)
  are the earlier UART-only check.
- [`evidence/simulation-output/result.json`](evidence/simulation-output/result.json)
  records the local four-assertion smoke, including the decoded red pixel.
- [`firmware/board_smoke.c`](firmware/board_smoke.c) and `firmware/link.ld` are
  the local fast-boot firmware.
