/* EFM32JG12: flash at 0x0 (Renode efm32xg1x/efm32jg12.repl) and SRAM sized
 * from the chip descriptor. The top 1 KiB of SRAM is kept out of the
 * linker's RAM so the smoke's summary word can never be overwritten by
 * the stack. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 1M
  RAM   : ORIGIN = 0x20000000, LENGTH = 255K
}
