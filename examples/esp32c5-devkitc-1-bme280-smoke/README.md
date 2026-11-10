# ESP32-C5-DevKitC-1-N8R8 BME280 I2C smoke

This example boots a small bare-metal RV32IMAC firmware image on the modeled
ESP32-C5 HP core, reads the BME280 chip ID (`0xD0`) and raw temperature
(`0xFA`–`0xFC`) over I2C0 on GPIO8/9, checks the expected values, and reports
a success marker through UART0.
GPIO5 also toggles through the GPIO output registers. On the ESP32-C5-DevKitC-1
v1.2 board, UART0 TX is J3 pin 2 and GPIO5 is J3 pin 9.

This is a focused new-chip smoke, not a full C5 system model. The ELF is loaded
directly into HP SRAM by the simulator; it does not use the C5 mask ROM, flash
boot path, or PSRAM. The I2C controller executes the C5 command list through
the GPIO8/9 matrix route and the BME280 model supplies ID and temperature
registers. The UART assertion observes bytes from the C5 UART FIFO model
through the host console sink; UART pin routing and PCR clock gating are not
enforced. The GPIO assertion observes model edges, not package voltage.

## Build and run

```sh
rustup target add riscv32imac-unknown-none-elf
cargo build -p firmware-esp32c5-devkitc-1-bme280-smoke \
  --release --target riscv32imac-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c5-devkitc-1-bme280-smoke/io-smoke.yaml \
  --output-dir out/esp32c5-devkitc-1-n8r8-bme280 --no-uart-stdout
```

The assertions require the BME280 ID and raw-temperature UART marker and four
or more transitions on `gpio:5`. For a physical bench check, connect a 3.3 V
BME280 breakout to GPIO8 (SDA), GPIO9 (SCL), 3V3, and GND; these pins are J1
headers 9 and 10. GPIO7 is a strapping pin and is intentionally avoided.
UART0 TX is available on J3 pin 2 (U0TXD). GPIO5 is J3 pin 9 and can drive an
external LED through a current-limiting resistor. The onboard addressable RGB
LED is on GPIO27 and is not part of this smoke.

See [`VALIDATION.md`](VALIDATION.md) for the claim boundary and committed
evidence, and [`vendor-status.md`](vendor-status.md) for the current buyability
check.

## Copy-paste Arduino example

On the physical board, `Serial` is UART0 and TX appears at J3 pin 2 (U0TXD).
This copy-paste Arduino example reads the BME280 at the default address `0x77`
using GPIO8/9:

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kBme280Address = 0x77;

bool readRegisters(uint8_t reg, uint8_t *data, size_t length) {
  Wire.beginTransmission(kBme280Address);
  if (Wire.write(reg) != 1 || Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(kBme280Address, static_cast<uint8_t>(length), true) != length)
    return false;
  for (size_t i = 0; i < length; ++i) data[i] = Wire.read();
  return true;
}

void setup() {
  Serial.begin(115200);
  Wire.begin(8, 9, 100000);  // SDA, SCL, 100 kHz
  uint8_t id = 0;
  uint8_t temp[3] = {};
  if (!readRegisters(0xD0, &id, 1) || id != 0x60 ||
      !readRegisters(0xFA, temp, sizeof(temp))) {
    Serial.println("C5_BME280_FAIL");
    return;
  }
  const uint32_t raw = (uint32_t(temp[0]) << 12) |
                       (uint32_t(temp[1]) << 4) | (temp[2] >> 4);
  Serial.printf("C5_BME280_OK ID=0x%02X TEMP_RAW=0x%06lX\n",
                id, (unsigned long)raw);
}

void loop() { delay(1000); }
```

This sketch is for a physical ESP32-C5 board. The committed simulator smoke
uses bare-metal firmware and tests BME280 I2C register reads, UART FIFO output,
and GPIO5 register edges; it does not boot Arduino/ESP-IDF or enforce U0TXD pad
routing.
