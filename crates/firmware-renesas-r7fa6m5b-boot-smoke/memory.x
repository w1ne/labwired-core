/* The imported RA6M5 profile uses its corrected code-flash and SRAM windows. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 2048K
  RAM   : ORIGIN = 0x20000000, LENGTH = 512K
}
