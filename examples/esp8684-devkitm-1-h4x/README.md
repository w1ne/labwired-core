# ESP8684-DevKitM-1-H4X UART0 and red LED smoke

ESP32-family demand rank: **#1 (38% of MCU use in the survey)**; ESP32-C2 use
is not scored separately. This profile targets the current 4 MiB **H4X**
order code; the older H4 variant is marked NRND in Espressif's current guide.

The smoke fast-loads a bare RV32IMAC ELF, sends a marker through the modeled
UART0 FIFO at 115200 baud, and toggles GPIO0. GPIO0 drives the board's red
discrete RGB LED channel. The smoke asserts UART output and GPIO transitions.

## Pin map

| Board signal | ESP8684 GPIO | Profile behavior |
|---|---:|---|
| Red RGB LED channel | GPIO0 | GPIO output register transitions are observed |
| UART0 TXD | GPIO20 | UART0 bytes are captured at the SoC peripheral |
| UART0 RXD | GPIO19 | Pin identity documented; not exercised |

Espressif's [live board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c2/esp8684-devkitm-1/user_guide.html)
lists H4X as the current 4 MiB order code and maps the RGB LED channels to
GPIO0, GPIO1, and GPIO8. The simulator does not model the USB-to-UART bridge,
pad electrical routing, LED current, or emitted light.

## Build and run

```sh
rustup target add riscv32imac-unknown-none-elf
examples/esp8684-devkitm-1-h4x/ci/test.sh
```

The smoke expects `C2_H4X_GPIO0_UART_OK`, at least four GPIO0 edges, and a
`max_steps` stop after the UART has shifted out the marker. Logs and result
JSON are committed under [`evidence/`](evidence/).

## Copy-paste ESP-IDF example

With ESP-IDF configured for `esp32c2`, this uses the board's UART0 and flashes
the onboard red LED channel:

```c
#include "driver/gpio.h"
#include "driver/uart.h"
#include "esp_rom_sys.h"
#include <stddef.h>

void app_main(void) {
    const uart_config_t uart_cfg = {
        .baud_rate = 115200,
        .data_bits = UART_DATA_8_BITS,
        .parity = UART_PARITY_DISABLE,
        .stop_bits = UART_STOP_BITS_1,
        .flow_ctrl = UART_HW_FLOWCTRL_DISABLE,
        .source_clk = UART_SCLK_DEFAULT,
    };
    uart_driver_install(UART_NUM_0, 256, 0, 0, NULL, 0);
    uart_param_config(UART_NUM_0, &uart_cfg);
    gpio_reset_pin(GPIO_NUM_0);
    gpio_set_direction(GPIO_NUM_0, GPIO_MODE_OUTPUT);
    static const char marker[] = "C2_H4X_GPIO0_UART_OK\n";
    uart_write_bytes(UART_NUM_0, marker, sizeof(marker) - 1);
    for (;;) {
        gpio_set_level(GPIO_NUM_0, 1);
        esp_rom_delay_us(250000);
        gpio_set_level(GPIO_NUM_0, 0);
        esp_rom_delay_us(250000);
    }
}
```

## Scope

- Modeled: bare-ELF fast load, UART0 FIFO/console output, and GPIO output
  register transitions on GPIO0.
- Not claimed: mask-ROM or bootloader startup, USB-UART bridge, IO_MUX
  electrical routing, Wi-Fi, Bluetooth LE, I2C, SPI, ADC, LEDC, RF, buttons,
  power behavior, or physical board validation.

Vendor availability evidence: [`evidence/vendor-status.md`](evidence/vendor-status.md).
