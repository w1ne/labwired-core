# ESP32-S3-DevKitC-1 v1.1 N16R8 board smoke

The base smoke boots firmware on the ESP32-S3-DevKitC-1 v1.1 N16R8, sends a
GRB frame to its onboard WS2812 RGB LED with RMT channel 0 through GPIO38, and
sends a UART0 marker. The I²C extension reads an Adafruit BME280 breakout
through I²C0, and the ADC extension samples a potentiometer on GPIO4 / ADC1_CH3.
The system profiles are
[`configs/systems/esp32-s3-devkitc-1-n16r8.yaml`](../../../configs/systems/esp32-s3-devkitc-1-n16r8.yaml),
[`configs/systems/esp32-s3-devkitc-1-n16r8-bme280.yaml`](../../../configs/systems/esp32-s3-devkitc-1-n16r8-bme280.yaml),
and [`configs/systems/esp32-s3-devkitc-1-n16r8-adc.yaml`](../../../configs/systems/esp32-s3-devkitc-1-n16r8-adc.yaml).

## Pin map

| Board connection | MCU signal | Pin | Scope |
| --- | --- | --- | --- |
| Onboard RGB LED DIN (v1.1) | RMT TX channel 0 through GPIO matrix | GPIO38 | Timed WS2812 GRB symbols are decoded by the attached LED model |
| J3-2 TX | UART0 TX / U0TXD | GPIO43 | UART FIFO is exercised; USB bridge is omitted |
| J3-3 RX | UART0 RX / U0RXD | GPIO44 | Pin is mapped; receive path is not exercised |
| J1-4 | ADC1_CH3 | GPIO4 | ADC extension samples a 1650 mV potentiometer input |
| Header IO8 | I²C0 SDA | GPIO8 | BME280 register reads in the I²C extension |
| Header IO9 | I²C0 SCL | GPIO9 | BME280 register reads in the I²C extension |

Espressif's [current user guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
documents the header signals and the LED pin. The v1.1 LED is wired to GPIO38;
the v1.0 LED is wired to GPIO48, so this profile explicitly models v1.1. The
twin captures UART0 output directly; it does not model the USB-to-UART bridge
or claim physical USB serial behavior.

## Scope and limits

- The base profile uses the S3 GPIO, UART0, and RMT models plus the WS2812
  component model. Firmware sends its UART marker only after observing RMT
  channel 0's TX_END status; the smoke also asserts the decoded red pixel.
- The BME280 profile adds the S3 I²C0 model and the BME280 component model. It
  checks the `0x60` chip ID and the modeled raw temperature register sample.
- The ADC profile attaches a potentiometer to SENS ADC1 channel 3, which the
  current board guide maps to GPIO4 / J1 pin 4. Firmware starts a SENS oneshot
  conversion and requires the 1650 mV input to produce code 2047.
- RMT TX completion is instantaneous in the model while the GPIO waveform is
  timed. The playback clock uses the declared CPU clock and default RMT source
  selection; fractional-divider changes and alternate source selections are
  outside this profile. RMT RX, PSRAM, Wi-Fi/BLE, buttons, native USB, the
  USB-to-UART bridge, and ESP-IDF boot-ROM startup are outside both smokes.
- The generic GPIO edge assertion undercounts this batched RMT waveform (29
  observed for a 48-transition frame). The smoke gates on RMT TX_END and the
  decoded red pixel; complete logic-analyzer edge capture is not claimed.
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

## ADC1_CH3 potentiometer extension

Use a 10 kΩ potentiometer between 3V3 and GND, with its wiper on GPIO4 / J1
pin 4. The ADC system profile drives the same input from a centered 1650 mV
potentiometer model. Its register-level firmware smoke selects SENS ADC1 channel
3, starts a conversion, and requires code 2047. This proves the board-to-ADC
input path in the local model; it is not an analog accuracy or physical-board
measurement.

Copy-paste Arduino example for a 12-bit raw reading:

```cpp
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  analogReadResolution(12);
  Serial.printf("GPIO4_ADC1_CH3=%d\n", analogRead(4));
}

void loop() {}
```

Reproduce the local repository smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-s3-devkitc-1-n16r8-adc \
  --script examples/esp32-s3-devkitc-1-n16r8/adc-smoke.yaml \
  --system configs/systems/esp32-s3-devkitc-1-n16r8-adc.yaml \
  --firmware-build-script examples/esp32-s3-devkitc-1-n16r8/ci/build-adc-firmware.sh \
  --out-dir examples/esp32-s3-devkitc-1-n16r8/evidence/adc
```

The local register-level harness is
[`firmware/adc_smoke.c`](firmware/adc_smoke.c); the script checks for
`ESP32_S3_DEVKITC1_ADC1_CH3_2047_OK`. See the dated
[vendor-status receipt](evidence/adc/vendor-status.md) and committed build and
simulation artifacts under [`evidence/adc/`](evidence/adc/).

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
`ESP32_S3_DEVKITC1_V11_RMT_OK`; the marker is sent after RMT TX_END, and the
smoke asserts that the LED model decodes a red pixel.

## Evidence

- [`evidence/vendor-status.md`](evidence/vendor-status.md) records the live
  Espressif catalog, revision pin map, and retailer availability check.
- [`evidence/rmt-model-provenance.md`](evidence/rmt-model-provenance.md) records
  the vendor-SVD register claims and the model-only RMT output-signal mapping.
- [`evidence/model-verification.json`](evidence/model-verification.json) records
  the 2026-11-06 hosted RMT TX_END marker check. It assumes ideal VDD and does
  not assert WS2812 pixel color; the local repository smoke asserts the pixel.
- [`evidence/model-verification-2026-11-05.json`](evidence/model-verification-2026-11-05.json)
  is the earlier hosted check for the prior bit-banged smoke.
- [`evidence/verify-request-2026-11-06.json`](evidence/verify-request-2026-11-06.json)
  preserves the hosted verification request and inline source.
- [`evidence/model-verification-2026-11-04.json`](evidence/model-verification-2026-11-04.json)
  and [`evidence/verify-request-2026-11-04.json`](evidence/verify-request-2026-11-04.json)
  are the earlier UART-only check.
- [`evidence/simulation-output/result.json`](evidence/simulation-output/result.json)
  records the local four-assertion smoke, including the decoded red pixel.
- [`firmware/board_smoke.c`](firmware/board_smoke.c) and `firmware/link.ld` are
  the local fast-boot firmware.

## ESP32-S3-DevKitC-1 N16R8 v1.0

The current base smoke above covers v1.1, whose onboard WS2812 LED is on
GPIO38. Espressif documents v1.0 boards with the same LED on GPIO48; the
separate [v1.0 system profile](../../configs/systems/esp32-s3-devkitc-1-n16r8-v10.yaml)
and [chip profile](../../configs/chips/esp32s3-devkitc-1-n16r8-v10-smoke.yaml)
model that exact wiring. The narrow simulation assumes an ideal VDD rail and boots a register-level image,
routes RMT channel 0 to GPIO48, checks a non-black decoded pixel, and captures
UART0 marker `ESP32_S3_DEVKITC1_V10_RMT_OK`. It does not assert the pixel's
color or claim the ESP-IDF boot path, USB bridge, radio, PSRAM, or a physical
board run.

For a copy-paste Arduino example, select **ESP32-S3 Dev Module** and use
[`firmware/arduino-v10/main.ino`](firmware/arduino-v10/main.ino). It configures
a red RMT frame for GPIO48; use GPIO38 on v1.1 boards. The hosted check decoded
the direct-register frame as black and a separate Adafruit NeoPixel probe
decoded no pixels, so neither hosted LED path is verified.
The [availability receipt](evidence/v10/vendor-status.md) records Espressif's
live product listing and revision-specific pin documentation.

Reproduce the v1.0 local smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-s3-devkitc-1-n16r8-v10 \
  --script examples/esp32-s3-devkitc-1-n16r8/v10-smoke.yaml \
  --system configs/systems/esp32-s3-devkitc-1-n16r8-v10.yaml \
  --firmware-build-script examples/esp32-s3-devkitc-1-n16r8/ci/build-v10-firmware.sh \
  --out-dir examples/esp32-s3-devkitc-1-n16r8/evidence/v10
```

The committed output in `evidence/v10/` includes build logs, simulator logs,
structured assertions, and onboarding metrics.
The hosted verification record in [`evidence/v10/hosted-verification.md`](evidence/v10/hosted-verification.md)
notes that the UART marker passed while the hosted RGB LED artifact remained
black. That hosted LED result is not claimed as verified; the committed local
smoke separately passes its non-black-pixel assertion.
