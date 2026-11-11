# Adafruit ESP32-S3 Feather, 8 MB Flash / no PSRAM

Demand rank: **ESP32 #1 by measured MCU usage (38%)**. This adds a real board
system for an already-modelled ESP32-S3; it is a board-coverage addition, not a
new chip family.

The profile boots firmware with 8 MiB flash and no external PSRAM, writes a
UART0 console marker, and drives the built-in one-pixel NeoPixel on GPIO33.
The smoke sends a cycle-timed WS2812 frame through GPIO33 and the attached
device model decodes the green pixel. This profile does not claim the Arduino
library's RMT transmission path; that attempt did not produce a complete frame
in the current simulator.

The accepted smoke frame decodes as GRB `[64, 0, 0]` with one lit pixel.

## Pins represented

| Board function | ESP32-S3 pin | Modeled behavior |
| --- | --- | --- |
| Built-in NeoPixel power enable | GPIO21 | Firmware drives high before use; the system profile does not enforce the gate electrically |
| Built-in NeoPixel data | GPIO33 | Timed GPIO waveform decoded as a one-pixel WS2812 stream |
| Console output | UART0 | Simulation capture at 115200 baud; USB CDC is not represented |

The battery charger and monitor, STEMMA QT power switch, USB, Wi-Fi, and BLE
are not claimed by this profile. The SoC's other peripherals are available
through the S3 model but are not board-specific claims here.

## Build and run

```sh
examples/adafruit-feather-esp32s3-8mb/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/adafruit-feather-esp32s3-8mb/io-smoke.yaml \
  --output-dir out/adafruit-feather-esp32s3-8mb \
  --no-uart-stdout
```

Vendor availability and the board pin references are recorded in
[`vendor-status.md`](vendor-status.md). Run output is committed under
[`evidence/`](evidence/).
