/* EFM32PG12: flash at 0x0 (Renode efm32xg1x/efm32pg12.repl) and SRAM sized
 * from the chip descriptor. The top 1 KiB of SRAM is kept out of the
 * linker's RAM so the smoke's summary word can never be overwritten by
 * the stack. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 255K
}
