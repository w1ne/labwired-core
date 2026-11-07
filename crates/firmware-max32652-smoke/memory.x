/* Bounded imported MAX32652 memory windows used by the UART smoke. */
MEMORY
{
  FLASH : ORIGIN = 0x10000000, LENGTH = 3M
  RAM   : ORIGIN = 0x20000000, LENGTH = 1M
}
