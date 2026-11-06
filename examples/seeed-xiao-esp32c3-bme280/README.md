# Seeed Studio XIAO ESP32-C3 + BME280

**Demand rank:** ESP32 is the #1 MCU family in the 2025 JetBrains State of
Embedded survey (38%). This is low-cost board-path depth for an already
supported C3 family, not a new MCU or peripheral family. The existing XIAO
profile covered GPIO only; this profile adds a tested I2C sensor design.

The narrow profile exposes the XIAO's D4/D5 I2C0 route, GPIO matrix, a UART0
console capture, and an attached BME280 model. Firmware performs repeated-start
I2C reads of the sensor ID and raw temperature ADC bytes. The smoke asserts both
values; the BME280 model's default inputs correspond to 25 °C, 1013.25 hPa, and
50% RH. The raw ADC bytes are a deterministic register-path check, not a
physical environmental measurement or compensated temperature claim.

## Wiring

Connect an Adafruit BME280 STEMMA QT breakout to the XIAO ESP32-C3:

| BME280 | XIAO ESP32-C3 |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | D4 / GPIO6 |
| SCK/SCL | D5 / GPIO7 |

The example uses address `0x77`. Leave CS unconnected for I2C and set SDO to
the breakout's `0x77` address selection. See [live availability and pin sources](evidence/vendor-status.md).

## Copy-paste Arduino example

Install the Arduino ESP32 core, Adafruit BME280 library, and Adafruit Unified
Sensor library, then upload:

```cpp
#include <Wire.h>
#include <Adafruit_Sensor.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin(D4, D5);  // XIAO D4/SDA = GPIO6, D5/SCL = GPIO7
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }
  Serial.print("Temperature C: ");
  Serial.println(bme.readTemperature());
  Serial.print("Pressure hPa: ");
  Serial.println(bme.readPressure() / 100.0F);
  Serial.print("Humidity %: ");
  Serial.println(bme.readHumidity());
}

void loop() {}
```

## Reproduce the repository smoke

From the repository root:

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id seeed-xiao-esp32c3-bme280 \
  --crate firmware-esp32c3-bme280-smoke \
  --target riscv32imc-unknown-none-elf \
  --firmware-build-script examples/seeed-xiao-esp32c3-bme280/build-firmware.sh \
  --script examples/seeed-xiao-esp32c3-bme280/onboarding-smoke.yaml \
  --system configs/systems/seeed-xiao-esp32c3-bme280.yaml \
  --out-dir examples/seeed-xiao-esp32c3-bme280/evidence
  --out-dir examples/seeed-xiao-esp32c3-bme280/evidence
```

The profile does not claim radio, USB, ROM boot, SPI, ADC, timers, physical
UART pin propagation, sensor conversion timing, power or analog behavior. It
captures UART0 directly for the smoke marker. The result is repository
simulator evidence, not physical-board testing or a hosted `labwired_verify`
verdict; see [`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).
