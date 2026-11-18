# ESP32-C61-DevKitC-1-N8R2 + BME280

**Demand position:** ESP32 is rank 1 in the measured MCU list (38%); the survey
does not report a C61-specific share. This round adds a real I²C sensor path to
the current C61 DevKitC-1 profile and verifies its onboard WS2812 RGB LED on
GPIO8. Live board availability and the GPIO8 LED mapping are in the
[vendor receipt](vendor-status.md).

## Wiring

Connect a BME280 breakout at address `0x77`:

| BME280 | ESP32-C61-DevKitC-1 v2.0 | Purpose |
|---|---|---|
| SDA | GPIO6, J1 pin 5 | I²C0 SDA |
| SCL | GPIO22, J3 pin 6 | I²C0 SCL |
| VIN | 3V3, J1 pin 1 | Sensor supply |
| GND | GND, J1 pin 15 or J3 pin 1 | Common ground |

Espressif's v2.0 guide documents the header mapping. Use a breakout with I²C
pull-ups or add suitable pull-ups to 3V3. The profile does not simulate supply,
pull-up electrical behavior, or the cable.

## Copy-paste ESP-IDF example

This example uses the ESP-IDF I²C master driver to read the BME280 chip ID.
The current ESP-IDF supports ESP32-C61; select the C61 target and configure
UART0 as the console to see the result.

```c
#include "driver/gpio.h"
#include "driver/i2c_master.h"
#include "esp_err.h"
#include "esp_log.h"

#define BME280_ADDRESS 0x77

void app_main(void) {
    i2c_master_bus_handle_t bus;
    const i2c_master_bus_config_t bus_config = {
        .i2c_port = I2C_NUM_0,
        .sda_io_num = GPIO_NUM_6,
        .scl_io_num = GPIO_NUM_22,
        .clk_source = I2C_CLK_SRC_DEFAULT,
        .glitch_ignore_cnt = 7,
    };
    ESP_ERROR_CHECK(i2c_new_master_bus(&bus_config, &bus));

    i2c_master_dev_handle_t sensor;
    const i2c_device_config_t sensor_config = {
        .dev_addr_length = I2C_ADDR_BIT_LEN_7,
        .device_address = BME280_ADDRESS,
        .scl_speed_hz = 100000,
    };
    ESP_ERROR_CHECK(i2c_master_bus_add_device(bus, &sensor_config, &sensor));

    const uint8_t chip_id_register = 0xD0;
    uint8_t chip_id = 0;
    ESP_ERROR_CHECK(i2c_master_transmit_receive(
        sensor, &chip_id_register, 1, &chip_id, 1, 1000));
    if (chip_id != 0x60) {
        ESP_LOGE("bme280", "unexpected chip id: 0x%02x", chip_id);
        return;
    }
    ESP_LOGI("bme280", "BME280_ID=0x%02x", chip_id);
}
```

## Simulator smoke

The committed bare-metal RISC-V firmware configures the C61 PCR I²C clock,
selects I²C0 on GPIO6/GPIO22, reads BME280 register `0xD0` and the raw
temperature test vector at `0xFA`, then emits
`C61_BME280_OK ID=0x60 TEMP_RAW=0x082DC0 I2C0=GPIO6/22`. It then bit-bangs one
red WS2812 frame on GPIO8 and emits `C61_RGB_OK`. The smoke requires the
NeoPixel model to decode one lit pixel with GRB bytes `00 FF 00`. Passing
therefore depends on the modeled I²C command engine, live GPIO matrix route,
attached BME280 register model, GPIO output path, WS2812 observer, and UART.

```sh
examples/esp32c61-devkitc-1-n8r2-bme280/ci/test.sh
```

The smoke fast-loads a bare ELF; it does not run mask ROM or Espressif's
bootloader. IO_MUX writes are stored but electrical pad muxing is not enforced.
The result does not validate strap sampling on GPIO8 during reset, RMT, the
wireless companion, PSRAM, sensor conversion timing, environmental readings,
or physical hardware. The LED smoke bit-bangs GPIO8 and does not claim RMT.
Evidence and the dated live availability receipt are under [`evidence/`](evidence/)
and [`vendor-status.md`](vendor-status.md).
