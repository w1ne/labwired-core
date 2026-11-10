/* STM32G491RET6: 512 KiB flash and 96 KiB SRAM1+SRAM2 (ST DS13122).
 * The separate 16 KiB CCM SRAM is deliberately outside this smoke image. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 512K
  RAM   : ORIGIN = 0x20000000, LENGTH = 96K
}
