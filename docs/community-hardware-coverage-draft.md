# Community hardware coverage — draft

**Draft only. Not published.** Publication still needs Andrii's explicit OK.

This page starts the demand-ranked coverage list with runnable board examples.
Each entry links to the exact hardware scope, copy-paste firmware, vendor status,
and simulator evidence. More demand-ranked targets remain to be added.

## ESP32 — rank 1, 38% of reported MCU use

### ESP32-S3-DevKitC-1 N8R8 + BME280

Espressif's current guide lists the N8R8 ordering code and a retail sample path.
The local N8R8 profile boots a direct-register smoke and reads the BME280 ID
`0x60` and raw-temperature sample over I2C0 on GPIO8/GPIO9. The hosted Arduino
example returned `model_verified` on the generic DevKitC-1 catalog target;
that target does not model the exact N8R8 flash size, which the local smoke
checks. VDD is idealized in the hosted run.

- [Wiring, scope, and copy-paste example](../examples/esp32-s3-devkitc-1-n8r8/README.md)
- [Live Espressif availability receipt](../examples/esp32-s3-devkitc-1-n8r8/evidence/bme280/vendor-status.md)
- [Committed local and hosted evidence](../examples/esp32-s3-devkitc-1-n8r8/evidence/bme280/)

```cpp
#include <Adafruit_BME280.h>
#include <Arduino.h>
#include <Wire.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin(8, 9, 100000);  // SDA=GPIO8, SCL=GPIO9
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    return;
  }
  Serial.printf("BME280_ID=0x%02X\n", bme.sensorID());
  Serial.printf("Temperature C: %.2f\n", bme.readTemperature());
}

void loop() {}
```

The profile does not claim PSRAM, USB, Wi-Fi/BLE, board buttons, or USB-to-UART
bridge behavior. The page remains an unpublished review draft pending Andrii's
approval.

## STM32F103 — rank 3, 17% of reported MCU use

### NUCLEO-F103RB + BME280

The NUCLEO-F103RB's STM32F103RBT6 boots a register-level smoke, reads the
BME280's `CHIP_ID` (`0x60`) over the I2C1 board route (D15/SCL = PB8,
D14/SDA = PB9), and pulses LD2 after the successful read.

- [Wiring, scope, and copy-paste Arduino sketch](../examples/nucleo-f103rb-bme280/README.md)
- [Live vendor status](../examples/nucleo-f103rb-bme280/evidence/vendor-status.md)
- [Committed local simulator evidence](../examples/nucleo-f103rb-bme280/evidence/)

```cpp
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin();  // NUCLEO-F103RB D14/D15: PB9/PB8, I2C1
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }
  Serial.println("NUCLEO_F103RB_BME280_OK");
}

void loop() {
  Serial.print("Temperature C: ");
  Serial.println(bme.readTemperature());
  Serial.print("Pressure hPa: ");
  Serial.println(bme.readPressure() / 100.0F);
  Serial.print("Humidity %: ");
  Serial.println(bme.readHumidity());
  delay(2000);
}
```
