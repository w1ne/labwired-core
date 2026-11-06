# ESP32-C3-DevKit-RUST-2

**Demand rank:** ESP32 is rank 1 in the JetBrains State of Embedded survey
(38% reported use). This round adds the board's onboard SHTC3 sensor path; the
survey does not rank this exact SKU. Live vendor and distributor availability
is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

The onboarding smoke runs a Rust/RISC-V ELF through the local simulator. It
bit-bangs a red WS2812B frame on GPIO2, then reads the onboard SHTC3 over the
modeled C3 I2C0 controller (SDA GPIO10, SCL GPIO8), checking both CRC bytes.
The profile adds the SHTC3 command protocol, sensor stimuli, and conversion
timing. The onboard ICM-42670-P IMU, button, plain user LED, radio, USB, and
power circuitry remain outside this profile.

## Copy-paste Arduino example

Install the Adafruit NeoPixel library and upload this sketch to the board:

```cpp
#include <Adafruit_NeoPixel.h>

Adafruit_NeoPixel led(1, 2, NEO_GRB + NEO_KHZ800);

void setup() {
  led.begin();
  led.setPixelColor(0, led.Color(255, 0, 0));
  led.show();
}

void loop() {}
```

The sketch demonstrates the same GPIO2 data path exercised by the simulator
smoke. It does not claim that the simulator implements the Arduino runtime.

## Copy-paste SHTC3 example

The SHTC3 is at I2C address `0x70`, with SDA on GPIO10 and SCL on GPIO8.
This sketch wakes the sensor, requests a low-power measurement, validates both
CRC bytes, and prints the decoded temperature and humidity:

```cpp
#include <Wire.h>

constexpr uint8_t SHTC3_ADDR = 0x70;

uint8_t crc8(uint8_t msb, uint8_t lsb) {
  uint8_t crc = 0xFF;
  const uint8_t data[2] = {msb, lsb};
  for (uint8_t i = 0; i < 2; ++i) {
    crc ^= data[i];
    for (uint8_t bit = 0; bit < 8; ++bit)
      crc = (crc & 0x80) ? (crc << 1) ^ 0x31 : crc << 1;
  }
  return crc;
}

bool command(uint16_t word) {
  Wire.beginTransmission(SHTC3_ADDR);
  Wire.write(word >> 8);
  Wire.write(word & 0xFF);
  return Wire.endTransmission() == 0;
}

void setup() {
  Serial.begin(115200);
  Wire.begin(10, 8); // SDA, SCL

  if (!command(0x3517)) return; // Wake-up
  delayMicroseconds(300);
  if (!command(0x609C)) return; // Low-power, temperature first
  delay(1);                     // Datasheet maximum is 0.8 ms
  if (Wire.requestFrom(SHTC3_ADDR, uint8_t(6)) != 6) return;

  uint8_t data[6];
  for (uint8_t &byte : data) byte = Wire.read();
  if (crc8(data[0], data[1]) != data[2] ||
      crc8(data[3], data[4]) != data[5]) return;

  uint16_t raw_t = (uint16_t(data[0]) << 8) | data[1];
  uint16_t raw_rh = (uint16_t(data[3]) << 8) | data[4];
  float temperature = -45.0f + 175.0f * raw_t / 65536.0f;
  float humidity = 100.0f * raw_rh / 65536.0f;
  Serial.printf("T=%.2f C RH=%.2f%%\n", temperature, humidity);
}

void loop() {}
```

## Run the simulator smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32c3-devkit-rust-2 \
  --crate firmware-esp32c3-rust2-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c3-devkit-rust-2/onboarding-smoke.yaml \
  --system configs/systems/esp32c3-devkit-rust-2.yaml \
  --out-dir examples/esp32c3-devkit-rust-2/evidence
```

The smoke requires the firmware to complete a CRC-checked SHTC3 read, at least
48 GPIO2 edges and a decoded red pixel (`RGB565 0xF800`), then stops at the
configured instruction limit. Committed logs and metrics are under
[`evidence/`](evidence/). This is repository simulator evidence, not a
physical-board test. The profile's scope and omitted hardware are listed in
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).
