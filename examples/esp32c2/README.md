# ESP32-C2 (ESP8684H4X) bare-metal smoke

The ESP32-C2 is Espressif's RV32IMC Wi-Fi 4 / Bluetooth LE 5 SoC, sold as
the ESP8684 (ESP8684H4X: 4 MB in-package flash, 120 MHz, 272 KB SRAM, QFN24).
Espressif lists it as the replacement for the NRND ESP8266EX and ESP8285.

This example boots a committed bare ELF on `configs/chips/esp32c2.yaml`:

1. checks a MUL/DIVU result (RV32IMC M extension, volatile operands);
2. writes and reads back the last word of the 256 KiB DRAM view (0x3FCDFFFC);
3. prints `Hello from ESP32-C2` on UART0 only if both checks pass;
4. toggles GPIO10 forever.

## Run it

```sh
labwired test --script examples/esp32c2/uart-smoke.yaml
```

The UART model drains the TX FIFO at the configured baud rate, so the line
needs a few hundred thousand steps; the script allows 600000.

## Rebuild the fixture

```sh
RISCV_CC=/path/to/riscv32-esp-elf-gcc examples/esp32c2/build.sh
```

This writes `tests/fixtures/esp32c2-smoke.elf`.

## Scope

The descriptor reuses the ESP32-C3 UART, GPIO, SYSTIMER and TIMG engines.
Their esp-idf v5.3 register headers (`uart_reg.h`, `gpio_reg.h`,
`systimer_reg.h`, `timer_group_reg.h`) are offset-identical between C2 and C3.
Not modelled: mask ROM contents and ROM boot (so stock ESP-IDF or
Arduino images do not run), IO_MUX, the interrupt matrix, SYSTEM/RTC
clocking, I2C, SPI, LEDC, ADC, GDMA, crypto, Wi-Fi and Bluetooth LE.

PR-gated test: `cargo test -p labwired-core --test vendor esp32c2_compiled_firmware::`.
This is local simulator evidence. It is not hosted `model_verified` and not
hardware confirmation.
