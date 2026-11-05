# Required Source Documents (ESP32-C6-DevKitC-1)

## MCU documentation (authoritative)

1. ESP32-C6 Technical Reference Manual (memory map, PCR, UART, IO_MUX, interrupt matrix):
   https://www.espressif.com/sites/default/files/documentation/esp32-c6_technical_reference_manual_en.pdf
2. ESP32-C6 Datasheet (HP core RV32IMAC up to 160 MHz, 512 KB HP SRAM, 320 KB ROM, LP core):
   https://www.espressif.com/sites/default/files/documentation/esp32-c6_datasheet_en.pdf
3. ESP32-C6 Memory Map (unified IROM/DROM window at 0x4200_0000, HP SRAM at 0x4080_0000,
   LP SRAM at 0x5000_0000):
   https://dl.espressif.com/public/esp32c6-mm.pdf

## Board documentation (authoritative)

1. ESP32-C6-DevKitC-1 v1.2 user guide (8 MB SPI flash module, USB-to-UART bridge on
   U0TXD GPIO16 / U0RXD GPIO17, addressable RGB LED on GPIO8, BOOT on GPIO9):
   https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c6/esp32-c6-devkitc-1/user_guide.html
2. ESP32-C6-DevKitC-1 schematic v1.4 (the GPIO8 addressable LED is a WS2812B):
   https://docs.espressif.com/projects/esp-dev-kits/en/latest/_static/esp32c6/esp32-c6-devkitc-1/schematics/esp32-c6-devkitc-1-schematics.pdf

## Register / interrupt cross-check (authoritative for bases and IRQs)

1. Espressif SVD repository, `svd/esp32c6.svd` — vendored at
   `tests/fixtures/real_world/esp32c6.svd`; consumed by `svd_conformance` and
   `register_coverage`:
   https://github.com/espressif/svd
2. ESP-IDF v5.3 `components/soc/esp32c6/include/soc/soc.h` (SOC_IROM_LOW =
   SOC_DROM_LOW = 0x42000000; SOC_IRAM_LOW = 0x40800000) and
   `soc/reg_base.h` (`DR_REG_UART_BASE` = 0x60000000, `DR_REG_UART1_BASE` =
   0x60001000, `DR_REG_GPIO_BASE` = 0x60091000, `DR_REG_IO_MUX_BASE` =
   0x60090000, `DR_REG_PCR_BASE` = 0x60096000, `DR_REG_INTERRUPT_MATRIX_BASE` =
   0x60010000):
   https://github.com/espressif/esp-idf/tree/v5.3/components/soc/esp32c6/include/soc

Address cross-checks only. The SVD is the committed arbitration source for
bases/IRQs; the TRM/datasheet are the sources for memory sizes and clocks.
