/* EFM32LG942: flash and SRAM sized from the chip descriptor. The top
 * 4 KiB of SRAM is kept out of the linker's RAM so the smoke's summary
 * word can never be overwritten by the stack. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 124K
}
