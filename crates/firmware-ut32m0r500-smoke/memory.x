/* Exact Renode UT32M0R500 platform code flash and SRAM windows. */
MEMORY
{
  FLASH : ORIGIN = 0x01000000, LENGTH = 64K
  RAM   : ORIGIN = 0x20000000, LENGTH = 96K
}
