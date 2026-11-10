# ESP32-S3-DevKitC-1 v1.1 N16R8 board smoke

The base smoke boots firmware on the ESP32-S3-DevKitC-1 v1.1 N16R8, sends a
GRB frame to its onboard WS2812 RGB LED with RMT channel 0 through GPIO38, and
sends a UART0 marker. The board extensions read BME280 and SHT40 sensors over
I²C0 and I²C1, and sample a potentiometer on GPIO4 / ADC1_CH3.
The system profiles are
[`configs/systems/esp32-s3-devkitc-1-n16r8.yaml`](../../../configs/systems/esp32-s3-devkitc-1-n16r8.yaml),
[`configs/systems/esp32-s3-devkitc-1-n16r8-bme280.yaml`](../../../configs/systems/esp32-s3-devkitc-1-n16r8-bme280.yaml),
[`configs/systems/esp32-s3-devkitc-1-n16r8-i2c1-bme280.yaml`](../../../configs/systems/esp32-s3-devkitc-1-n16r8-i2c1-bme280.yaml),
[`configs/systems/esp32-s3-devkitc-1-n16r8-i2c1-sht40.yaml`](../../../configs/systems/esp32-s3-devkitc-1-n16r8-i2c1-sht40.yaml),
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
| J3-7 | I²C1 SDA | GPIO41 | BME280 and SHT40 profiles |
| J3-6 | I²C1 SCL | GPIO42 | BME280 and SHT40 profiles |

Espressif's [current user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
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
- The I²C1 profile exercises the second external controller at `0x60027000`,
  routed through GPIO matrix signals 91/92 to J3 GPIO42/GPIO41. It uses the
  same BME280 component model and adds no new peripheral model.
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
- The hosted `Wire1` BME280 attempt on GPIO41/GPIO42 did not find the sensor;
  the catalog exposes I²C0 but not I²C1 in its board bus summary. The local
  I²C1 register-level smoke passes. See
  [`evidence/i2c1-bme280/hosted-verification.md`](evidence/i2c1-bme280/hosted-verification.md).

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

## BME280 I²C1 extension

This is maintenance/depth work on the same rank-1 ESP32 family board, not a
new board or MCU pick. It exercises the second external I²C controller while
keeping the existing I²C0 profile on GPIO8/GPIO9. Connect the Adafruit BME280
breakout VIN/3V and GND to board 3V3 and GND, SDA to J3 pin 7 / GPIO41, and
SCL to J3 pin 6 / GPIO42. Set SDO high for I²C address `0x77`.

Copy-paste Arduino example:

```cpp
#include <Adafruit_BME280.h>
#include <Arduino.h>
#include <Wire.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire1.begin(41, 42, 100000);  // I2C1 SDA=GPIO41, SCL=GPIO42
  if (!bme.begin(0x77, &Wire1)) {
    Serial.println("BME280 not found on I2C1");
    return;
  }
  Serial.printf("BME280_ID=0x%02X\n", bme.sensorID());
  Serial.printf("Temperature C: %.2f\n", bme.readTemperature());
}

void loop() {}
```

Reproduce the local register-level smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-s3-devkitc-1-n16r8-i2c1-bme280 \
  --script examples/esp32-s3-devkitc-1-n16r8/i2c1-bme280-smoke.yaml \
  --system configs/systems/esp32-s3-devkitc-1-n16r8-i2c1-bme280.yaml \
  --firmware-build-script examples/esp32-s3-devkitc-1-n16r8/ci/build-i2c1-bme280-firmware.sh \
  --out-dir examples/esp32-s3-devkitc-1-n16r8/evidence/i2c1-bme280
```

The live availability check is in
[`evidence/i2c1-bme280/vendor-status.md`](evidence/i2c1-bme280/vendor-status.md).

## SHT40 temperature and humidity on I²C1

The [I²C1 SHT40 system](../../../configs/systems/esp32-s3-devkitc-1-n16r8-i2c1-sht40.yaml)
attaches an Adafruit SHT40 STEMMA QT breakout (PID 4885) at address 0x44. Wire
VIN to 3V3, GND to GND, SDA to J3 pin 7 / GPIO41, and SCL to J3 pin 6 / GPIO42.
The profile runs a high-precision measurement and validates both data words
and their CRC bytes against the attached SHT40 model.

Copy-paste Arduino example using Adafruit's SHT4x library:

~~~cpp
#include <Adafruit_SHT4x.h>
#include <Arduino.h>
#include <Wire.h>

Adafruit_SHT4x sht4;

void setup() {
  Serial.begin(115200);
  Wire1.begin(41, 42, 100000);  // I2C1 SDA=GPIO41, SCL=GPIO42
  if (!sht4.begin(&Wire1)) {
    Serial.println("SHT40 not found on I2C1");
    return;
  }
  sht4.setPrecision(SHT4X_HIGH_PRECISION);
  sensors_event_t humidity, temperature;
  if (sht4.getEvent(&humidity, &temperature)) {
    Serial.printf("Temperature: %.2f C, RH: %.2f %%\n",
                  temperature.temperature, humidity.relative_humidity);
  }
}

void loop() {}
~~~

Reproduce the local register-level smoke from the repository root:

~~~sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-s3-devkitc-1-n16r8-i2c1-sht40 \
  --script examples/esp32-s3-devkitc-1-n16r8/sht40-smoke.yaml \
  --system configs/systems/esp32-s3-devkitc-1-n16r8-i2c1-sht40.yaml \
  --firmware-build-script examples/esp32-s3-devkitc-1-n16r8/ci/build-sht40-firmware.sh \
  --out-dir examples/esp32-s3-devkitc-1-n16r8/evidence/sht40
~~~

The committed local smoke is in
[evidence/sht40/](evidence/sht40/). Its live vendor availability receipt is
[evidence/sht40/vendor-status.md](evidence/sht40/vendor-status.md), and the
hosted catalog limitation is recorded in
[evidence/sht40/hosted-catalog.md](evidence/sht40/hosted-catalog.md).

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
