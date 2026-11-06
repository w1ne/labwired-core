# ESP32-DevKitC-32E GPIO, UART0, and BME280 I2C smokes

**Demand rank:** ESP32 is rank 1 in the standing list (38% of reported MCU
use). This adds a real sensor path to the already-modelled ESP32-DevKitC-32E;
it is peripheral depth on the highest-demand family, not a new MCU target.

## Board and modeled scope

The board is Espressif's exact **ESP32-DevKitC-32E** with an ESP32-WROOM-32E
module and 4 MB flash. The narrow board profile exposes the classic ESP32 GPIO,
UART0 console, and I2C0 models. GPIO23 maps to J3 pin 2; it is a header pin,
not a built-in LED. For a physical blink, connect an LED in series with a
resistor from GPIO23 to GND.

| Board connection | MCU behavior | Smoke |
| --- | --- | --- |
| J3 pin 2 | GPIO23 | Base smoke toggles the output and requires at least 16 edges |
| UART0 output | Serial console capture | Base smoke asserts `ESP32_DEVKITC32E_UART0_OK` |
| J2 GPIO21/GPIO22 | I2C0 SDA/SCL | BME280 smoke reads sensor ID and temperature ADC data |

The BME280 breakout uses the standard `0x77` address and is powered directly
from board 3V3/GND. Firmware reads ID register `0xD0` (`0x60`) and the raw
temperature registers `0xFA..0xFC` (`0x082DC0` in the deterministic model).
The breakout, rather than a built-in board component, is declared as an
external device in the BME280 system manifest.

The UART0 controller is modeled. The board's USB-to-UART bridge and the
pin-level UART TX/RX waveforms are not, so console capture does not prove USB
serial behavior or a signal at the board header. Wi-Fi/Bluetooth, SPI, timers,
flash boot behavior, and the 5 V power LED are outside these profiles. The
classic ESP32 builder still has zero-answering windows for Wi-Fi MAC/PHY,
I2S0/1, UHCI0, RMT, and PCNT, along with APB_CTRL and the DPORT analog-AHB
fallbacks. The board profiles do not claim those blocks.

## Copy-paste firmware example

This complete Arduino sketch reads the chip ID and raw temperature registers
from an Adafruit BME280 breakout on GPIO21/GPIO22:

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kBme280Address = 0x77;

bool readRegisters(uint8_t first, uint8_t *bytes, uint8_t count) {
  Wire.beginTransmission(kBme280Address);
  Wire.write(first);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(kBme280Address, count) != count) return false;
  for (uint8_t i = 0; i < count; ++i) bytes[i] = Wire.read();
  return true;
}

bool writeRegister(uint8_t reg, uint8_t value) {
  Wire.beginTransmission(kBme280Address);
  Wire.write(reg);
  Wire.write(value);
  return Wire.endTransmission() == 0;
}

void setup() {
  Serial.begin(115200);
  Wire.begin(21, 22, 100000);  // SDA=GPIO21, SCL=GPIO22

  uint8_t id = 0;
  if (!readRegisters(0xD0, &id, 1) || id != 0x60) {
    Serial.println("BME280 not found");
    return;
  }

  // Enable humidity first, then temperature/pressure oversampling and normal mode.
  if (!writeRegister(0xF2, 0x01) || !writeRegister(0xF4, 0x27)) {
    Serial.println("BME280 setup failed");
    return;
  }
  delay(100);

  uint8_t temperature[3] = {};
  if (!readRegisters(0xFA, temperature, sizeof(temperature))) {
    Serial.println("BME280 read failed");
    return;
  }
  const uint32_t raw = (static_cast<uint32_t>(temperature[0]) << 12) |
                       (static_cast<uint32_t>(temperature[1]) << 4) |
                       (temperature[2] >> 4);
  Serial.printf("BME280_TEMP_RAW=0x%06lX\n", static_cast<unsigned long>(raw));
}

void loop() {}
```

The local acceptance firmware is
[`firmware/src/bme280_smoke.cpp`](firmware/src/bme280_smoke.cpp). It programs
the classic ESP32 I2C0 command registers directly and reads the same ID and
temperature registers. The Arduino ESP32 3.3.12 `Wire` driver stalls after its
first split-command `END_DETECT` boundary in the current simulator, so the
local smoke does not claim to verify that driver path.

## Reproduce

From the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-bme280 \
  --script examples/esp32-devkitc-32e/bme280-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-bme280.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/bme280 \
  --firmware-build-script scripts/build-esp32-devkitc-32e-bme280.sh
```

The build log, local simulator output, and generated metrics are committed
under `evidence/bme280/`. This is deterministic digital-twin evidence, not a
physical-board run. The classic ESP32 hosted verifier does not support its
serial oracle; the committed local CLI smoke is the acceptance evidence.

The current lifecycle and stock check is in
[`evidence/vendor-status.md`](evidence/vendor-status.md). The evidence records
why the exact board and WROOM-32E variant satisfy the no-obsolete-parts rule.
