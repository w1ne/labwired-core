# Raspberry Pi Pico W + BME280

**Demand position:** RP2040/RP2350 target group #2 in the demand-ordered
backlog. RP2040 was survey rank 5 at 12% of reported MCU use; RP2350 was not
scored separately. This adds an exact, current Pico W board and sensor path to
the existing RP2040 model.

## Wiring

Connect an Adafruit BME280 STEMMA QT breakout to the Pico W:

| Pico W | BME280 breakout | Purpose |
| --- | --- | --- |
| GP4, header pin 6 | SDA | I2C0 data |
| GP5, header pin 7 | SCL | I2C0 clock |
| 3V3(OUT), header pin 36 | VIN | Breakout supply |
| GND, header pin 38 | GND | Common ground |
| GP0, header pin 1 | USB-to-UART adapter RX | Smoke console |
| GND, header pin 3 | USB-to-UART adapter GND | Console reference |

The breakout is set to address `0x77`. The model reads its BME280 chip ID
register `0xD0` and requires the returned value `0x60`.

## Copy-paste Arduino-Pico example

Select **Raspberry Pi Pico W** in the Arduino-Pico core. Connect the BME280 as
above and use a 3.3 V USB-to-UART adapter on GP0 to view the result.

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDRESS = 0x77;

void setup() {
  Serial1.setTX(0);
  Serial1.setRX(1);
  Serial1.begin(115200);  // UART0 TX on GP0, RX on GP1
  Wire.setSDA(4);         // I2C0 SDA on GP4, header pin 6
  Wire.setSCL(5);         // I2C0 SCL on GP5, header pin 7
  Wire.begin();

  Wire.beginTransmission(BME280_ADDRESS);
  Wire.write(0xD0);  // BME280 chip-ID register
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(BME280_ADDRESS, static_cast<uint8_t>(1)) != 1) {
    Serial1.println("PICO_W_BME280_I2C_ERROR");
    return;
  }

  if (Wire.read() == 0x60) {
    Serial1.println("PICO_W_BME280_ID=0x60_I2C0_GP4_GP5");
  } else {
    Serial1.println("PICO_W_BME280_WRONG_ID");
  }
}

void loop() {}
```

## Build and smoke

From the repository root:

```sh
rustup target add thumbv6m-none-eabi
examples/pico-w-bme280/ci/test.sh
```

The local smoke passed **3/3 assertions at 100,259 steps**. It compiles a
Cortex-M0+ image, fast-boots the firmware's named register-level entry, routes
GP4/GP5 to I2C0, reads the BME280 ID, and prints its success marker through
the UART0 model. It skips the mask ROM and stage-2 boot2 sequence. The smoke
asserts the marker and both I2C pin-mux values. Logs and structured output are
committed under [`evidence/run/`](evidence/run/).

The board profile covers RP2040, I2C0 GP4/GP5, the BME280 ID transaction, and
UART0. It does not claim Wi-Fi, Bluetooth, the CYW43439 radio path, the onboard
LED routed through WL_GPIO0, USB, physical board behavior, or environmental
sensor accuracy. Raspberry Pi's Pico W board lifetime and live stock check are
recorded in [`vendor-status.md`](vendor-status.md).
