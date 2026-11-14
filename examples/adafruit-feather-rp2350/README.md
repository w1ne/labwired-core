# Adafruit Feather RP2350 with HSTX (no-PSRAM, Product ID 6000)

## Demand position

The RP2040/RP2350 family is demand group **#2**. RP2040 was reported by 12%
of respondents (MCU rank 5); RP2350 was not measured separately. This adds
board coverage to the existing RP2350 model, rather than new silicon behavior.
The live availability receipt is in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Board pin map

The chip descriptor maps RP2350 GPIO labels `GP0` through `GP29` to the SIO
pad model. These board labels and routes are the ones exercised or called out
by the smoke:

| Feather label | RP2350 pad | Board route | Twin coverage |
|---|---:|---|---|
| TX | GP0 | UART0 TX | UART0 output capture |
| RX | GP1 | UART0 RX | UART0 peripheral modeled; RX is not exercised |
| SDA / STEMMA QT | GP2 | I2C1 SDA | Not exercised by this profile |
| SCL / STEMMA QT | GP3 | I2C1 SCL | Not exercised by this profile |
| D13 | GP7 | Onboard red LED, active high | Toggled through SIO |
| NeoPixel | GP21 | Onboard addressable RGB LED | Not modeled |
| SCK / MO / MI | GP22 / GP23 / GP20 | SPI0 | Not exercised by this profile |
| HSTX port | GP12–GP19 | Eight HSTX-capable GPIO pads | HSTX is not modeled |
| A0–A3 | GP26–GP29 | RP2350 ADC inputs | Not exercised by this profile |

Adafruit documents `Serial1` on the default UART pins GP0/GP1 and `PIN_LED`
on GP7 in its [pinout guide](https://learn.adafruit.com/adafruit-feather-rp2350/pinouts).
The board's no-PSRAM version is selected because GPIO8 is the optional PSRAM
chip-select pad.

## Supported scope

The descriptor uses the RP2350's 8 MiB QSPI flash geometry and 520 KiB SRAM.
The smoke runs bare-metal Cortex-M33 core 0, writes a marker to UART0, and
toggles the onboard red LED through SIO GPIO7. UART output is captured at the
UART0 model; on physical hardware, use a UART adapter on TX/RX if a UART
terminal is needed. USB CDC is not part of this smoke.

HSTX/DVI, the NeoPixel, USB enumeration/CDC, bootloader behavior, charger and
charge LED, and optional PSRAM are outside this profile. POWERMAN/TICKS and
TBMAN ranges retain the shared RP2350 descriptor's stub behavior. The smoke
does not validate the remaining modeled RP2350 peripherals or physical-board
behavior.

## Copy-paste Arduino example

Select **Adafruit Feather RP2350** in the Arduino IDE with Adafruit's RP2350
board support. Connect a USB-to-UART adapter to TX (GP0), RX (GP1), and GND
to view `Serial1` output.

```cpp
#include <Arduino.h>

constexpr uint8_t LED_PIN = 7;  // D13, onboard red LED

void setup() {
  pinMode(LED_PIN, OUTPUT);
  Serial1.begin(115200);  // UART0 on TX=GP0, RX=GP1
  Serial1.println("FEATHER_RP2350_GPIO7_UART_OK");
}

void loop() {
  digitalWrite(LED_PIN, HIGH);
  delay(500);
  digitalWrite(LED_PIN, LOW);
  delay(500);
}
```

## Reproduce the committed simulator smoke

```sh
scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-rp2350 \
  --crate firmware-adafruit-feather-rp2350-smoke \
  --target thumbv8m.main-none-eabi \
  --firmware-build-dir crates/firmware-adafruit-feather-rp2350-smoke \
  --script examples/adafruit-feather-rp2350/io-smoke.yaml \
  --system configs/systems/adafruit-feather-rp2350.yaml \
  --out-dir examples/adafruit-feather-rp2350/evidence/run
```

The smoke checks `FEATHER_RP2350_GPIO7_UART_OK`, at least two observed SIO
edges on GPIO7, and a clean stop at the configured step limit. Its committed
logs are under [`evidence/run/`](evidence/run/).
