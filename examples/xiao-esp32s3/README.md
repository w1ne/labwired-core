# Seeed Studio XIAO ESP32-S3

Demand rank: **ESP32 #1 (38%)** in the JetBrains State of Embedded survey.
This adds board coverage for the existing ESP32-S3 model; it is not a new chip
family.

The profile starts Arduino firmware at its ELF entry point, writes a marker
over USB CDC, and toggles the XIAO's built-in active-low user LED on GPIO21.
The board's 8 MiB flash is represented. The physical ESP32-S3R8 has 8 MiB of OPI PSRAM,
but this profile builds in QSPI-only mode and excludes that memory window.
Camera, microphone, SD, Wi-Fi, BLE, battery charging, charge LED, and
reset/boot buttons are outside the tested profile.

## Pin map

| XIAO label | ESP32-S3 pin | Modeled use |
|---|---:|---|
| D0 | GPIO1 | Header GPIO |
| D1 | GPIO2 | Header GPIO |
| D2 | GPIO3 | Header GPIO |
| D3 | GPIO4 | Header GPIO |
| D4 / SDA | GPIO5 | Header GPIO / I²C0 SDA |
| D5 / SCL | GPIO6 | Header GPIO / I²C0 SCL |
| D6 / TX | GPIO43 | UART0 TX |
| D7 / RX | GPIO44 | UART0 RX |
| D8 / SCK | GPIO7 | Header GPIO / SPI clock |
| D9 / MISO | GPIO8 | Header GPIO / SPI MISO |
| D10 / MOSI | GPIO9 | Header GPIO / SPI MOSI |
| USER LED | GPIO21 | Built-in active-low LED, exercised by the smoke |
| USB-C | GPIO19/20 | Native USB Serial/JTAG console used by the smoke |

Only the active-low LED and USB Serial/JTAG console are asserted by this
profile. The other header functions are documented pin mappings, not
peripheral acceptance tests.
The smoke's serial marker uses the native USB Serial/JTAG connector; D6/D7 are
the separate UART0 pins.

## Copy-paste Arduino example

```cpp
#include <Arduino.h>

constexpr int kUserLedPin = 21;

void setup() {
  Serial.begin(115200);
  pinMode(kUserLedPin, OUTPUT);
}

void loop() {
  digitalWrite(kUserLedPin, LOW);  // on: active-low LED
  delay(500);
  digitalWrite(kUserLedPin, HIGH); // off
  delay(500);
}
```

## Run the committed simulator smoke

```sh
examples/xiao-esp32s3/firmware/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/xiao-esp32s3/io-smoke.yaml \
  --output-dir out/xiao-esp32s3 \
  --no-uart-stdout
```

See [`vendor-status.md`](vendor-status.md) for the live availability check,
[`VALIDATION.md`](VALIDATION.md) for the evidence scope, and `evidence/` for the
captured local smoke output.
