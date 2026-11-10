# NUCLEO-F103RB + BME280

**Demand rank:** STM32F103 is **#3** in MCU use at 17%; Nucleo/Discovery kits
are used by 23% of respondents in the standing demand evidence. This extends
the existing NUCLEO-F103RB board profile with a real I²C sensor use case; it is
not a new MCU. ST currently lists the exact MCU and board as Active, with the
board available from ST and distributors. See
[`evidence/vendor-status.md`](evidence/vendor-status.md).

The profile boots bare-metal Cortex-M3 firmware on the exact
STM32F103RBT6-based NUCLEO-F103RB, reads the BME280 chip-ID register through
I2C1, sends the `0x60` result over the board's USART2/ST-LINK virtual COM port,
and toggles the onboard LD2 LED. The default I2C1 pins are PB6/SCL and PB7/SDA.

## Wiring

| BME280 STEMMA QT breakout | NUCLEO-F103RB |
| --- | --- |
| VIN / VCC | 3V3 |
| GND | GND |
| SCL | PB6 / I2C1_SCL |
| SDA | PB7 / I2C1_SDA |

The breakout includes I²C pull-ups and defaults to address `0x77`. For a bare
BME280 sensor, add pull-ups and tie SDO high for `0x77` or low for `0x76`.
Keep the Nucleo and sensor grounds connected.

## Copy-paste Arduino example

Select **NUCLEO-F103RB** with the STM32 Arduino core. This reads the BME280 ID
without requiring a sensor library:

```cpp
#include <Arduino.h>
#include <Wire.h>

TwoWire bmeWire(PB7, PB6);  // SDA, SCL: default I2C1 pins on STM32F103

void setup() {
  Serial.begin(115200);
  bmeWire.begin();
  bmeWire.beginTransmission(0x77);
  bmeWire.write(0xD0);  // BME280 chip-ID register
  if (bmeWire.endTransmission(false) != 0 ||
      bmeWire.requestFrom(0x77, static_cast<uint8_t>(1)) != 1) {
    Serial.println("BME280 not found");
    return;
  }
  Serial.printf("BME280_ID=0x%02X\n", bmeWire.read());
}

void loop() {}
```

The repository smoke uses direct STM32F1 register accesses rather than this
Arduino sketch. The sketch is provided as a starting point for a physical
board; it was not built or run by the local smoke.

## Run the local simulator smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id nucleo-f103rb-bme280 \
  --crate firmware-nucleo-f103rb-bme280-smoke \
  --target thumbv7m-none-eabi \
  --script examples/nucleo-f103rb-bme280/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f103rb-bme280.yaml \
  --out-dir examples/nucleo-f103rb-bme280/evidence
```

The smoke requires the USART2 marker `NUCLEO_F103RB_BME280_ID=0x60`, four
transitions on LD2/PA5, and the requested simulation stop. The committed logs
and structured result are in [`evidence/`](evidence/). This is local twin
evidence, not a physical-board run. The hosted verifier catalog has no exact
NUCLEO-F103RB target, so no hosted `model_verified` verdict is claimed.

## Scope

- Exercised: STM32F103RBT6 reset-clock startup, GPIOA/PA5, GPIOB/PB6-PB7
  configuration, I2C1 controller, BME280 ID read, USART2 output, and LD2.
- Not claimed: full temperature/pressure/humidity compensation, physical
  sensor parity, I2C pin waveform parity, other I2C transactions, B1 input,
  timers, ADC, DMA, SPI, USB, CAN, clock-tree switching, or physical hardware
  behavior.
- The profile excludes the family descriptor's zero-answering USB and
  backup-register windows.
