# Required Source Documents (ESP32-P4 / Guition JC4880P443)

## MCU documentation (authoritative)

1. ESP32-P4 Technical Reference Manual (memory map, HP_SYS_CLKRST, UART, IO_MUX,
   interrupt matrix, MIPI-DSI):
   https://www.espressif.com/sites/default/files/documentation/esp32-p4_technical_reference_manual_en.pdf
2. ESP32-P4 Series Datasheet (HP core RV32IMAFC up to 400 MHz, 768 KB HP SRAM,
   128 KB ROM, 32 KB LP SRAM, LP core, MIPI-DSI/CSI):
   https://documentation.espressif.com/esp32-p4_datasheet_en.html
3. ESP32-P4 Memory Map (unified IROM/DROM window at 0x4000_0000, HP SRAM at
   0x4FF0_0000, mask ROM at 0x4FC0_0000, LP SRAM at 0x5010_8000, external
   PSRAM window at 0x4800_0000):
   https://developer.espressif.com/blog/2024/08/esp32-memory-map-101/

## Module documentation (authoritative)

1. Guition JC4880P443 product page (4.3" 480x800 MIPI-DSI touch panel,
   ESP32-P4, 16 MB PSRAM, 32 MB flash):
   https://www.guition.com/ (JC4880P443 module listing)

## Register / cross-check (authoritative for bases and register maps)

1. Espressif SVD repository, `svd/esp32p4.svd` — the arbitration source for
   peripheral base addresses and register offsets:
   https://github.com/espressif/svd
2. ESP-IDF `components/soc/esp32p4/include/soc/soc.h` (SOC_IROM_LOW =
   SOC_DROM_LOW = 0x40000000, SOC_IROM_HIGH = 0x44000000; SOC_IRAM_LOW/HIGH =
   0x4ff00000/0x4ffc0000; SOC_DIRAM_ROM_RESERVE_HIGH = 0x4ff40000) and
   `components/soc/esp32p4/register/soc/reg_base.h` (peripheral bases):
   https://github.com/espressif/esp-idf/tree/master/components/soc/esp32p4

Address cross-checks only. The SVD is the committed source for bases; soc.h is
the source for memory sizes.
