# ESP32-C61-DevKitC-1-N8R2 UART0 + GPIO4 smoke

This is a demand-rank 1 ESP32 board profile (38% reported usage in the target
list). The bare-metal smoke boots on the ESP32-C61-DevKitC-1-N8R2, writes a UART0
marker, and toggles GPIO4 through the C61 GPIO output model.

## Pin map

The official v2.0 board guide maps J1 pin 3 to GPIO4. To try the output on a
physical board, connect GPIO4 through a 220 Ω series resistor and LED to GND.
The simulator smoke observes the GPIO model's output edges; it does not prove a
voltage on the package pin because IO_MUX pad routing and electrical behavior
are outside this profile.

## Copy-paste ESP-IDF example

In an ESP-IDF application for `esp32c61`, this blinks GPIO4:

```c
#include "driver/gpio.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

void app_main(void) {
    gpio_reset_pin(GPIO_NUM_4);
    gpio_set_direction(GPIO_NUM_4, GPIO_MODE_OUTPUT);
    while (true) {
        gpio_set_level(GPIO_NUM_4, 1);
        vTaskDelay(pdMS_TO_TICKS(500));
        gpio_set_level(GPIO_NUM_4, 0);
        vTaskDelay(pdMS_TO_TICKS(500));
    }
}
```

## Build and run the simulator smoke

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c61-devkitc-1-n8r2-uart-gpio \
  --crate firmware-esp32c61-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c61-devkitc-1-n8r2/uart-smoke.yaml \
  --system configs/systems/esp32c61-devkitc-1-n8r2.yaml \
  --out-dir examples/esp32c61-devkitc-1-n8r2/evidence
```

The smoke asserts a UART0 marker, at least eight GPIO4 edges, and the bounded
stop. It exercises the C61 GPIO output latch and enable path at the C61 register
offsets. It does not exercise GPIO input, interrupts, other GPIO matrix routes,
IO_MUX pad routing, Wi-Fi, Bluetooth, native USB, PSRAM, or the RGB LED.

See [`vendor-status.md`](vendor-status.md) for the live availability check and
[`VALIDATION.md`](VALIDATION.md) for the committed evidence and boundaries.

The profile also has a separate [BME280 I2C0 example](../esp32c61-devkitc-1-n8r2-bme280/)
that reads sensor registers over GPIO6/GPIO22. Its scope and evidence are
documented independently.
