# Raspberry Pi Pico 2 W + BME280

Demand position: **RP2040/RP2350 target group #2**. The measured MCU survey
reported RP2040 at **rank 5 (12%)**; it did not score RP2350 separately. This
extends the existing RP2350 model to a real Pico 2 W sensor design. It is board
and sensor coverage, not new silicon or wireless coverage.

## Wiring

Connect the Adafruit BME280 STEMMA QT breakout to the Pico 2 W:

| Pico 2 W | BME280 | Purpose |
| --- | --- | --- |
| GP4, physical pin 6 | SDA | I2C0 data |
| GP5, physical pin 7 | SCL | I2C0 clock |
| 3V3, physical pin 36 | VIN | breakout supply |
| GND, physical pin 38 | GND | common ground |

The breakout's default I2C address is 0x77. The system profile attaches a BME280
model to the RP2350's I2C0 controller. It does not model the cable, supply
voltage, pull-up electrical behavior, sensor conversion timing, or environmental
variation.

The onboard user LED is driven by the Pico 2 W's CYW43439 wireless companion
through WL_GPIO0, so this profile does not claim LED output. Wi-Fi, Bluetooth,
and the CYW43439 SPI/IRQ path are also outside the modeled scope.

## Copy-paste Arduino example

Select **Raspberry Pi Pico 2 W** with the Arduino-Pico core. Connect a UART
adapter to GP0 (physical pin 1, TX) and GP1 (physical pin 2, RX) to read the
message.

~~~cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDRESS = 0x77;
constexpr uint8_t CHIP_ID_REGISTER = 0xD0;

void setup() {
  Serial1.begin(115200);  // UART0 on GP0/GP1
  Wire.setSDA(4);         // GP4, physical pin 6
  Wire.setSCL(5);         // GP5, physical pin 7
  Wire.begin();

  Wire.beginTransmission(BME280_ADDRESS);
  Wire.write(CHIP_ID_REGISTER);
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(BME280_ADDRESS, static_cast<uint8_t>(1)) != 1) {
    Serial1.println("BME280_I2C_ERROR");
    return;
  }

  const uint8_t chipId = Wire.read();
  if (chipId == 0x60) {
    Serial1.println("PICO2W_BME280_ID=0x60_I2C0_GP4_GP5");
  } else {
    Serial1.println("BME280_WRONG_ID");
  }
}

void loop() {}
~~~

## Local simulator smoke

The committed smoke is a small RP2350 Cortex-M33 register-level firmware
fixture. It sets GP4/GP5 to I2C0 in IO_BANK0, reads register 0xD0 from the
attached BME280 model, and emits the success marker only when the returned chip
ID is 0x60. The smoke asserts both pin-mux register values. UART0 capture is the
simulator's observation path; electrical levels, the cable, and USB CDC or
wireless behavior are outside the claim.

Run from the repository root:

~~~sh
rustup target add thumbv8m.main-none-eabi
scripts/onboarding_smoke.sh --target-id rpi-pico-2-w-bme280 --crate firmware-pico2w-bme280-smoke --target thumbv8m.main-none-eabi --firmware-build-dir crates/firmware-pico2w-bme280-smoke --script examples/rpi-pico-2-w-bme280/io-smoke.yaml --system configs/systems/rpi-pico-2-w-bme280.yaml --out-dir examples/rpi-pico-2-w-bme280/evidence/run
~~~

Committed simulator output is in [evidence/run](evidence/run/). Vendor
availability is recorded in [vendor-status.md](vendor-status.md).
