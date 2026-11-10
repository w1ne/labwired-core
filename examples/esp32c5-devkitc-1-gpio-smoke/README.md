# ESP32-C5-DevKitC-1-N8R8 UART0 + GPIO5 smoke

This example boots a small bare-metal RV32IMAC firmware image on the modeled
ESP32-C5 HP core, sends `C5_UART_OK` through the modeled UART0 transmitter,
selects GPIO5, and toggles it through the GPIO output registers. On the current
ESP32-C5-DevKitC-1 v1.2 board, UART0 TX is J3 pin 2 and GPIO5 is J3 pin 9.

This is a focused new-chip smoke, not a full C5 system model. The ELF is loaded
directly into HP SRAM by the simulator; it does not use the C5 mask ROM, flash
boot path, or PSRAM. The UART assertion observes bytes from the C5 UART FIFO
model through the host console sink; UART pin routing and PCR clock gating are
not enforced. The GPIO assertion observes model edges, not package voltage.

## Build and run

```sh
rustup target add riscv32imac-unknown-none-elf
cargo build -p firmware-esp32c5-devkitc-1-gpio-smoke \
  --release --target riscv32imac-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c5-devkitc-1-gpio-smoke/io-smoke.yaml \
  --output-dir out/esp32c5-devkitc-1-n8r8-gpio --no-uart-stdout
```

The assertions require the UART marker and four or more transitions on
`gpio:5`. For a physical bench check, UART0 is available on J3 pin 2 (U0TXD);
connect J3 pin 9 (GPIO5) through a current-limiting resistor to an LED and
ground. The board's onboard addressable RGB LED is on GPIO27 and is not part of
this smoke.

See [`VALIDATION.md`](VALIDATION.md) for the claim boundary and committed
evidence, and [`vendor-status.md`](vendor-status.md) for the current buyability
check.

## Copy-paste Arduino example

Espressif currently lists ESP32-C5 as supported by Arduino-ESP32 when used as
an ESP-IDF component or with rebuilt static libraries. On the physical board,
`Serial` is UART0 and TX appears at J3 pin 2 (U0TXD):

```cpp
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  Serial.println("C5_UART_OK");
  pinMode(5, OUTPUT);
}

void loop() {
  static bool led_on = false;
  digitalWrite(5, led_on ? HIGH : LOW);
  led_on = !led_on;
  delay(250);
}
```

This sketch is for a physical ESP32-C5 board. The committed simulator smoke
uses bare-metal firmware and tests UART FIFO output plus GPIO register edges;
it does not boot Arduino/ESP-IDF or enforce U0TXD pad routing.
