# ESP32-C5-DevKitC-1 v1.2

This focused board profile runs a bare-metal RISC-V smoke on the ESP32-C5 HP
core. Firmware writes `ESP32_C5_UART0_OK` through UART0, reads a BME280 over
I2C0, paints an SSD1306 OLED on the same I2C bus, and sends a red GRB WS2812
frame through RMT TX0 to the onboard RGB LED on GPIO27. The BME280 and OLED
share GPIO6/GPIO10. The C5-specific GPIO matrix, IO_MUX, PCR clock controls,
I2C transactions, and timed RMT TX path are modeled for these paths. UART0
TX/RX are GPIO11/GPIO12 on J3 pins 2/3.

Demand rank: **ESP32 family #1** in the 2025 JetBrains MCU-use survey (38%).
This round adds an SSD1306 I2C display to an existing DevKitC profile. It is
coverage depth in the top-ranked family, not a new MCU or board demand pick.

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

Connect an Adafruit SSD1306 OLED breakout (PID 326) to the same bus. Leave its
interface jumpers in their default I2C configuration. It uses address `0x3C`.

| SSD1306 | ESP32-C5-DevKitC-1 |
|---|---|
| VIN | 3V3 |
| GND | GND |
| SDA | GPIO6 (J1 pin 7) |
| SCL | GPIO10 (J1 pin 11) |

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

For the SSD1306, install Adafruit SSD1306 and Adafruit GFX, then upload:

```cpp
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>
#include <Wire.h>

Adafruit_SSD1306 display(128, 64, &Wire, -1);

void setup() {
  Wire.begin(6, 10);  // SDA=GPIO6, SCL=GPIO10; shared with the BME280
  if (!display.begin(SSD1306_SWITCHCAPVCC, 0x3C)) return;
  display.clearDisplay();
  display.setTextSize(1);
  display.setTextColor(SSD1306_WHITE);
  display.setCursor(0, 0);
  display.println("ESP32-C5");
  display.println("BME280 ready");
  display.display();
}

void loop() {}
```

For the onboard RGB LED, install the Espressif Arduino core for ESP32-C5-DevKitC-1
and upload this sketch:

```cpp
#include <Arduino.h>

void setup() {
  rgbLedWrite(27, 64, 0, 0);
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
fixture checks UART output, BME280 ID (`0x60`) and raw temperature
(`0x082DC0`), an SSD1306 paint over the shared I2C bus, RMT TX completion,
GPIO27 edges, and the decoded red LED frame. The committed simulator smoke
uses a register-level bare-metal SSD1306 driver; the Arduino snippet is a
copy-paste starting point and was not compiled by that run. To run the firmware,
use
[`src/main.rs`](../../crates/firmware-esp32c5-devkitc-smoke/src/main.rs) with
the Cargo command above.

This profile does not model a ROM boot, RMT receive/carrier/DMA, the other
board peripherals, general-purpose behavior on unlisted GPIO pads, or the
USB-UART bridge's electrical propagation. The repository smoke is local runner
evidence; the external `labwired_verify` service does not yet list ESP32-C5 as a
supported target. See [known limitations](KNOWN_LIMITATIONS.md).
