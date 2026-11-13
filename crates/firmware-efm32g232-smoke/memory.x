/* EFM32G232: flash and SRAM sized from the chip descriptor (flash at
 * 0x00000000 as on silicon and in Renode). The top 1 KiB of SRAM is kept
 * out of the linker's RAM so the smoke's summary word can never be
 * overwritten by the stack. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 128K
  RAM   : ORIGIN = 0x20000000, LENGTH = 15K
}
