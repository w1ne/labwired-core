/* Conservative minimum SAM4S2 memory windows exercised by this smoke. */
MEMORY
{
  FLASH : ORIGIN = 0x00400000, LENGTH = 128K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}
