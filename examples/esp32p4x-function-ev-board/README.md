# Espressif ESP32-P4X-Function-EV-Board

**Demand rank:** ESP32 family **#1** in the 2025 JetBrains State of Embedded
Development survey (38%). This round adds a focused profile for the current
P4X revision of Espressif's multimedia development board.

The committed local firmware smoke configures GPIO8 and checks output edges.
On the physical board, GPIO8 is **J1 pin 5**. Connect an LED in series with a
current-limiting resistor from GPIO8 to GND, or attach a logic analyzer, to
observe the output. The local smoke emits 24 high/low pulses and then parks so
its committed edge trace remains compact.

## Copy-paste ESP-IDF example

Create an ESP-IDF project with target `esp32p4`, then use this as `main.c`:

```c
#include "driver/gpio.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

void app_main(void) {
    gpio_reset_pin(GPIO_NUM_8);
    gpio_set_direction(GPIO_NUM_8, GPIO_MODE_OUTPUT);

    for (;;) {
        gpio_set_level(GPIO_NUM_8, 1);
        vTaskDelay(pdMS_TO_TICKS(500));
        gpio_set_level(GPIO_NUM_8, 0);
        vTaskDelay(pdMS_TO_TICKS(500));
    }
}
```

Espressif documents the P4X board as using ESP32-P4 silicon revision v3.1 or
later. The board guide lists GPIO8 on J1 pin 5. See the [live availability
receipt](evidence/vendor-status.md) before ordering.

## Run the committed smoke

From the repository root:

```bash
rustup target add riscv32imac-unknown-none-elf
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32p4x-function-ev-board \
  --crate firmware-esp32p4x-function-ev-board-smoke \
  --target riscv32imac-unknown-none-elf \
  --script examples/esp32p4x-function-ev-board/onboarding-smoke.yaml \
  --system configs/systems/esp32p4x-function-ev-board.yaml \
  --out-dir examples/esp32p4x-function-ev-board/evidence
```

The smoke asserts at least 32 observed GPIO8 transitions and the expected
step-limit stop. Its bare-metal image executes from the RISC-V ELF entry point;
it does not exercise mask ROM, the SPI-flash boot path, or ESP-IDF startup.
Committed build and simulator transcripts are under [`evidence/`](evidence/).

## Profile scope

This system profile models one HP RISC-V execution context, the GPIO8 output
latch and matrix selection, and GPIO8's IO_MUX function select. It omits the
second HP core, LP core, ESP32-C6 companion module, mask ROM, flash controller,
USB, Ethernet, audio, camera, display, storage, and other GPIOs. It also does
not model the physical header, LED electrical limits, or a physical-board
capture. See [known limitations](KNOWN_LIMITATIONS.md).
