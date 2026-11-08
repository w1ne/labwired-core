/* EFR32MG13: flash at 0x0 and main SRAM at 0x20000000, as in the chip
 * descriptor. The top 1 KiB of SRAM is kept out of the linker's RAM so the
 * smoke's summary word can never be overwritten by the stack. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 512K
  RAM   : ORIGIN = 0x20000000, LENGTH = 63K
}
