# Raspberry Pi Pico 2 + BME280

**Demand position:** RP2040/RP2350 is the demand-ordered target group **#2**;
RP2040 ranked 5 at 12% in the measured MCU survey. The survey did not score
RP2350 separately. This is a tested board-variant addition over the existing
RP2350 model, not new silicon coverage.

## Wiring

Connect an I2C BME280 breakout at address `0x77`:

| BME280 breakout | Raspberry Pi Pico 2 | Signal |
|---|---|---|
| SDA | GP4, header pin 6 | I2C0 SDA |
| SCL | GP5, header pin 7 | I2C0 SCL |
| VIN | 3V3(OUT), header pin 36 | Supply |
| GND | GND, header pin 38 | Ground |

Use a breakout with I2C pull-ups or add pull-ups to 3V3. The simulator does
not model the cable, electrical pull-ups, supply rail, or sensor conversion
timing. The non-W Pico 2 has no CYW43439 wireless companion.

## Copy-paste Arduino-Pico sketch

Select **Raspberry Pi Pico 2** in the Arduino-Pico core. Connect a USB-to-UART
adapter to GP0 (header pin 1, TX) and GP1 (header pin 2, RX) to view the result.

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDRESS = 0x77;

void setup() {
  Serial1.begin(115200);  // UART0 on GP0/GP1
  Wire.setSDA(4);         // GP4, header pin 6
  Wire.setSCL(5);         // GP5, header pin 7
  Wire.begin();

  Wire.beginTransmission(BME280_ADDRESS);
  Wire.write(0xD0);  // BME280 CHIP_ID register
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(BME280_ADDRESS, static_cast<uint8_t>(1)) != 1) {
    Serial1.println("PICO2_BME280_I2C_ERROR");
    return;
  }

  const uint8_t chipId = Wire.read();
  if (chipId == 0x60) {
    Serial1.println("PICO2_BME280_ID=0x60_I2C0_GP4_GP5");
  } else {
    Serial1.println("PICO2_BME280_WRONG_ID");
  }
}

void loop() {}
```

## Local simulator smoke

The committed bare-metal RP2350 fixture selects I2C0 on GP4/GP5, performs a
repeated-start read of BME280 `CHIP_ID` (`0xD0`), checks for `0x60`, and prints
the success marker over UART0. The smoke also asserts both IO_BANK0 function
selection values.

```sh
rustup target add thumbv8m.main-none-eabi
examples/pico2-bme280/ci/test.sh
```

Committed local simulator logs and structured output are under
[`evidence/run/`](evidence/run/). Live board status is in
[`vendor-status.md`](vendor-status.md).

## Scope

This profile covers the Pico 2 RP2350 firmware path, UART0 output, I2C0 over
GP4/GP5, and the BME280's modeled ID register. It fast-loads the firmware and
does not run the mask ROM or bootloader. It does not claim USB, boot selection,
wireless, physical sensor accuracy, electrical behavior, or operation of the
complete hardware board. The shared RP2350 descriptor still has storage-only
stubs for `powman` and `tbman`; this smoke does not access them.
