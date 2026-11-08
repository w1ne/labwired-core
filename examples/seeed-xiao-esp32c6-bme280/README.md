# Seeed Studio XIAO ESP32-C6 + BME280

This narrow board profile runs bare-metal firmware on the XIAO ESP32-C6 and
reads the BME280 chip ID and raw temperature over I2C0. It uses the board's
documented D4/SDA=GPIO22 and D5/SCL=GPIO23 pins, then reports the observed
register bytes through UART0.

Demand rank: **ESP32 family #1**, used by 38% of respondents in the 2025
JetBrains State of Embedded survey cited in the lane's demand-ordering brief.
This adds sensor coverage to an existing C6 model; it is not a new MCU family.

## Wiring

| BME280 breakout | XIAO ESP32-C6 |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA / SDI | D4 / GPIO22 |
| SCK / SCL | D5 / GPIO23 |
| CS | Leave unconnected for I2C |
| SDO | Leave at the breakout default (`0x77`) |

The sensor profile assumes the breakout's normal I2C address and ideal power
and pull-ups. It does not model analog voltage, sensor conversion timing,
oversampling, filtering, or power behavior.

## Copy-paste Arduino example

Install the Arduino ESP32 core and Adafruit BME280 library, then upload:

```cpp
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin(22, 23);  // XIAO D4/SDA, D5/SCL
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }
  Serial.print("Temperature C: ");
  Serial.println(bme.readTemperature());
}

void loop() {}
```

## Reproduce the smoke

From the repository root:

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id seeed-xiao-esp32c6-bme280 \
  --crate firmware-xiao-esp32c6-bme280-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/seeed-xiao-esp32c6-bme280/onboarding-smoke.yaml \
  --system configs/systems/seeed-xiao-esp32c6-bme280.yaml \
  --out-dir examples/seeed-xiao-esp32c6-bme280/evidence
```

The smoke checks both the BME280 ID (`0x60`) and its raw temperature value
(`0x082DC0`) through the local deterministic I2C model. This is simulator
evidence, not a physical-board test or a hosted `labwired_verify` verdict.

Generate the per-target evidence scoreboard with:

```bash
python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/seeed-xiao-esp32c6-bme280/evidence \
  --markdown-out examples/seeed-xiao-esp32c6-bme280/evidence/onboarding-scoreboard.md \
  --json-out examples/seeed-xiao-esp32c6-bme280/evidence/onboarding-scoreboard.json
```

The profile loads only UART0, GPIO, I2C0, the C6 PCR clock gates, and required
memory. Wi-Fi/Bluetooth/802.15.4, USB, the LP core, SPI, ADC, and the onboard
USER LED are outside this board profile. See the existing
[`GPIO-only XIAO profile`](../seeed-xiao-esp32c6/README.md) for LED coverage.
