# ESP32-H2-DevKitM-1-N4S UART and RGB smoke

ESP32 family demand rank: **#1 (38% MCU usage in the 2025 survey)**; H2-specific
demand is not separately measured. This extends board coverage for the
ESP32-H2 Thread/Zigbee-capable member. Espressif's current board order code is
**ESP32-H2-DevKitM-1-N4S**; the earlier N4 order code is marked NRND.

This profile fast-loads a bare RISC-V ELF into the 4 MiB flash window, writes
through the H2 UART0 FIFO, and bit-bangs a red WS2812 frame through GPIO8. The
attached NeoPixel model must decode one lit red pixel. Espressif's board guide
identifies GPIO8 as the addressable RGB LED data pin.

UART0 capture goes directly to the simulator host console. The USB-to-UART
bridge and physical IO_MUX pad route are not simulated, so the smoke does not
claim a physical USB serial connection. The RGB check verifies the modeled
GPIO register output and decoded protocol; it does not model package voltage,
electrical loading, or RMT hardware.

## Build and run

```sh
rustup target add riscv32imc-unknown-none-elf
examples/esp32h2-devkitm-1-n4s/ci/test.sh
```

The UART test expects `H2_N4S_RGB_OK`, observes GPIO8 edges, and requires the
NeoPixel observer to decode a single red pixel. The committed run output is
under [`evidence/`](evidence/).

## Copy-paste Arduino example

Install the **Adafruit NeoPixel** library, select an ESP32-H2 board in the
Arduino ESP32 core, then upload this sketch. The board guide maps its onboard
RGB LED to GPIO8.

```cpp
#include <Adafruit_NeoPixel.h>
#include <Arduino.h>

constexpr uint8_t RGB_PIN = 8;
Adafruit_NeoPixel rgb(1, RGB_PIN, NEO_GRB + NEO_KHZ800);

void setup() {
  Serial.begin(115200);
  rgb.begin();
  rgb.setPixelColor(0, rgb.Color(255, 0, 0));
  rgb.show();
  Serial.println("H2_N4S_RGB_OK");
}

void loop() {}
```

## Scope

- Modeled for this profile: RISC-V fast-boot ELF loading, UART0 FIFO/console,
  UART0 clock gating, the common GPIO data/enable register head, and a
  single-pixel WS2812 observer on GPIO8.
- Not modeled: mask-ROM/bootloader startup, USB-to-UART bridge, IO_MUX pad
  electrical routing, RMT, button/strap behavior, GPIO interrupts, Bluetooth
  LE, IEEE 802.15.4, Thread, Zigbee, USB, and the remaining H2 peripherals.
- This is simulator evidence, not a live-silicon differential.

Vendor availability evidence: [`evidence/vendor-status.md`](evidence/vendor-status.md).
