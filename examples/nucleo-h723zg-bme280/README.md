# NUCLEO-H723ZG with BME280

**Demand position:** STM32 usage ranks #4 in the measured list as “other
STM32” (13%); Nucleo/Discovery kits appear in 23% of projects. The demand
brief specifically calls for newer professional H7 Nucleo boards. This adds a
real I2C sensor path to the NUCLEO-H723ZG, not another chip variant.

The model smoke boots firmware for the board's STM32H723ZGT6, reads the BME280
chip ID (`0x60`) over I2C1 on PB8/PB9, reports the observed byte on USART3, and
pulses the board's active-high LD1 on PB0 only after a successful read.

## Wiring

| Adafruit BME280 breakout | NUCLEO-H723ZG |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SCL | Arduino D15 / PB8 / I2C1_SCL |
| SDA | Arduino D14 / PB9 / I2C1_SDA |
| SDO | Leave at the breakout default (address `0x77`) |

The Adafruit breakout includes level shifting and pull-ups. The simulator
attaches the BME280 model to the I2C1 bus; it does not simulate supply voltage,
pull-up resistance, or physical wiring.

## Copy-paste Arduino example

Install the Adafruit BME280 library and its Adafruit Unified Sensor dependency,
select **NUCLEO-H723ZG**, and upload this sketch. The board variant maps the
default `Wire` bus to I2C1 on PB8/PB9.

```cpp
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin();
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }
  Serial.println("NUCLEO_H723ZG_BME280_OK");
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

## Reproduce the committed model smoke

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-h723zg-bme280 \
  --crate firmware-nucleo-h723zg-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-h723zg-bme280/onboarding-smoke.yaml \
  --system configs/systems/nucleo-h723zg-bme280.yaml \
  --out-dir examples/nucleo-h723zg-bme280/evidence
```

The evidence directory contains the CLI build log, firmware build log,
simulator log, structured result, and onboarding metrics. Availability checks
are recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

This profile exposes the tested RCC, GPIOB/GPIOD, USART3, and I2C1 path. It
models the NUCLEO-H723ZG header mapping D15/PB8 as SCL and D14/PB9 as SDA, the
board LD1 on PB0, and the ST-LINK virtual COM route on USART3 PD8/PD9. The
BME280 model returns its documented ID and deterministic default measurements;
conversion timing and environmental dynamics are not simulated. This is local
simulator evidence, not a hosted `model_verified` verdict or physical-board
confirmation.

## Availability and source documents

- [ST NUCLEO-H723ZG product page](https://www.st.com/en/evaluation-tools/nucleo-h723zg.html)
- [ST UM2407 Nucleo-144 board manual](https://www.st.com/resource/en/user_manual/um2407-stm32h7-nucleo144-board-stmicroelectronics.pdf)
- [ST STM32H723ZGT6 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32h723zg.html)
- [Bosch BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
- [Adafruit BME280 breakout, PID 2652](https://www.adafruit.com/product/2652)
