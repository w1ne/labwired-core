# ESP32-H2-DevKitM-1-N4S

ESP32 family demand rank 1 (38% reported MCU use in the target list); the H2 is not separately ranked. This profile covers the low-power Thread/Zigbee board's UART0 console and a GPIO10 header output. It does not claim radio, USB, flash boot, or addressable RGB LED behavior.

## Copy-paste ESP-IDF example

This blinks GPIO10. Connect an LED and series resistor between GPIO10 and GND on the board headers.

~~~c
#include "driver/gpio.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

void app_main(void) {
    gpio_reset_pin(GPIO_NUM_10);
    gpio_set_direction(GPIO_NUM_10, GPIO_MODE_OUTPUT);
    while (true) {
        gpio_set_level(GPIO_NUM_10, 1);
        vTaskDelay(pdMS_TO_TICKS(500));
        gpio_set_level(GPIO_NUM_10, 0);
        vTaskDelay(pdMS_TO_TICKS(500));
    }
}
~~~

## Build and run the simulator smoke

~~~sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32h2-devkitm-1-n4s-uart-gpio \
  --crate firmware-esp32h2-devkitm-1-smoke \
  --target riscv32imac-unknown-none-elf \
  --script examples/esp32h2-devkitm-1-n4s/uart-gpio-smoke.yaml \
  --system configs/systems/esp32h2-devkitm-1-n4s.yaml \
  --out-dir examples/esp32h2-devkitm-1-n4s/evidence
~~~

The smoke asserts the UART0 console marker, at least eight GPIO10 output edges, and the bounded stop condition. GPIO assertions prove the modeled output latch and enable path, not voltage at the physical pin.

See [vendor status](vendor-status.md) and [validation notes](VALIDATION.md).
