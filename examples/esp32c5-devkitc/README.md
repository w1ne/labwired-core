# ESP32-C5-DevKitC-1 v1.2

This focused board profile runs a bare-metal RISC-V smoke on the ESP32-C5 HP
core. Firmware writes `ESP32_C5_UART0_OK` through UART0, reads the BME280 chip
ID and raw temperature ADC over I2C0 on GPIO6/GPIO10, and sends a red GRB
WS2812 frame to the onboard RGB LED on GPIO27. The C5-specific GPIO matrix,
IO_MUX and PCR clock gate are modeled for these paths. UART0 TX/RX are
GPIO11/GPIO12 on J3 pins 2/3.

Demand rank: **ESP32 family #1** in the 2025 JetBrains MCU-use survey (38%).
This extends the existing ESP32-C5-DevKitC-1 v1.2 coverage with a common
environmental sensor breakout.

## Wiring

Connect an Adafruit BME280 breakout (PID 2652) to the ESP32-C5-DevKitC-1:

| BME280 | ESP32-C5-DevKitC-1 |
|---|---|
| VIN | 3V3 |
| GND | GND |
| SDA | GPIO6 (J1 pin 7) |
| SCK/SCL | GPIO10 (J1 pin 11) |

The board guide marks GPIO7 as a strapping pin; this example uses GPIO6 and
GPIO10. The breakout's default I2C address is `0x77`.

## Copy-paste Arduino example

Install the Adafruit BME280 and Adafruit Unified Sensor libraries, then upload:

```cpp
#include <Wire.h>
#include <Adafruit_Sensor.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin(6, 10);  // SDA=GPIO6, SCL=GPIO10

  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }

  Serial.print("Temperature C: ");
  Serial.println(bme.readTemperature());
  Serial.print("Pressure hPa: ");
  Serial.println(bme.readPressure() / 100.0F);
  Serial.print("Humidity %: ");
  Serial.println(bme.readHumidity());
}

void loop() {}
```

## Quick run

From the repository root:

```bash
rustup target add riscv32imc-unknown-none-elf
cargo build -p firmware-esp32c5-devkitc-smoke --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c5-devkitc/onboarding-smoke.yaml \
  --output-dir out/esp32c5-devkitc \
  --no-uart-stdout
```

The committed [`evidence/`](evidence/) directory contains the onboarding
runner's build logs, smoke transcript, simulation output, and metrics. The
fixture checks UART output, the BME280 ID (`0x60`), the modeled raw temperature
sample (`0x082DC0`), GPIO27 edges, and the decoded red LED frame. To run the
bare-metal firmware, use
[`src/main.rs`](../../crates/firmware-esp32c5-devkitc-smoke/src/main.rs) with
the Cargo command above.

This profile does not model a ROM boot, RMT, the other board peripherals,
general-purpose behavior on unlisted GPIO pads, or the USB-UART bridge's
electrical propagation. The repository smoke is local runner evidence; the
external `labwired_verify` service does not yet list ESP32-C5 as a supported
target. See [known limitations](KNOWN_LIMITATIONS.md).
