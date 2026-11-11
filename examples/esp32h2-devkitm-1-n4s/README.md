# ESP32-H2-DevKitM-1-N4S

ESP32 family demand rank 1 (38% reported MCU use in the target list); the H2 is not separately ranked. This is a capability expansion of the already onboarded N4S board, not a new board pick. The profile now covers UART0 console output, GPIO10 direct output, and I2C0 reading a BME280 breakout on GPIO4/GPIO5.

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

For the simulator BME280 smoke, connect a BME280 breakout to 3V3/GND and use
GPIO4 for SDA and GPIO5 for SCL (the board header exposes these pads as J1 pins
9 and 10). The firmware reads the BME280 chip ID and raw temperature through
I2C0 before it emits the UART success marker. See the
[bare-metal smoke source](../../crates/firmware-esp32h2-devkitm-1-smoke/src/main.rs)
for the register-level example used by the simulator.

## Build and run the simulator smoke

~~~sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32h2-devkitm-1-n4s-bme280 \
  --crate firmware-esp32h2-devkitm-1-smoke \
  --target riscv32imac-unknown-none-elf \
  --script examples/esp32h2-devkitm-1-n4s/uart-gpio-smoke.yaml \
  --system configs/systems/esp32h2-devkitm-1-n4s.yaml \
  --out-dir examples/esp32h2-devkitm-1-n4s/evidence
~~~

The smoke asserts the BME280 chip ID and temperature read over I2C0, the UART0
console marker, at least eight GPIO10 output edges, and the bounded stop
condition. These are repository simulator results, not a silicon run.

See [vendor status](vendor-status.md) and [validation notes](VALIDATION.md).
