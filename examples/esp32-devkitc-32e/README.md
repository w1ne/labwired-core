# ESP32-DevKitC-32E GPIO, UART0, BME280 I2C, RMT, and ADC smokes

**Demand rank:** ESP32 is rank 1 in the standing list (38% of reported MCU
use). The current ADC smoke adds the stock `analogRead(36)` path to the
existing board coverage; it is peripheral depth on the highest-demand family,
not a new MCU target or board variant.

## Board and modeled scope

The board is Espressif's exact **ESP32-DevKitC-32E** with an ESP32-WROOM-32E
module and 4 MB flash. The narrow board profile exposes classic ESP32 GPIO,
UART0 console, I2C0, and ADC1 channel 0. GPIO23 maps to J3 pin 2; it is a
header pin, not a built-in LED. For a physical blink, connect an LED in series
with a resistor from GPIO23 to GND.

| Board connection | MCU behavior | Smoke |
| --- | --- | --- |
| J3 pin 2 | GPIO23 | Base smoke toggles the output and requires at least 16 edges |
| UART0 output | Serial console capture | Base smoke asserts `ESP32_DEVKITC32E_UART0_OK` |
| J2 GPIO21/GPIO22 | I2C0 SDA/SCL | BME280 smoke reads sensor ID and temperature ADC data |
| J2 pin 3 / VP (GPIO36) | ADC1 channel 0 | ADC smoke reads a 1650 mV potentiometer through `analogRead(36)` and expects 2047 at 12-bit resolution |
| GPIO23 header | RMT channel 0 TX matrix output | RMT smoke sends red to an external WS2812 pixel and waits for TX_END |
| GPIO18 header | RMT channel 1 TX matrix output | RMT smoke sends green to a second pixel and waits for TX_END |

The BME280 breakout uses the standard `0x77` address and is powered directly
from board 3V3/GND. Firmware reads ID register `0xD0` (`0x60`) and the raw
temperature registers `0xFA..0xFC` (`0x082DC0` in the deterministic model).
The breakout, rather than a built-in board component, is declared as an
external device in the BME280 system manifest.

The ADC test uses the board's J2 pin 3 (VP), which Espressif maps to GPIO36 /
ADC1_CH0. Its potentiometer is an external simulation fixture, not a component
fitted to the board. The default midpoint supplies 1650 mV and produces code
2047 in the deterministic 12-bit model. The ADC model does not include noise,
calibration, or the analog attenuation curve, so this is a simulator input
check rather than a physical accuracy claim.

The RMT model implements per-channel TX control/status offsets, RMTMEM symbol
RAM, independent TX timing for channels 0 and 1, GPIO-matrix output routing,
and per-channel TX_END after the stream's reset interval. It does not model RX,
or advancing APB FIFO pointers. This smoke exercises channels 0 and 1; channels
2–7 use the same transmit path but do not have a smoke here. The pixels are
external generic WS2812-compatible fixtures, not onboard DevKitC LEDs. The
UART0 controller is modeled. The board's USB-to-UART bridge and the pin-level
UART TX/RX waveforms are not, so console capture does not prove USB serial
behavior or a signal at the board header. Wi-Fi/Bluetooth, SPI, timers, flash
boot behavior, and the 5 V power LED are outside these profiles. I2S0/1,
UHCI0, PCNT, APB_CTRL and DPORT analog-AHB retain fallback windows.

## Copy-paste firmware example

This complete Arduino sketch reads the chip ID and raw temperature registers
from an Adafruit BME280 breakout on GPIO21/GPIO22:

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kBme280Address = 0x77;

bool readRegisters(uint8_t first, uint8_t *bytes, uint8_t count) {
  Wire.beginTransmission(kBme280Address);
  Wire.write(first);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(kBme280Address, count) != count) return false;
  for (uint8_t i = 0; i < count; ++i) bytes[i] = Wire.read();
  return true;
}

bool writeRegister(uint8_t reg, uint8_t value) {
  Wire.beginTransmission(kBme280Address);
  Wire.write(reg);
  Wire.write(value);
  return Wire.endTransmission() == 0;
}

void setup() {
  Serial.begin(115200);
  Wire.begin(21, 22, 100000);  // SDA=GPIO21, SCL=GPIO22

  uint8_t id = 0;
  if (!readRegisters(0xD0, &id, 1) || id != 0x60) {
    Serial.println("BME280 not found");
    return;
  }

  // Enable humidity first, then temperature/pressure oversampling and normal mode.
  if (!writeRegister(0xF2, 0x01) || !writeRegister(0xF4, 0x27)) {
    Serial.println("BME280 setup failed");
    return;
  }
  delay(100);

  uint8_t temperature[3] = {};
  if (!readRegisters(0xFA, temperature, sizeof(temperature))) {
    Serial.println("BME280 read failed");
    return;
  }
  const uint32_t raw = (static_cast<uint32_t>(temperature[0]) << 12) |
                       (static_cast<uint32_t>(temperature[1]) << 4) |
                       (temperature[2] >> 4);
  Serial.printf("BME280_TEMP_RAW=0x%06lX\n", static_cast<unsigned long>(raw));
}

void loop() {}
```

The local acceptance firmware is
[`firmware/src/bme280_smoke.cpp`](firmware/src/bme280_smoke.cpp). It programs
the classic ESP32 I2C0 command registers directly and reads the same ID and
temperature registers. The Arduino ESP32 3.3.12 `Wire` driver stalls after its
first split-command `END_DETECT` boundary in the current simulator, so the
local smoke does not claim to verify that driver path.

The ADC acceptance firmware is [`firmware/src/adc_smoke.cpp`](firmware/src/adc_smoke.cpp).
It calls the Arduino `analogRead(36)` API and checks the resulting ADC1_CH0
value against the potentiometer fixture. The run log and metrics are committed
under [`evidence/adc/`](evidence/adc/).

The RMT smoke firmware is [`firmware/src/rmt_smoke.cpp`](firmware/src/rmt_smoke.cpp).
It writes separate red and green GRB frames into channel 0 and 1 RMTMEM regions,
starts both channels, and waits for both timed TX_END bits. External pixel
models independently decode GPIO23 and GPIO18, and the smoke requires both
colors.

## Reproduce

From the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-bme280 \
  --script examples/esp32-devkitc-32e/bme280-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-bme280.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/bme280 \
  --firmware-build-script scripts/build-esp32-devkitc-32e-bme280.sh
```

The build log, local simulator output, and generated metrics are committed
under `evidence/bme280/`. This is deterministic digital-twin evidence, not a
physical-board run. The classic ESP32 hosted verifier does not support its
serial oracle; the committed local CLI smoke is the acceptance evidence.

Run the RMT + pixel smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-rmt-neopixel \
  --script examples/esp32-devkitc-32e/rmt-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-rmt-neopixel.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/rmt-neopixel \
  --firmware-build-script scripts/build-esp32-devkitc-32e-rmt-smoke.sh
```

The run evidence belongs under `evidence/rmt-neopixel/`. This is deterministic
digital-twin evidence, not a physical-board run.

Run the ADC input smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-adc \
  --script examples/esp32-devkitc-32e/adc-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-adc.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/adc \
  --firmware-build-script scripts/build-esp32-devkitc-32e-adc-smoke.sh
```

Copy-paste Arduino example for a midpoint potentiometer wired from 3V3 to GND
with its wiper on J2 pin 3 / GPIO36:

```cpp
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  analogReadResolution(12);
  const int raw = analogRead(36);  // ADC1_CH0 / VP
  Serial.printf("ADC1_CH0=%d\n", raw);
}

void loop() {}
```

The current lifecycle and stock check is in
[`evidence/vendor-status.md`](evidence/vendor-status.md). The evidence records
why the exact board and WROOM-32E variant satisfy the no-obsolete-parts rule.
The current-round ADC availability and GPIO36 source check is recorded in
[`evidence/adc/vendor-status.md`](evidence/adc/vendor-status.md).
