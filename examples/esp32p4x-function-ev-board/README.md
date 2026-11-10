# Espressif ESP32-P4X-Function-EV-Board + BME280

**Demand rank:** ESP32 family **#1** in the 2025 JetBrains State of Embedded
Development survey (38%). This round adds an I2C0 controller and an end-to-end
BME280 ID read to the existing ESP32-P4X Function EV board profile.

The local firmware reads BME280 register `0xD0` at address `0x77` and emits
GPIO8 pulses only when the returned ID is `0x60`. On the board, GPIO8 is **J1
pin 5**. For a physical sensor setup, connect a BME280 breakout to GPIO21 (**J1
pin 11**), GPIO22 (**J1 pin 12**), 3V3 and GND. The simulator attaches the
modeled sensor at the I2C0 controller boundary; it does not model the GPIO
matrix SDA/SCL route or a physical I2C waveform. The local smoke emits 24
high/low pulses after a successful read and then parks.

## Copy-paste ESP-IDF example

Create an ESP-IDF project with target `esp32p4`, connect the BME280 breakout as
described above, then use this as `main.c`:

```c
#include <stdint.h>
#include <stdlib.h>

#include "driver/gpio.h"
#include "driver/i2c_master.h"
#include "esp_err.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

void app_main(void) {
    i2c_master_bus_config_t bus_config = {
        .i2c_port = I2C_NUM_0,
        .sda_io_num = GPIO_NUM_21,
        .scl_io_num = GPIO_NUM_22,
        .clk_source = I2C_CLK_SRC_DEFAULT,
        .glitch_ignore_cnt = 7,
    };
    i2c_master_bus_handle_t bus;
    ESP_ERROR_CHECK(i2c_new_master_bus(&bus_config, &bus));

    i2c_device_config_t sensor_config = {
        .dev_addr_length = I2C_ADDR_BIT_LEN_7,
        .device_address = 0x77,
        .scl_speed_hz = 100000,
    };
    i2c_master_dev_handle_t sensor;
    ESP_ERROR_CHECK(i2c_master_bus_add_device(bus, &sensor_config, &sensor));

    uint8_t id_register = 0xD0;
    uint8_t chip_id = 0;
    ESP_ERROR_CHECK(i2c_master_transmit_receive(
        sensor, &id_register, 1, &chip_id, 1, 1000));
    if (chip_id != 0x60) {
        abort();
    }

    ESP_ERROR_CHECK(gpio_reset_pin(GPIO_NUM_8));
    ESP_ERROR_CHECK(gpio_set_direction(GPIO_NUM_8, GPIO_MODE_OUTPUT));
    for (;;) {
        ESP_ERROR_CHECK(gpio_set_level(GPIO_NUM_8, 1));
        vTaskDelay(pdMS_TO_TICKS(500));
        ESP_ERROR_CHECK(gpio_set_level(GPIO_NUM_8, 0));
        vTaskDelay(pdMS_TO_TICKS(500));
    }
}
```

The BME280 breakout should provide I2C pull-ups; if it does not, add 4.7 kΩ
pull-ups from SDA and SCL to 3.3 V. The [official board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32p4/esp32-p4x-function-ev-board/user_guide.html)
maps GPIO8 to J1 pin 5, GPIO21 to J1 pin 11, and GPIO22 to J1 pin 12. The
[ESP-IDF I2C guide](https://docs.espressif.com/projects/esp-idf/en/stable/esp32p4/api-reference/peripherals/i2c.html)
documents the master bus and write/read API used above.

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

The smoke requires at least 32 GPIO8 transitions and the configured step-limit
stop. Firmware only emits those edges after the modeled BME280 returns ID
`0x60`. The bare-metal image executes from the RISC-V ELF entry point; it does
not exercise mask ROM, the SPI-flash boot path, or ESP-IDF startup. Committed
build and simulator transcripts are under [`evidence/`](evidence/).

## Profile scope

The system models one HP RISC-V execution context, I2C0's command-list
controller with a bus-attached BME280, GPIO8 output and matrix selection, and
GPIO8's IO_MUX function select. It omits the physical I2C pads and matrix
routing, second HP core, LP core, ESP32-C6 companion module, mask ROM, flash
controller, USB, Ethernet, audio, camera, display, storage, and other GPIOs. It
does not model a physical-board capture or the electrical limits of connected
hardware. See [known limitations](KNOWN_LIMITATIONS.md).
