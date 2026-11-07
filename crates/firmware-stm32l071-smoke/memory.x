/* Conservative STM32L071 family flash window; SRAM is 20 KiB across the family. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 64K
  RAM   : ORIGIN = 0x20000000, LENGTH = 20K
}
