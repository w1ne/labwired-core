# ESP32-C6-DevKitC-1 + Adafruit BME280

This example attaches the buyable Adafruit BME280 breakout (PID 2652) to the
ESP32-C6-DevKitC-1-N8's modeled I2C0. Its firmware reads the sensor's chip ID
and raw temperature registers through the C6 I2C command-list engine and GPIO
matrix, then reports those bytes over UART0.

ESP32 ranks first in the measured demand list (38%). This is a sensor design
for an already modeled ESP32-C6 board, not a new board or chip SKU.

## Wiring

| Adafruit BME280 breakout | ESP32-C6-DevKitC-1-N8 |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA / SDI | GPIO6 |
| SCK / SCL | GPIO7 |
| CS | Leave unconnected for I2C |
| SDO | Leave at the breakout default |

The LabWired board model exposes GPIO6 as I2C0 SDA and GPIO7 as I2C0 SCL.
The simulation binds the BME280 at `0x77`, Adafruit's breakout default. Power,
pull-up voltage, and physical wiring are outside the simulation.

## Copy-paste Arduino example

With the Arduino ESP32 core and Adafruit BME280 library installed:

```cpp
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin(6, 7);  // SDA, SCL
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }
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

## Reproduce the model smoke

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c6-devkitc-bme280 \
  --crate firmware-esp32c6-bme280-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c6-devkitc-bme280/onboarding-smoke.yaml \
  --system configs/systems/esp32c6-devkitc-bme280.yaml \
  --out-dir examples/esp32c6-devkitc-bme280/evidence
```

The committed logs and simulator outputs are under [`evidence/`](evidence/).
Current vendor status and the availability check are in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

This is a local deterministic Core CLI model smoke. It verifies I2C0 traffic
against the BME280 register model and UART output. The sensor model supplies
factory coefficients and fixed default raw measurements; conversion timing,
oversampling dynamics, filtering, power behavior, and physical GPIO voltage are
not modeled. There is no VDD wire in the simulation; the engine assumes an
ideal supply rail. The hosted verifier does not currently offer an ESP32-C6
compile path, so this local run is not a hosted `model_verified` verdict. This
profile only claims the tested UART0 and routed BME280 I2C path. The board's
built-in RGB LED has separate coverage in
[`esp32c6-devkitc`](../esp32c6-devkitc/README.md).
