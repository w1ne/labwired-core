# Espressif ESP-Mosaico V1.2 (ESP32-S31)

**Demand rank:** ESP32 family **#1** in the 2025 JetBrains State of Embedded
Development survey (38%). This adds a board from the current S31 generation.
Espressif announced the S31 in mass production; the official ESP-Mosaico store
currently accepts the V1.2 Vision Combo Kit as a preorder.

The committed firmware smoke polls the board's active-low function button on
GPIO7 and controls the vibration motor through GPIO8. It tests the real GPIO
input, output latch, output enable, matrix selector and IO_MUX paths. The model
does not simulate vibration mechanics.

## Copy-paste ESP-IDF example

Use target `esp32s31` and the ESP-Mosaico V1.2 board. This example turns the
vibration motor control on while the function button is pressed:

```c
#include <stdbool.h>
#include "esp_err.h"
#include "driver/gpio.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

void app_main(void) {
    const gpio_config_t button = {
        .pin_bit_mask = 1ULL << GPIO_NUM_7,
        .mode = GPIO_MODE_INPUT,
        .pull_up_en = GPIO_PULLUP_ENABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_DISABLE,
    };
    ESP_ERROR_CHECK(gpio_config(&button));
    ESP_ERROR_CHECK(gpio_reset_pin(GPIO_NUM_8));
    ESP_ERROR_CHECK(gpio_set_direction(GPIO_NUM_8, GPIO_MODE_OUTPUT));

    for (;;) {
        const bool pressed = gpio_get_level(GPIO_NUM_7) == 0;
        ESP_ERROR_CHECK(gpio_set_level(GPIO_NUM_8, pressed));
        vTaskDelay(pdMS_TO_TICKS(10));
    }
}
```

See the [live vendor-status receipt](evidence/vendor-status.md) before
ordering. The board listing is a preorder, not an in-stock claim.

## Run the committed smoke

From the repository root:

```bash
rustup target add riscv32imac-unknown-none-elf
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp-mosaico-v1-2 \
  --crate firmware-esp-mosaico-v1-2-smoke \
  --target riscv32imac-unknown-none-elf \
  --script examples/esp-mosaico-v1-2/onboarding-smoke.yaml \
  --system configs/systems/esp-mosaico-v1-2.yaml \
  --out-dir examples/esp-mosaico-v1-2/evidence
```

The smoke drives the modeled function-button stimulus and checks at least two
GPIO8 transitions, a released/high GPIO7 input, and a low GPIO8 output latch
after release. The ELF is loaded directly into HP SRAM; this is not a ROM/flash
boot or a physical-board test. Build logs and simulator results are committed in
[`evidence/`](evidence/).

## Profile scope

The profile contains one HP RISC-V execution context, GPIO7 and GPIO8, the two
pad mux controls, and a stimulus-driven board button. It omits the second HP
core, LP CPU, flash boot controller, radios, USB, displays, sensors, other
GPIOs, and the vibration motor's mechanical behavior. See
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).
