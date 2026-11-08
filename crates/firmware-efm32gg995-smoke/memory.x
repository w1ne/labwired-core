/* EFM32GG995: 1 MiB flash, 128 KiB SRAM @ 0x20000000. The top 4 KiB of SRAM
 * is kept out of the linker's RAM so the smoke's summary word at 0x2001FF00
 * can never be overwritten by the stack. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 124K
}
