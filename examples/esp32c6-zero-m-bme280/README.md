# Waveshare ESP32-C6-Zero-M + BME280

**Demand position:** ESP32 is rank 1 in the measured MCU list (38%). Demand for
the exact C6-Zero-M board is not measured separately. This adds a board-specific
I2C sensor path to the existing ESP32-C6 model; it does not add new silicon.

The profile uses the 4 MB `esp32c6fh4` memory map to match the currently stocked
Electrokit listing. Waveshare's current product page says C6FH8 / 8 MB, so the
profile deliberately limits the modeled flash to 4 MB. The live vendor and
stock evidence is in [`vendor-status.md`](vendor-status.md).

## Wiring

Use the C6-Zero-M header version and a 3.3 V BME280 breakout at address `0x77`.
The vendor pinout exposes GPIO6 and GPIO7 on the headers.

| BME280 breakout | ESP32-C6-Zero-M |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | GPIO6 |
| SCL | GPIO7 |

The breakout must provide I2C pull-ups to 3.3 V. The firmware routes I2C0
through the C6 GPIO matrix, reads the BME280 chip ID (`0x60`) and raw
temperature registers, then writes both values to UART0. The simulator captures
UART0 internally; it does not model the board's USB interface or UART pads.

The local smoke reuses
`firmware-esp32c6-devkitc-bme280-smoke`. That ELF uses an 8 MB linker region
for DevKitC compatibility, but its loaded segments fit within this profile's
4 MB flash window and the simulator ran it successfully there.

## Copy-paste Arduino example

Install Adafruit's `Adafruit BME280 Library` and `Adafruit Unified Sensor`
libraries, connect the breakout as shown above, and upload:

```cpp
#include <Adafruit_BME280.h>
#include <Arduino.h>
#include <Wire.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin(6, 7, 100000);  // SDA=GPIO6, SCL=GPIO7
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    return;
  }
  Serial.printf("BME280_ID=0x%02X\n", bme.sensorID());
  Serial.printf("Temperature C: %.2f\n", bme.readTemperature());
}

void loop() {}
```

## Reproduce the local simulator smoke

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c6-zero-m-bme280 \
  --script examples/esp32c6-zero-m-bme280/bme280-smoke.yaml \
  --system configs/systems/esp32c6-zero-m-bme280.yaml \
  --firmware-build-script examples/esp32c6-zero-m-bme280/ci/build.sh \
  --out-dir examples/esp32c6-zero-m-bme280/evidence
```

The smoke exercises the C6 HP core, UART0 capture, I2C0 command engine, GPIO
matrix route, and BME280 model. It does not claim the onboard WS2812 LED, USB,
Wi-Fi, Bluetooth, 802.15.4, LP core, reset/strap behavior, or physical-board
validation.
