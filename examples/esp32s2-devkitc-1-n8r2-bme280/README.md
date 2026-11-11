# ESP32-S2-DevKitC-1-N8R2 + BME280

ESP32 ranks **#1** in the demand list (38% reported MCU use); the survey does
not rank the S2 separately. This round adds I2C0 and a BME280 read to the
already-onboarded exact N8R2 DevKitC-1 board. The model boots an S2 LX7 ELF,
sets up GPIO-matrix I2C0, reads BME280 ID `0x60` and the deterministic raw
temperature value, then reports the result over UART0.

## Wiring

Connect a BME280 breakout at I2C address `0x77`:

| BME280 | ESP32-S2-DevKitC-1-N8R2 |
| --- | --- |
| SDA | GPIO8 |
| SCL | GPIO9 |
| GND | GND |
| VIN | 3V3 |

The S2 GPIO matrix routes I2C0's SCL/SDA signals to the selected pads. The
breakout should provide the usual I2C pull-ups. The simulator exercises the
GPIO matrix register setup and I2C command-list transactions, with the BME280
responding through the named `i2c0` connection. It does not resolve the sensor
transaction electrically through the GPIO pads or model IO_MUX settings,
external pull-up voltage, environmental sampling, flash boot, PSRAM, Wi-Fi,
USB OTG or the board's USB-to-UART bridge.

## Copy-paste Arduino example

This sketch checks the sensor ID and reads the raw temperature register on a
physical board using the Arduino `Wire` API:

```cpp
#include <Wire.h>

constexpr uint8_t BME280_ADDR = 0x77;

bool readRegister(uint8_t reg, uint8_t *data, size_t count) {
  Wire.beginTransmission(BME280_ADDR);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(BME280_ADDR, static_cast<uint8_t>(count)) != count) return false;
  for (size_t i = 0; i < count; ++i) data[i] = Wire.read();
  return true;
}

void setup() {
  Serial.begin(115200);
  Wire.begin(8, 9);  // SDA, SCL
  uint8_t id = 0, raw[3] = {};
  if (!readRegister(0xD0, &id, 1) || id != 0x60 ||
      !readRegister(0xFA, raw, sizeof(raw))) {
    Serial.println("BME280 read failed");
    return;
  }
  uint32_t temperature = (uint32_t(raw[0]) << 12) |
                         (uint32_t(raw[1]) << 4) | (raw[2] >> 4);
  Serial.printf("BME280 ID=0x%02X TEMP_RAW=0x%06lX\n",
                id, static_cast<unsigned long>(temperature));
}

void loop() {}
```

The committed simulator smoke is separate bare-metal firmware. It exercises
the I2C command-list model and reads the BME280 model through its configured
connection; it is not a physical board run or pin-level electrical simulation.

## Run the local smoke

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32s2-devkitc-1-n8r2-bme280 \
  --firmware-build-script examples/esp32s2-devkitc-1-n8r2-bme280/firmware/build-firmware.sh \
  --script examples/esp32s2-devkitc-1-n8r2-bme280/bme280-smoke.yaml \
  --system configs/systems/esp32s2-devkitc-1-n8r2-bme280.yaml \
  --out-dir examples/esp32s2-devkitc-1-n8r2-bme280/evidence \
  --threshold-seconds 3600
```

See [validation](VALIDATION.md) and [vendor status](vendor-status.md).
