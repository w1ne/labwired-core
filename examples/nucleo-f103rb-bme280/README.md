# NUCLEO-F103RB + BME280

**Demand position:** STM32F103 ranks #3 in the measured MCU list (17%); Nucleo/Discovery kits appear in 23% of projects. This adds a sensor path to the existing NUCLEO-F103RB support; it is not a new MCU family.

The smoke boots an STM32F103RBT6 image, reads the BME280 chip ID (`0x60`) on I2C1 at address `0x77`, writes the observed result to the ST-LINK virtual COM port, and pulses the board's active-high LD2 only after the expected ID is read.

## Wiring

| Adafruit BME280 breakout | NUCLEO-F103RB |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SCL | Arduino D15 / PB8 / I2C1_SCL |
| SDA | Arduino D14 / PB9 / I2C1_SDA |
| SDO | Leave at the breakout default (address `0x77`) |

The firmware enables AFIO's I2C1 remap so I2C1 uses PB8/PB9. The breakout includes level shifting and pull-ups. The simulator connects the BME280 model to I2C1; it does not simulate voltage, pull-up resistance, or physical wiring.

## Copy-paste Arduino example

Install the Adafruit BME280 library and Adafruit Unified Sensor dependency, select **NUCLEO-F103RB**, and upload:

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

## Reproduce the committed simulator smoke

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-f103rb-bme280 \
  --crate firmware-nucleo-f103rb-bme280-smoke \
  --target thumbv7m-none-eabi \
  --script examples/nucleo-f103rb-bme280/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f103rb-bme280.yaml \
  --out-dir examples/nucleo-f103rb-bme280/evidence
```

The committed evidence includes the CLI and firmware build logs, smoke output, structured verdict, and onboarding metrics.

Recorded result: **PASS, 3/3 assertions**, 400,000 simulation steps. The UART
captured `NUCLEO_F103RB_BME280_OK ID=0x60 I2C1=PB8/PB9`; LD2 exceeded the
32-edge minimum after the read succeeded.

## Model boundary

The profile declares RCC, AFIO, GPIOA, GPIOB, USART2, I2C1, and the Cortex-M3 SysTick core timer. The PR-run core-timer differential loads this exact profile and compares SysTick interrupt execution with the legacy walk; it does not extend the sensor smoke claims. It exercises the USART2 VCP, LD2, and one BME280 `CHIP_ID` register read through the board's PB8/PB9 route. The profile does not claim the Nucleo's button or the other STM32F103 peripherals. The BME280 model provides deterministic register responses; environmental conversion timing and changes in temperature, pressure, and humidity are outside this smoke.

## Vendor status and source documents

- [Live vendor status evidence](evidence/vendor-status.md)
- [ST NUCLEO-F103RB product page](https://www.st.com/en/evaluation-tools/nucleo-f103rb.html)
- [ST UM1724 Nucleo-64 board manual](https://www.st.com/resource/en/user_manual/um1724-stm32-nucleo64-boards-mb1136-stmicroelectronics.pdf)
- [ST STM32F103RB product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f103rb.html)
- [Adafruit BME280 breakout, PID 2652](https://www.adafruit.com/product/2652)
