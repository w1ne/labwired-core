# STM32F103C8T6 Blue Pill + BME280

**Demand rank:** STM32F103 is rank **#3** in MCU usage (17%) and the Blue Pill
is explicitly named in the demand list. This adds a BME280 I2C path to the
existing Blue Pill board profile; it reuses the already-modelled STM32F103C8
and BME280 and is maintenance-depth work, not a new-vendor bet.

## Wiring

Use an Adafruit BME280 STEMMA QT breakout or a compatible BME280 I2C module.
The module's default address should be `0x77`.

| BME280 breakout | Blue Pill |
| --- | --- |
| VIN / 3V | 3V3 |
| GND | GND |
| SDA | PB7 / I2C1_SDA |
| SCL | PB6 / I2C1_SCL |

For a physical UART console, connect a **3.3 V** USB-serial adapter's RX to PA9
and connect grounds. USART1 RX is PA10. The smoke captures USART1 through the
simulator console; it does not model a USB-UART bridge.

## Copy-paste Arduino example

Install the Adafruit BME280 and Adafruit Unified Sensor libraries, select a
STM32F103C8T6 Blue Pill board in STM32duino, then upload:

```cpp
#include <Arduino.h>
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial1.begin(115200);       // USART1: TX=PA9, RX=PA10
  Wire.setSDA(PB7);
  Wire.setSCL(PB6);
  Wire.begin();                // I2C1 default pins
  pinMode(PC13, OUTPUT);       // Blue Pill user LED, active low

  if (!bme.begin(0x77, &Wire)) {
    Serial1.println("BME280 not found");
    while (true) delay(10);
  }
  Serial1.println("BLUEPILL_F103C8_BME280_OK");
}

void loop() {
  digitalWrite(PC13, LOW);
  Serial1.print("Temperature C: ");
  Serial1.println(bme.readTemperature());
  delay(500);
  digitalWrite(PC13, HIGH);
  delay(500);
}
```

## Run the simulator smoke

```sh
examples/stm32f103-blue-pill-bme280/ci/build.sh
examples/stm32f103-blue-pill-bme280/ci/test.sh
```

The committed smoke firmware reads `BME280.CHIP_ID` (`0xD0`), requires `0x60`,
prints `BLUEPILL_F103C8_BME280_OK ID=0x60 I2C1=PB6/PB7` on USART1, and only then
pulses the active-low PC13 LED for 20 cycles. The simulator evidence is under
[`evidence/`](evidence/).

## Modeled scope

The narrow chip profile declares SysTick, RCC, GPIOA/B/C, USART1, and I2C1. The
system attaches a real BME280 model at `0x77`. The smoke verifies the I2C chip-ID
transaction, USART1 output, and PC13 edges. It does not simulate the board's USB
device/bootloader, USB-to-serial conversion, SWD hardware, analog behavior, or
differences among Blue Pill clones. Sensor temperature, pressure, and humidity
conversions are outside this smoke.

See [`vendor-status.md`](vendor-status.md) for the live lifecycle and purchase
checks.
