# Pico 2 (RP2350) Onboarding Example

Run all commands from `core/`.

## Purpose

Deterministic bring-up for Raspberry Pi Pico 2 (RP2350, dual Cortex-M33,
4 MB XIP flash @ `0x10000000`, 520 KB SRAM @ `0x20000000`) reusing the
RP2040 behavioural models with the `rp2350` clock/reset profile (moved
APB map from rp2350 addressmap.h).

## Quick Run

```bash
rustup target add thumbv8m.main-none-eabi   # once
RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-rp2350-demo \
  --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/pico2/uart-smoke.yaml \
  --output-dir out/pico2/uart-smoke \
  --no-uart-stdout
```

Expected result:
1. smoke test passes (exit 0)
2. UART contains `RP2350_SMOKE_OK`

## Pico 2 + BME280 sensor smoke

The board profile [`configs/systems/rp2350-pico2-bme280.yaml`](../../configs/systems/rp2350-pico2-bme280.yaml)
connects an Adafruit BME280 STEMMA QT breakout to I2C0 at address `0x77`.
On the Pico 2 header, GP4 is I2C0 SDA and GP5 is I2C0 SCL. The committed
smoke firmware reads BME280 `CHIP_ID` (`0xD0` → `0x60`) and the raw temperature
sample (`0xFA..0xFC` → `0x082DC0`) through the RP2350 I2C0 controller model.

Reproduce the full source build and simulator smoke:

```bash
rustup target add thumbv8m.main-none-eabi   # once
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id rp2350-pico2-bme280 \
  --crate firmware-rp2350-pico2-bme280-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/pico2/bme280-smoke.yaml \
  --system configs/systems/rp2350-pico2-bme280.yaml \
  --out-dir examples/pico2/evidence/bme280
```

The committed local evidence is in
[`evidence/bme280/`](evidence/bme280/); the CLI reported 4/4 assertions,
including the exact BME280 ID and raw sample, plus four GP25 LED transitions
after the reads. This is simulator evidence, not a physical-board test. The
UART text is diagnostic output from the simulator capture; the bare-metal smoke
does not configure a physical UART. The profile attaches the BME280 model
directly to I2C0; the RP2350 model does not electrically enforce the GP4/GP5 pad
route or simulate pull-ups and voltage levels.

## Wiring and copy-paste Arduino sketch

| Adafruit BME280 breakout | Raspberry Pi Pico 2 |
| --- | --- |
| VIN | 3V3(OUT), pin 36 |
| GND | GND, pin 38 |
| SDA | GP4, pin 6 / I2C0 SDA |
| SCL | GP5, pin 7 / I2C0 SCL |

Use the Adafruit BME280 library and Adafruit Unified Sensor dependency with
the **Raspberry Pi Pico 2** board selected in the Earle Philhower Arduino-Pico
core:

```cpp
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.setSDA(4);
  Wire.setSCL(5);
  Wire.begin();
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }
  Serial.println("PICO2_BME280_OK");
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

## Files

1. `system.yaml`: base Pico 2 profile (LED on GP25 via SIO).
2. `bme280-smoke.yaml`: deterministic RP2350 I2C0 + BME280 and UART smoke.
3. `REQUIRED_DOCS.md`: source-grounding references.
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
