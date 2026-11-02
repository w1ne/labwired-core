/* ESP32-P4 (Guition JC4880P443 module, 32 MiB flash, 16 MiB PSRAM).
 *
 * The P4 maps BOTH instruction fetch and constant data through the unified
 * external-memory window 0x4000_0000..0x43FF_FFFF (esp-idf esp32p4
 * soc/esp32p4/include/soc/soc.h: SOC_IROM_LOW = SOC_DROM_LOW = 0x40000000,
 * SOC_IROM_HIGH = 0x44000000). The chip descriptor declares `flash.base` =
 * 0x4000_0000 and `flash.size` = 32 MiB; this image only needs the low region.
 *
 * HP SRAM is at 0x4FF0_0000 (SOC_IRAM_LOW = SOC_DRAM_LOW) and is reachable from
 * both the instruction and data sides at that single address; 768 KB, matching
 * configs/chips/esp32p4.yaml `ram`. The mask ROM reserves the top of it
 * (0x4FF4_0000..0x4FFC_0000), so the app stack is seeded at 0x4FF4_0000 by
 * boot::esp32p4_rom.
 */
MEMORY
{
  FLASH : ORIGIN = 0x40000000, LENGTH = 4M
  RAM   : ORIGIN = 0x4FF00000, LENGTH = 768K
}

REGION_ALIAS("REGION_TEXT", FLASH);
REGION_ALIAS("REGION_RODATA", FLASH);
REGION_ALIAS("REGION_DATA", RAM);
REGION_ALIAS("REGION_BSS", RAM);
REGION_ALIAS("REGION_HEAP", RAM);
REGION_ALIAS("REGION_STACK", RAM);
