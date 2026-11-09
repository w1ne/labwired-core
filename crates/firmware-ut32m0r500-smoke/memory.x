/* Exact Renode UT32M0R500 platform code flash and SRAM windows. The top
 * 256 bytes of SRAM hold the smoke's summary word at 0x20017F00, outside
 * the stack. */
MEMORY
{
  FLASH : ORIGIN = 0x01000000, LENGTH = 64K
  RAM   : ORIGIN = 0x20000000, LENGTH = 96K - 256
}
