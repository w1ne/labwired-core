/* Bounded to the imported RAMN STM32L552 code flash and SRAM windows. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 512K
  RAM   : ORIGIN = 0x20000000, LENGTH = 256K
}
